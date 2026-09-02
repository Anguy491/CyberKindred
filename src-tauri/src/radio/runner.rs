use std::{collections::HashMap, sync::Arc};

use tokio::sync::{Mutex, Notify, mpsc, watch};
use uuid::Uuid;

use crate::{
    contracts::{ProgramPlanSegmentsItem, ProgramPlanTrackSegment, ProgramPlanVoiceSegment},
    ipc::{ApiError, InternalReason},
    program::PlannedProgram,
};

use super::{
    ConfirmedProgramStart, ProgramEventState, ProgramPlayback, ProgramPlaybackSignal,
    ProgramRunPhase, ProgramSegmentEventState, ProgramSegmentPhase, ProgramSpeech,
    ProgramSpeechOutcome, RadioClock, RadioProgramStore, events::RadioEventPublisher,
    playback::ProgramTrack,
};

const PROGRAM_FAILED_MESSAGE: &str = "节目无法继续，已安全停止。";
const SPEECH_DEGRADED_MESSAGE: &str = "语音不可用，已保留文字并继续播放。";
const PLAN_DEGRADED_MESSAGE: &str = "AI 计划不可用，已使用确定性本地队列。";

pub(crate) struct ActiveRun {
    pub(crate) program_id: Uuid,
    cancellation: watch::Sender<bool>,
    current_speech: std::sync::Mutex<Option<Uuid>>,
    completion: Mutex<Option<Result<u64, ApiError>>>,
    completion_notify: Notify,
}

impl ActiveRun {
    pub(crate) fn new(program_id: Uuid) -> Arc<Self> {
        let (cancellation, _) = watch::channel(false);
        Arc::new(Self {
            program_id,
            cancellation,
            current_speech: std::sync::Mutex::new(None),
            completion: Mutex::new(None),
            completion_notify: Notify::new(),
        })
    }

    pub(crate) fn cancel(&self) {
        self.cancellation.send_replace(true);
    }

    pub(crate) fn cancellation(&self) -> watch::Receiver<bool> {
        self.cancellation.subscribe()
    }

    pub(crate) fn current_speech(&self) -> Option<Uuid> {
        self.current_speech.lock().ok().and_then(|guard| *guard)
    }

    fn set_current_speech(&self, segment_id: Option<Uuid>) {
        if let Ok(mut current) = self.current_speech.lock() {
            *current = segment_id;
        }
    }

    pub(crate) async fn finish(&self, result: Result<u64, ApiError>) {
        *self.completion.lock().await = Some(result);
        self.completion_notify.notify_waiters();
    }

    pub(crate) async fn wait(&self) -> Result<u64, ApiError> {
        loop {
            let notified = self.completion_notify.notified();
            if let Some(result) = self.completion.lock().await.clone() {
                return result;
            }
            notified.await;
        }
    }
}

#[derive(Clone)]
pub struct ProgramRunner {
    store: Arc<dyn RadioProgramStore>,
    playback: Arc<dyn ProgramPlayback>,
    speech: Arc<dyn ProgramSpeech>,
    events: RadioEventPublisher,
    clock: Arc<dyn RadioClock>,
}

impl ProgramRunner {
    #[must_use]
    pub(crate) fn new(
        store: Arc<dyn RadioProgramStore>,
        playback: Arc<dyn ProgramPlayback>,
        speech: Arc<dyn ProgramSpeech>,
        events: RadioEventPublisher,
        clock: Arc<dyn RadioClock>,
    ) -> Self {
        Self {
            store,
            playback,
            speech,
            events,
            clock,
        }
    }

    pub(crate) async fn run(
        &self,
        active: Arc<ActiveRun>,
        planned: PlannedProgram,
        authorization: ConfirmedProgramStart,
        initial_revision: u64,
    ) -> Result<u64, ApiError> {
        let mut execution = RunExecution::new(self, active, planned, initial_revision)?;
        execution.publish_queued();
        match execution.execute(authorization).await {
            Ok(revision) => Ok(revision),
            Err(_) if execution.cancelled() => execution.stop().await,
            Err(_) => execution.fail().await,
        }
    }
}

struct RunExecution<'a> {
    runner: &'a ProgramRunner,
    active: Arc<ActiveRun>,
    planned: PlannedProgram,
    program_id: Uuid,
    phase: ProgramRunPhase,
    revision: u64,
    segment_states: HashMap<Uuid, ProgramSegmentPhase>,
    running_emitted: bool,
    speech_degraded_emitted: bool,
}

