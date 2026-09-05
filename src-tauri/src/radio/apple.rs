use std::{sync::Arc, time::Duration};

use tokio::sync::{mpsc, watch};
use tokio::time::Instant;
use uuid::Uuid;

use crate::{
    contracts::{PlaybackState, PlaybackStateSourceKind, PlaybackStateStatus},
    ipc::{ApiError, EmptyRequest, InternalReason},
    playback::{PlaybackService, SelectMusicSourceRequest},
    storage::Repository,
};

use super::{ConfirmedProgramStart, PlaybackEventHub, ProgramSpeechOutcome, traits::RadioFuture};

const APPLE_SOURCE_ID: &str = "apple_music";
const METADATA_STABILITY: Duration = Duration::from_secs(1);
const MIN_REACTION_TRACK_DURATION_MS: u64 = 30_000;
const COMPLETION_POSITION_TOLERANCE_MS: u64 = 5_000;
const NORMAL_REACTION_INTERVAL: Duration = Duration::from_mins(8);
const QUIET_REACTION_INTERVAL: Duration = Duration::from_mins(15);
const DISCONNECTED_MESSAGE: &str =
    "Apple Music 会话已断开；陪伴模式保持静音并等待 Windows App 会话恢复。";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppleReactionDensity {
    Quiet,
    Balanced,
    Frequent,
}

impl AppleReactionDensity {
    const fn completed_tracks(self) -> u8 {
        match self {
            Self::Quiet => 5,
            Self::Balanced => 3,
            Self::Frequent => 2,
        }
    }

    const fn minimum_interval(self) -> Duration {
        match self {
            Self::Quiet => QUIET_REACTION_INTERVAL,
            Self::Balanced | Self::Frequent => NORMAL_REACTION_INTERVAL,
        }
    }
}

pub trait AppleCompanionPolicySource: Send + Sync {
    fn reaction_density(
        &self,
        program_id: Uuid,
    ) -> RadioFuture<'_, Result<AppleReactionDensity, ApiError>>;
}

impl AppleCompanionPolicySource for Repository {
    fn reaction_density(
        &self,
        program_id: Uuid,
    ) -> RadioFuture<'_, Result<AppleReactionDensity, ApiError>> {
        Box::pin(async move {
            let settings = self
                .load_provider_settings()
                .await
                .map_err(|_| ApiError::from_reason(InternalReason::StorageReadFailed))?;
            let voice_allowed = Repository::voice_allowed_after_feedback(self, program_id)
                .await
                .map_err(|_| ApiError::from_reason(InternalReason::StorageReadFailed))?;
            if !voice_allowed {
                return Ok(AppleReactionDensity::Quiet);
            }
            match settings.narration_density.as_str() {
                "quiet" => Ok(AppleReactionDensity::Quiet),
                "balanced" => Ok(AppleReactionDensity::Balanced),
                "frequent" => Ok(AppleReactionDensity::Frequent),
                _ => Err(ApiError::from_reason(
                    InternalReason::StorageIntegrityFailed,
                )),
            }
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppleCompanionSignal {
    Reaction(String),
    Disconnected,
    Reconnected,
}

pub trait SystemProgramSpeech: Send + Sync {
    fn present_opening(
        &self,
        program_id: Uuid,
        authorization: ConfirmedProgramStart,
        cancellation: watch::Receiver<bool>,
    ) -> RadioFuture<'_, Result<ProgramSpeechOutcome, ApiError>>;

    fn cancel(&self, program_id: Uuid);
}

pub trait AppleCompanion: Send + Sync {
    fn connect(&self) -> RadioFuture<'_, Result<PlaybackState, ApiError>>;

    fn run(
        &self,
        program_id: Uuid,
        initial: PlaybackState,
        authorization: ConfirmedProgramStart,
        cancellation: watch::Receiver<bool>,
        signals: mpsc::Sender<AppleCompanionSignal>,
    ) -> RadioFuture<'_, Result<(), ApiError>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct UnavailableAppleCompanion;

impl AppleCompanion for UnavailableAppleCompanion {
    fn connect(&self) -> RadioFuture<'_, Result<PlaybackState, ApiError>> {
        Box::pin(async { Err(ApiError::from_reason(InternalReason::SourceUnavailable)) })
    }

    fn run(
        &self,
        _program_id: Uuid,
        _initial: PlaybackState,
        _authorization: ConfirmedProgramStart,
        _cancellation: watch::Receiver<bool>,
        _signals: mpsc::Sender<AppleCompanionSignal>,
    ) -> RadioFuture<'_, Result<(), ApiError>> {
        Box::pin(async { Err(ApiError::from_reason(InternalReason::SourceUnavailable)) })
    }
}

pub struct AppleCompanionMonitor {
    playback: PlaybackService,
    events: PlaybackEventHub,
    speech: Arc<dyn SystemProgramSpeech>,
    policy: Arc<dyn AppleCompanionPolicySource>,
}

impl AppleCompanionMonitor {
    #[must_use]
    pub fn new(
        playback: PlaybackService,
        events: PlaybackEventHub,
        speech: Arc<dyn SystemProgramSpeech>,
        policy: Arc<dyn AppleCompanionPolicySource>,
    ) -> Self {
        Self {
            playback,
            events,
            speech,
            policy,
        }
    }
}

impl AppleCompanion for AppleCompanionMonitor {
    fn connect(&self) -> RadioFuture<'_, Result<PlaybackState, ApiError>> {
        Box::pin(async move {
            self.playback
                .select_music_source(SelectMusicSourceRequest {
                    client_request_id: Uuid::now_v7(),
                    source_id: APPLE_SOURCE_ID.to_owned(),
                })
                .await
                .map(|response| response.state)
        })
    }

    fn run(
        &self,
        program_id: Uuid,
        initial: PlaybackState,
        authorization: ConfirmedProgramStart,
        mut cancellation: watch::Receiver<bool>,
        signals: mpsc::Sender<AppleCompanionSignal>,
    ) -> RadioFuture<'_, Result<(), ApiError>> {
        Box::pin(async move {
            let mut events = self.events.subscribe();
            let opening =
                self.speech
                    .present_opening(program_id, authorization, cancellation.clone());
            tokio::pin!(opening);
            tokio::select! {
                result = &mut opening => { let _ = result?; }
                changed = cancellation.changed() => {
                    self.speech.cancel(program_id);
                    if changed.is_err() || *cancellation.borrow() { return Ok(()); }
                }
            }

            let reconciled = self
                .playback
                .get_playback_state(EmptyRequest {})
                .await
                .unwrap_or(initial);
            let started_at = Instant::now();
            let context = AppleObservationContext {
                playback: &self.playback,
                policy: self.policy.as_ref(),
                program_id,
                signals: &signals,
                started_at,
            };
            let mut gate = AppleReactionGate::default();
            observe(&context, reconciled, &mut cancellation, &mut gate).await?;
            loop {
                tokio::select! {
                    changed = cancellation.changed() => {
                        if changed.is_err() || *cancellation.borrow() {
                            self.speech.cancel(program_id);
                            return Ok(());
                        }
                    }
                    event = events.recv() => {
                        let event = event.map_err(|_| ApiError::unexpected())?;
                        if event.source_id != APPLE_SOURCE_ID { continue; }
                        if let Some(state) = event.state {
                            observe(
                                &context,
                                state,
                                &mut cancellation,
                                &mut gate,
                            )
                            .await?;
                        }
                    }
                }
            }
        })
    }
}

struct AppleObservationContext<'a> {
    playback: &'a PlaybackService,
    policy: &'a dyn AppleCompanionPolicySource,
    program_id: Uuid,
    signals: &'a mpsc::Sender<AppleCompanionSignal>,
    started_at: Instant,
}