impl<'a> RunExecution<'a> {
    fn new(
        runner: &'a ProgramRunner,
        active: Arc<ActiveRun>,
        planned: PlannedProgram,
        revision: u64,
    ) -> Result<Self, ApiError> {
        let program_id = parse_v7(&planned.plan.program_id)?;
        let segment_states = planned
            .plan
            .segments
            .iter()
            .map(|segment| segment_id(segment).map(|id| (id, ProgramSegmentPhase::Planned)))
            .collect::<Result<HashMap<_, _>, _>>()?;
        Ok(Self {
            runner,
            active,
            planned,
            program_id,
            phase: ProgramRunPhase::Ready,
            revision,
            segment_states,
            running_emitted: false,
            speech_degraded_emitted: false,
        })
    }

    fn publish_queued(&self) {
        for segment in &self.planned.plan.segments {
            if let Ok(segment_id) = segment_id(segment) {
                self.runner.events.segment(
                    self.program_id,
                    segment_id,
                    ProgramSegmentEventState::Queued,
                );
            }
        }
    }

    async fn execute(&mut self, authorization: ConfirmedProgramStart) -> Result<u64, ApiError> {
        let mut index = 0_usize;
        while index < self.planned.plan.segments.len() {
            self.ensure_not_cancelled()?;
            match self.planned.plan.segments[index].clone() {
                ProgramPlanSegmentsItem::VoiceSegment(voice) => {
                    self.execute_voice(&voice, authorization).await?;
                    index += 1;
                }
                ProgramPlanSegmentsItem::TrackSegment(_) => {
                    let (tracks, next_index) = self.track_batch(index)?;
                    self.execute_track_batch(&tracks, authorization).await?;
                    index = next_index;
                }
            }
        }
        self.transition_run(ProgramRunPhase::Completing, None)
            .await?;
        self.transition_run(ProgramRunPhase::Completed, None)
            .await?;
        self.runner
            .events
            .state(self.program_id, ProgramEventState::Completed, None);
        Ok(self.revision)
    }

    async fn execute_voice(
        &mut self,
        voice: &ProgramPlanVoiceSegment,
        authorization: ConfirmedProgramStart,
    ) -> Result<(), ApiError> {
        let segment_id = parse_v7(&voice.segment_id)?;
        self.transition_run(ProgramRunPhase::VoicePreparing, None)
            .await?;
        self.emit_running_once();
        self.transition_segment(segment_id, ProgramSegmentPhase::Preparing, None, None)
            .await?;
        self.transition_run(ProgramRunPhase::Voice, None).await?;
        self.transition_segment(
            segment_id,
            ProgramSegmentPhase::Active,
            Some(ProgramSegmentEventState::Playing),
            None,
        )
        .await?;
        self.active.set_current_speech(Some(segment_id));
        let mut cancellation = self.active.cancellation();
        let presentation = self.runner.speech.present(
            self.program_id,
            segment_id,
            &voice.text,
            authorization,
            self.active.cancellation(),
        );
        tokio::pin!(presentation);
        let result = tokio::select! {
            result = &mut presentation => result,
            changed = cancellation.changed() => {
                self.runner.speech.cancel(segment_id);
                if changed.is_err() || self.cancelled() {
                    Err(ApiError::from_reason(InternalReason::OperationCancelled))
                } else {
                    Err(ApiError::unexpected())
                }
            }
        };
        self.active.set_current_speech(None);
        match result {
            Ok(ProgramSpeechOutcome::Spoken) => {
                self.transition_segment(
                    segment_id,
                    ProgramSegmentPhase::Completed,
                    Some(ProgramSegmentEventState::Completed),
                    None,
                )
                .await
            }
            Ok(ProgramSpeechOutcome::TextOnly) => {
                self.transition_segment(
                    segment_id,
                    ProgramSegmentPhase::Completed,
                    Some(ProgramSegmentEventState::Completed),
                    None,
                )
                .await?;
                self.emit_speech_degraded_once();
                Ok(())
            }
            Err(error) if self.cancelled() => Err(error),
            Err(_) => {
                self.transition_segment(
                    segment_id,
                    ProgramSegmentPhase::Failed,
                    Some(ProgramSegmentEventState::Failed),
                    Some("speech_failed"),
                )
                .await?;
                self.emit_speech_degraded_once();
                Ok(())
            }
        }
    }