async fn observe(
    context: &AppleObservationContext<'_>,
    state: PlaybackState,
    cancellation: &mut watch::Receiver<bool>,
    gate: &mut AppleReactionGate,
) -> Result<(), ApiError> {
    if !is_apple_state(&state) {
        return Ok(());
    }
    if !gate.observe_timeline(&state) {
        return Ok(());
    }
    if state.status == PlaybackStateStatus::Disconnected {
        if gate.disconnected_reported {
            return Ok(());
        }
        gate.disconnected_reported = true;
        return context
            .signals
            .send(AppleCompanionSignal::Disconnected)
            .await
            .map_err(|_| ApiError::unexpected());
    }
    if gate.disconnected_reported && state.status != PlaybackStateStatus::Playing {
        return Ok(());
    }
    let Some(identity) = ReactionIdentity::from_state(&state) else {
        return Ok(());
    };
    if !gate.disconnected_reported
        && (gate.evaluated_identity.as_ref() == Some(&identity)
            || gate.last_reacted_track_id.as_deref() == Some(identity.track_id.as_str()))
    {
        return Ok(());
    }

    let stable = stabilize_metadata(context.playback, state, cancellation).await?;
    if !is_apple_state(&stable) {
        return Ok(());
    }
    if !gate.observe_timeline(&stable) {
        return Ok(());
    }
    if stable.status == PlaybackStateStatus::Disconnected {
        if gate.disconnected_reported {
            return Ok(());
        }
        gate.disconnected_reported = true;
        return context
            .signals
            .send(AppleCompanionSignal::Disconnected)
            .await
            .map_err(|_| ApiError::unexpected());
    }
    if gate.disconnected_reported && stable.status != PlaybackStateStatus::Playing {
        return Ok(());
    }
    if gate.disconnected_reported {
        gate.disconnected_reported = false;
        context
            .signals
            .send(AppleCompanionSignal::Reconnected)
            .await
            .map_err(|_| ApiError::unexpected())?;
    }
    let Some(stable_identity) = ReactionIdentity::from_state(&stable) else {
        return Ok(());
    };
    gate.evaluated_identity = Some(stable_identity.clone());
    if stable_identity.duration_ms <= MIN_REACTION_TRACK_DURATION_MS
        || gate.last_reacted_track_id.as_deref() == Some(stable_identity.track_id.as_str())
    {
        return Ok(());
    }

    let density = context
        .policy
        .reaction_density(context.program_id)
        .await
        .unwrap_or(AppleReactionDensity::Quiet);
    let elapsed = context.started_at.elapsed();
    if !gate.should_react(&stable_identity, density, elapsed) {
        return Ok(());
    }
    let reaction =
        deterministic_reaction(&stable_identity.title, stable_identity.artist.as_deref());
    context
        .signals
        .send(AppleCompanionSignal::Reaction(reaction))
        .await
        .map_err(|_| ApiError::unexpected())
}

fn is_apple_state(state: &PlaybackState) -> bool {
    state.source_id == APPLE_SOURCE_ID
        && state.source_kind == PlaybackStateSourceKind::SystemSession
}

async fn stabilize_metadata(
    playback: &PlaybackService,
    mut candidate: PlaybackState,
    cancellation: &mut watch::Receiver<bool>,
) -> Result<PlaybackState, ApiError> {
    loop {
        tokio::select! {
            biased;
            changed = cancellation.changed() => {
                if changed.is_err() || *cancellation.borrow() {
                    return Err(ApiError::from_reason(InternalReason::OperationCancelled));
                }
            }
            () = tokio::time::sleep(METADATA_STABILITY) => {}
        }
        let refreshed = playback.get_playback_state(EmptyRequest {}).await?;
        if refreshed.status == PlaybackStateStatus::Disconnected
            || ReactionIdentity::from_state(&candidate) == ReactionIdentity::from_state(&refreshed)
        {
            return Ok(refreshed);
        }
        candidate = refreshed;
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReactionIdentity {
    track_id: String,
    title: String,
    artist: Option<String>,
    album: Option<String>,
    duration_ms: u64,
}

impl ReactionIdentity {
    fn from_state(state: &PlaybackState) -> Option<Self> {
        if !is_apple_state(state) {
            return None;
        }
        let track = state.current_track.as_ref()?;
        Some(Self {
            track_id: track.track_id.clone(),
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            duration_ms: state.duration_ms?,
        })
    }
}

#[derive(Debug)]
struct ObservedTrack {
    track_id: String,
    duration_ms: u64,
    minimum_position_ms: u64,
    maximum_position_ms: u64,
}

impl ObservedTrack {
    fn from_state(state: &PlaybackState) -> Option<Self> {
        if !is_apple_state(state) {
            return None;
        }
        let track = state.current_track.as_ref()?;
        let duration_ms = state.duration_ms?;
        Some(Self {
            track_id: track.track_id.clone(),
            duration_ms,
            minimum_position_ms: state.position_ms,
            maximum_position_ms: state.position_ms,
        })
    }

    fn update(&mut self, state: &PlaybackState) {
        if let Some(duration_ms) = state.duration_ms {
            self.duration_ms = duration_ms;
        }
        self.minimum_position_ms = self.minimum_position_ms.min(state.position_ms);
        self.maximum_position_ms = self.maximum_position_ms.max(state.position_ms);
    }

    const fn completed(&self) -> bool {
        self.duration_ms > MIN_REACTION_TRACK_DURATION_MS
            && self.minimum_position_ms <= COMPLETION_POSITION_TOLERANCE_MS
            && self
                .maximum_position_ms
                .saturating_add(COMPLETION_POSITION_TOLERANCE_MS)
                >= self.duration_ms
    }
}

#[derive(Debug, Default)]
struct AppleReactionGate {
    current: Option<ObservedTrack>,
    last_revision: u64,
    evaluated_identity: Option<ReactionIdentity>,
    last_reacted_track_id: Option<String>,
    completed_since_reaction: u8,
    last_reaction_at: Option<Duration>,
    disconnected_reported: bool,
}

impl AppleReactionGate {
    fn observe_timeline(&mut self, state: &PlaybackState) -> bool {
        if state.revision < self.last_revision {
            return false;
        }
        self.last_revision = self.last_revision.max(state.revision);
        if state.status == PlaybackStateStatus::Disconnected {
            self.current = None;
            self.evaluated_identity = None;
            return true;
        }
        let Some(next) = ObservedTrack::from_state(state) else {
            return true;
        };
        match self.current.as_mut() {
            Some(current) if current.track_id == next.track_id => current.update(state),
            Some(_) => {
                let previous = self.current.replace(next);
                if previous.as_ref().is_some_and(ObservedTrack::completed) {
                    self.completed_since_reaction = self.completed_since_reaction.saturating_add(1);
                }
                self.evaluated_identity = None;
            }
            None => self.current = Some(next),
        }
        true
    }

    fn should_react(
        &mut self,
        identity: &ReactionIdentity,
        density: AppleReactionDensity,
        elapsed: Duration,
    ) -> bool {
        let eligible = match self.last_reaction_at {
            None => true,
            Some(last) => {
                self.completed_since_reaction >= density.completed_tracks()
                    && elapsed.saturating_sub(last) >= density.minimum_interval()
            }
        };
        if !eligible {
            return false;
        }
        self.last_reacted_track_id = Some(identity.track_id.clone());
        self.completed_since_reaction = 0;
        self.last_reaction_at = Some(elapsed);
        true
    }
}

fn deterministic_reaction(title: &str, artist: Option<&str>) -> String {
    match artist {
        Some(artist) => {
            format!("正在播放《{title}》— {artist}。曲目信息来自 Apple Music；这条反应在本机生成。")
        }
        None => format!("正在播放《{title}》。曲目信息来自 Apple Music；这条反应在本机生成。"),
    }
}

#[must_use]
pub(crate) const fn disconnected_message() -> &'static str {
    DISCONNECTED_MESSAGE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(track: &str, position_ms: u64, duration_ms: u64, revision: u64) -> PlaybackState {
        PlaybackState {
            schema_version: crate::ipc::IPC_SCHEMA_VERSION.to_owned(),
            source_id: APPLE_SOURCE_ID.to_owned(),
            source_kind: crate::contracts::PlaybackStateSourceKind::SystemSession,
            status: PlaybackStateStatus::Playing,
            capabilities: crate::contracts::PlaybackStateCapabilities {
                play: true,
                pause: true,
                seek: true,
                next: true,
                previous: true,
                set_queue: false,
            },
            current_track: Some(crate::contracts::PlaybackStateTrack {
                track_id: track.to_owned(),
                title: format!("Title {track}"),
                artist: Some("Artist".to_owned()),
                album: Some("Album".to_owned()),
                artwork_uri: None,
                origin: crate::contracts::PlaybackStateTrackOrigin::SystemSession,
            }),
            position_ms,
            duration_ms: Some(duration_ms),
            revision,
            updated_at: "2026-09-05T00:00:00Z".to_owned(),
            last_error: None,
        }
    }

    fn complete_and_change(
        gate: &mut AppleReactionGate,
        previous: &str,
        next: &str,
        revision: u64,
    ) {
        assert!(gate.observe_timeline(&state(previous, 175_500, 180_000, revision)));
        assert!(gate.observe_timeline(&state(next, 0, 180_000, revision + 1)));
    }

    #[test]
    fn apple_program_reaction_is_deterministic_and_does_not_infer_missing_artist() {
        assert_eq!(
            deterministic_reaction("Title", None),
            "正在播放《Title》。曲目信息来自 Apple Music；这条反应在本机生成。"
        );
        assert_eq!(
            deterministic_reaction("Title", Some("Artist")),
            deterministic_reaction("Title", Some("Artist"))
        );
    }

    #[test]
    fn apple_program_reaction_requires_stable_long_metadata() {
        assert_ne!(
            ReactionIdentity::from_state(&state("one", 0, 180_000, 1)),
            ReactionIdentity::from_state(&state("two", 0, 180_000, 2))
        );
        assert_eq!(
            ReactionIdentity::from_state(&state("one", 0, 180_000, 1)),
            ReactionIdentity::from_state(&state("one", 20, 180_000, 1))
        );

        let mut gate = AppleReactionGate::default();
        let short =
            ReactionIdentity::from_state(&state("short", 0, 30_000, 1)).expect("short identity");
        gate.observe_timeline(&state("short", 0, 30_000, 1));
        assert!(short.duration_ms <= MIN_REACTION_TRACK_DURATION_MS);
        assert!(!gate.current.as_ref().expect("short current").completed());

        let mut local = state("local", 0, 180_000, 2);
        local.source_id = "local".to_owned();
        local.source_kind = PlaybackStateSourceKind::Local;
        assert!(!is_apple_state(&local));
        assert!(ReactionIdentity::from_state(&local).is_none());
        assert!(ObservedTrack::from_state(&local).is_none());
    }

    #[test]
    fn apple_program_balanced_reaction_waits_for_three_complete_tracks_and_eight_minutes() {
        let mut gate = AppleReactionGate::default();
        let opening_state = state("opening", 0, 180_000, 1);
        assert!(gate.observe_timeline(&opening_state));
        let opening = ReactionIdentity::from_state(&opening_state).expect("opening identity");
        assert!(gate.should_react(&opening, AppleReactionDensity::Balanced, Duration::ZERO));

        complete_and_change(&mut gate, "opening", "second", 1);
        complete_and_change(&mut gate, "second", "third", 2);
        let third =
            ReactionIdentity::from_state(&state("third", 0, 180_000, 3)).expect("third identity");
        assert!(!gate.should_react(
            &third,
            AppleReactionDensity::Balanced,
            Duration::from_mins(9)
        ));

        complete_and_change(&mut gate, "third", "fourth", 3);
        let fourth =
            ReactionIdentity::from_state(&state("fourth", 0, 180_000, 4)).expect("fourth identity");
        assert!(!gate.should_react(
            &fourth,
            AppleReactionDensity::Balanced,
            Duration::from_secs(7 * 60 + 59)
        ));
        assert!(gate.should_react(
            &fourth,
            AppleReactionDensity::Balanced,
            Duration::from_mins(8)
        ));
    }

    #[test]
    fn apple_program_quiet_reaction_waits_for_five_complete_tracks_and_fifteen_minutes() {
        let mut gate = AppleReactionGate::default();
        let opening_state = state("one", 0, 180_000, 1);
        gate.observe_timeline(&opening_state);
        let opening = ReactionIdentity::from_state(&opening_state).expect("opening identity");
        assert!(gate.should_react(&opening, AppleReactionDensity::Quiet, Duration::ZERO));
        for (previous, next, revision) in [
            ("one", "two", 1),
            ("two", "three", 2),
            ("three", "four", 3),
            ("four", "five", 4),
            ("five", "six", 5),
        ] {
            complete_and_change(&mut gate, previous, next, revision);
        }
        let sixth =
            ReactionIdentity::from_state(&state("six", 0, 180_000, 6)).expect("sixth identity");
        assert!(!gate.should_react(
            &sixth,
            AppleReactionDensity::Quiet,
            Duration::from_secs(14 * 60 + 59)
        ));
        assert!(gate.should_react(&sixth, AppleReactionDensity::Quiet, Duration::from_mins(15)));
    }
}