    fn track_batch(&self, start: usize) -> Result<(Vec<ProgramTrack>, usize), ApiError> {
        let mut tracks = Vec::with_capacity(3);
        let mut index = start;
        while index < self.planned.plan.segments.len() && tracks.len() < 3 {
            let ProgramPlanSegmentsItem::TrackSegment(track) = &self.planned.plan.segments[index]
            else {
                break;
            };
            tracks.push(program_track(track)?);
            index += 1;
        }
        if tracks.is_empty()
            || self
                .planned
                .plan
                .segments
                .get(index)
                .is_some_and(|segment| matches!(segment, ProgramPlanSegmentsItem::TrackSegment(_)))
        {
            return Err(ApiError::from_reason(
                InternalReason::ProviderInvalidResponse,
            ));
        }
        Ok((tracks, index))
    }

    async fn execute_track_batch(
        &mut self,
        tracks: &[ProgramTrack],
        authorization: ConfirmedProgramStart,
    ) -> Result<(), ApiError> {
        self.transition_run(ProgramRunPhase::Music, None).await?;
        self.emit_running_once();
        let (signals, mut receiver) = mpsc::channel(tracks.len().saturating_mul(2).max(2));
        let playback = self.runner.playback.play_batch(
            self.program_id,
            tracks,
            authorization,
            self.active.cancellation(),
            signals,
        );
        tokio::pin!(playback);
        let mut cancellation = self.active.cancellation();
        loop {
            tokio::select! {
                result = &mut playback => {
                    result?;
                    while let Ok(signal) = receiver.try_recv() {
                        self.apply_playback_signal(signal).await?;
                    }
                    self.ensure_all_tracks_terminal(tracks)?;
                    return Ok(());
                }
                signal = receiver.recv() => {
                    let Some(signal) = signal else {
                        return Err(ApiError::unexpected());
                    };
                    self.apply_playback_signal(signal).await?;
                }
                changed = cancellation.changed() => {
                    if changed.is_err() || self.cancelled() {
                        return Err(ApiError::from_reason(InternalReason::OperationCancelled));
                    }
                }
            }
        }
    }

    async fn apply_playback_signal(
        &mut self,
        signal: ProgramPlaybackSignal,
    ) -> Result<(), ApiError> {
        match signal {
            ProgramPlaybackSignal::Started(segment_id) => {
                self.transition_segment(
                    segment_id,
                    ProgramSegmentPhase::Active,
                    Some(ProgramSegmentEventState::Playing),
                    None,
                )
                .await
            }
            ProgramPlaybackSignal::Completed(segment_id) => {
                self.transition_segment(
                    segment_id,
                    ProgramSegmentPhase::Completed,
                    Some(ProgramSegmentEventState::Completed),
                    None,
                )
                .await
            }
            ProgramPlaybackSignal::Failed(segment_id) => {
                self.transition_segment(
                    segment_id,
                    ProgramSegmentPhase::Failed,
                    Some(ProgramSegmentEventState::Failed),
                    Some("media_failed"),
                )
                .await
            }
            ProgramPlaybackSignal::Paused => {
                self.transition_run(ProgramRunPhase::Paused, None).await?;
                self.runner
                    .events
                    .state(self.program_id, ProgramEventState::Paused, None);
                Ok(())
            }
            ProgramPlaybackSignal::Resumed => {
                self.transition_run(ProgramRunPhase::Music, None).await?;
                self.runner
                    .events
                    .state(self.program_id, ProgramEventState::Running, None);
                Ok(())
            }
        }
    }

    fn ensure_all_tracks_terminal(&self, tracks: &[ProgramTrack]) -> Result<(), ApiError> {
        if tracks.iter().all(|track| {
            self.segment_states
                .get(&track.segment_id)
                .copied()
                .is_some_and(segment_terminal)
        }) {
            Ok(())
        } else {
            Err(ApiError::unexpected())
        }
    }

    async fn transition_run(
        &mut self,
        next: ProgramRunPhase,
        failure_code: Option<&'static str>,
    ) -> Result<(), ApiError> {
        self.revision = self
            .runner
            .store
            .transition_program(
                self.program_id,
                self.phase,
                next,
                self.revision,
                self.runner.clock.now_ms(),
                failure_code,
            )
            .await?;
        self.phase = next;
        Ok(())
    }

    async fn transition_segment(
        &mut self,
        segment_id: Uuid,
        next: ProgramSegmentPhase,
        event_state: Option<ProgramSegmentEventState>,
        failure_code: Option<&'static str>,
    ) -> Result<(), ApiError> {
        let expected = self
            .segment_states
            .get(&segment_id)
            .copied()
            .ok_or_else(ApiError::unexpected)?;
        self.runner
            .store
            .transition_segment(
                self.program_id,
                segment_id,
                expected,
                next,
                self.runner.clock.now_ms(),
                failure_code,
            )
            .await?;
        self.segment_states.insert(segment_id, next);
        if let Some(event_state) = event_state {
            self.runner
                .events
                .segment(self.program_id, segment_id, event_state);
        }
        Ok(())
    }

    async fn stop(&mut self) -> Result<u64, ApiError> {
        if !matches!(
            self.phase,
            ProgramRunPhase::Stopping | ProgramRunPhase::Completed | ProgramRunPhase::Failed
        ) {
            self.transition_run(ProgramRunPhase::Stopping, Some("user_stop"))
                .await?;
            self.runner
                .events
                .state(self.program_id, ProgramEventState::Stopping, None);
        }
        if let Some(segment_id) = self.active.current_speech() {
            self.runner.speech.cancel(segment_id);
        }
        let _ = self.runner.playback.stop().await;
        self.cancel_unfinished_segments().await?;
        self.transition_run(ProgramRunPhase::Completed, Some("user_stop"))
            .await?;
        self.runner
            .events
            .state(self.program_id, ProgramEventState::Completed, None);
        Ok(self.revision)
    }

    async fn fail(&mut self) -> Result<u64, ApiError> {
        if let Some(segment_id) = self.active.current_speech() {
            self.runner.speech.cancel(segment_id);
        }
        let _ = self.runner.playback.stop().await;
        self.fail_unfinished_segments().await?;
        self.transition_run(ProgramRunPhase::Failed, Some("runner_failed"))
            .await?;
        self.runner.events.state(
            self.program_id,
            ProgramEventState::Failed,
            Some(PROGRAM_FAILED_MESSAGE),
        );
        Ok(self.revision)
    }

    async fn cancel_unfinished_segments(&mut self) -> Result<(), ApiError> {
        let unfinished = self
            .segment_states
            .iter()
            .filter(|(_, state)| !segment_terminal(**state))
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for segment_id in unfinished {
            self.transition_segment(
                segment_id,
                ProgramSegmentPhase::Cancelled,
                Some(ProgramSegmentEventState::Skipped),
                Some("user_stop"),
            )
            .await?;
        }
        Ok(())
    }

    async fn fail_unfinished_segments(&mut self) -> Result<(), ApiError> {
        let unfinished = self
            .segment_states
            .iter()
            .filter(|(_, state)| !segment_terminal(**state))
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for segment_id in unfinished {
            self.transition_segment(
                segment_id,
                ProgramSegmentPhase::Failed,
                Some(ProgramSegmentEventState::Failed),
                Some("runner_failed"),
            )
            .await?;
        }
        Ok(())
    }

    fn emit_running_once(&mut self) {
        if !self.running_emitted {
            let safe_message = self
                .planned
                .degradation
                .is_some()
                .then_some(PLAN_DEGRADED_MESSAGE);
            self.runner
                .events
                .state(self.program_id, ProgramEventState::Running, safe_message);
            self.running_emitted = true;
        }
    }

    fn emit_speech_degraded_once(&mut self) {
        if !self.speech_degraded_emitted {
            self.runner.events.state(
                self.program_id,
                ProgramEventState::Running,
                Some(SPEECH_DEGRADED_MESSAGE),
            );
            self.speech_degraded_emitted = true;
        }
    }

    fn cancelled(&self) -> bool {
        *self.active.cancellation.borrow()
    }

    fn ensure_not_cancelled(&self) -> Result<(), ApiError> {
        if self.cancelled() {
            Err(ApiError::from_reason(InternalReason::OperationCancelled))
        } else {
            Ok(())
        }
    }
}

fn program_track(track: &ProgramPlanTrackSegment) -> Result<ProgramTrack, ApiError> {
    Ok(ProgramTrack {
        segment_id: parse_v7(&track.segment_id)?,
        track_id: track.track_id.clone(),
    })
}

fn segment_id(segment: &ProgramPlanSegmentsItem) -> Result<Uuid, ApiError> {
    match segment {
        ProgramPlanSegmentsItem::TrackSegment(track) => parse_v7(&track.segment_id),
        ProgramPlanSegmentsItem::VoiceSegment(voice) => parse_v7(&voice.segment_id),
    }
}

fn parse_v7(value: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(value)
        .ok()
        .filter(|id| id.get_version_num() == 7 && id.to_string() == value)
        .ok_or_else(|| ApiError::from_reason(InternalReason::ProviderInvalidResponse))
}

const fn segment_terminal(state: ProgramSegmentPhase) -> bool {
    matches!(
        state,
        ProgramSegmentPhase::Completed
            | ProgramSegmentPhase::Skipped
            | ProgramSegmentPhase::Failed
            | ProgramSegmentPhase::Cancelled
    )
}
