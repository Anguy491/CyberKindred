use super::*;
use crate::{
    providers::{
        CancellationFlag, Clock, ProviderCallContext, ProviderFailure, ProviderFailureCategory,
    },
    storage::{CanonicalOrigin, SecretValue},
};
use chrono::{DateTime, Utc};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, AtomicUsize, Ordering},
    },
    time::Duration,
};
use uuid::Uuid;

#[derive(Clone)]
struct FakeClock(Arc<AtomicI64>);

impl FakeClock {
    fn new(now_ms: i64) -> Self {
        Self(Arc::new(AtomicI64::new(now_ms)))
    }

    fn advance(&self, millis: i64) {
        self.0.fetch_add(millis, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::from_timestamp_millis(self.0.load(Ordering::SeqCst)).expect("fake time is valid")
    }
}

struct FakeCredentials {
    loads: AtomicUsize,
}

impl FakeCredentials {
    fn new() -> Self {
        Self {
            loads: AtomicUsize::new(0),
        }
    }
}

impl SpeechCredentialSource for FakeCredentials {
    fn load(&self) -> Result<SecretValue, ProviderFailure> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        SecretValue::new("sk-test-canary-never-captured".to_owned())
            .map_err(|_| ProviderFailure::new(ProviderFailureCategory::Authentication))
    }
}

enum FakeResponse {
    Immediate(Result<SpeechTransportResponse, ProviderFailure>),
    Blocked {
        release: Arc<tokio::sync::Notify>,
        response: Result<SpeechTransportResponse, ProviderFailure>,
    },
}

struct CapturedSpeechRequest {
    text: String,
    voice_id: String,
    model_id: String,
    format: AudioFormat,
    speed: f32,
    locale: String,
    provenance: SpeechProvenance,
    timeout: Duration,
}

struct FakeTransport {
    responses: Mutex<VecDeque<FakeResponse>>,
    captured: Mutex<Vec<CapturedSpeechRequest>>,
    calls: AtomicUsize,
}

impl FakeTransport {
    fn new(responses: Vec<FakeResponse>) -> Self {
        Self {
            responses: Mutex::new(responses.into()),
            captured: Mutex::new(Vec::new()),
            calls: AtomicUsize::new(0),
        }
    }
}

impl SpeechTransport for FakeTransport {
    fn send<'a>(
        &'a self,
        request: &'a SpeechTransportRequest,
        _secret: &'a SecretValue,
        timeout: Duration,
    ) -> provider::SpeechFuture<'a, Result<SpeechTransportResponse, ProviderFailure>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.captured
            .lock()
            .expect("capture lock")
            .push(CapturedSpeechRequest {
                text: request.input().text().to_owned(),
                voice_id: request.input().voice_id().to_owned(),
                model_id: request.input().model_id().to_owned(),
                format: request.input().format(),
                speed: request.input().speed(),
                locale: request.input().locale().to_owned(),
                provenance: request.input().provenance(),
                timeout,
            });
        let response = self
            .responses
            .lock()
            .expect("response lock")
            .pop_front()
            .unwrap_or_else(|| {
                FakeResponse::Immediate(Err(ProviderFailure::new(
                    ProviderFailureCategory::Unavailable,
                )))
            });
        Box::pin(async move {
            match response {
                FakeResponse::Immediate(result) => result,
                FakeResponse::Blocked { release, response } => {
                    release.notified().await;
                    response
                }
            }
        })
    }
}

struct FakePlayback {
    plays: AtomicUsize,
    stops: AtomicUsize,
    blocked: bool,
    release: Arc<tokio::sync::Notify>,
}

impl FakePlayback {
    fn new(blocked: bool) -> Self {
        Self {
            plays: AtomicUsize::new(0),
            stops: AtomicUsize::new(0),
            blocked,
            release: Arc::new(tokio::sync::Notify::new()),
        }
    }
}

impl SpeechPlayback for FakePlayback {
    fn play(
        &self,
        _operation_id: Uuid,
        _bytes: Arc<[u8]>,
        operation_cancellation: CancellationFlag,
        caller_cancellation: CancellationFlag,
    ) -> provider::SpeechFuture<'_, Result<(), ProviderFailure>> {
        if operation_cancellation.is_cancelled() || caller_cancellation.is_cancelled() {
            return Box::pin(async {
                Err(ProviderFailure::new(ProviderFailureCategory::Unavailable))
            });
        }
        self.plays.fetch_add(1, Ordering::SeqCst);
        let blocked = self.blocked;
        let release = Arc::clone(&self.release);
        Box::pin(async move {
            if blocked {
                release.notified().await;
            }
            if operation_cancellation.is_cancelled() || caller_cancellation.is_cancelled() {
                Err(ProviderFailure::new(ProviderFailureCategory::Unavailable))
            } else {
                Ok(())
            }
        })
    }

    fn stop(&self, _operation_id: Uuid) {
        self.stops.fetch_add(1, Ordering::SeqCst);
        self.release.notify_waiters();
    }
}

fn mp3() -> SpeechTransportResponse {
    SpeechTransportResponse {
        bytes: vec![0xff, 0xfb, 0x90, 0x64, 1, 2, 3, 4],
        declared_mime: "audio/mpeg".to_owned(),
    }
}

fn segment(text: &str) -> SpeechInput {
    SpeechInput::segment(
        text.to_owned(),
        "alloy".to_owned(),
        "gpt-4o-mini-tts".to_owned(),
        1.0,
        "zh-CN".to_owned(),
        SpeechProvenance::LocalProgram,
    )
    .expect("valid segment")
}

fn actor(
    transport: Arc<FakeTransport>,
    credentials: Arc<FakeCredentials>,
    playback: Arc<FakePlayback>,
    clock: &FakeClock,
) -> SpeechActor {
    let provider = Arc::new(OpenAiTtsProvider::new(transport, credentials));
    SpeechActor::new(
        provider,
        SpeechCache::new(8, 1_024),
        playback,
        Arc::new(clock.clone()),
        32,
    )
}

#[test]
fn tts_provider_input_bounds_preview_and_cache_key_are_deterministic() {
    let preview = SpeechInput::voice_preview(
        "alloy".to_owned(),
        "gpt-4o-mini-tts".to_owned(),
        1.0,
        "zh-CN".to_owned(),
    )
    .expect("valid preview");
    assert_eq!(preview.text(), VOICE_PREVIEW_TEXT_V1);
    assert_eq!(preview.provenance(), SpeechProvenance::VoicePreview);
    assert!(
        SpeechInput::segment(
            "caller text".to_owned(),
            "alloy".to_owned(),
            "gpt-4o-mini-tts".to_owned(),
            1.0,
            "zh-CN".to_owned(),
            SpeechProvenance::VoicePreview,
        )
        .is_err()
    );
    assert!(
        SpeechInput::segment(
            "界".repeat(500),
            "alloy".to_owned(),
            "gpt-4o-mini-tts".to_owned(),
            0.75,
            "zh-CN".to_owned(),
            SpeechProvenance::Chat,
        )
        .is_ok()
    );
    assert!(
        SpeechInput::segment(
            "界".repeat(501),
            "alloy".to_owned(),
            "gpt-4o-mini-tts".to_owned(),
            1.0,
            "zh-CN".to_owned(),
            SpeechProvenance::Chat,
        )
        .is_err()
    );
    let normalized = segment("hello   world");
    assert_eq!(normalized.cache_key(), segment(" hello world ").cache_key());
    assert_ne!(
        normalized.cache_key(),
        SpeechInput::segment(
            "hello world".to_owned(),
            "echo".to_owned(),
            "gpt-4o-mini-tts".to_owned(),
            1.0,
            "zh-CN".to_owned(),
            SpeechProvenance::LocalProgram,
        )
        .expect("valid variant")
        .cache_key()
    );
}

#[tokio::test]
async fn tts_actor_disabled_is_zero_provider_credential_and_audio_calls() {
    let clock = FakeClock::new(1_000);
    let transport = Arc::new(FakeTransport::new(vec![]));
    let credentials = Arc::new(FakeCredentials::new());
    let playback = Arc::new(FakePlayback::new(false));
    let actor = actor(
        Arc::clone(&transport),
        Arc::clone(&credentials),
        Arc::clone(&playback),
        &clock,
    );
    let operation_id = Uuid::now_v7();
    let outcome = actor
        .run(
            operation_id,
            segment("visible fallback"),
            SpeechOwner::Segment(Uuid::now_v7()),
            false,
            None,
            &ProviderCallContext::new(Duration::from_secs(5)),
        )
        .await
        .expect("disabled is a deterministic fallback");
    assert_eq!(
        outcome,
        SpeechRunOutcome::TextOnly {
            text: "visible fallback".to_owned(),
            reason: SpeechFallbackReason::Disabled,
        }
    );
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    assert_eq!(credentials.loads.load(Ordering::SeqCst), 0);
    assert_eq!(playback.plays.load(Ordering::SeqCst), 0);
    assert_eq!(
        actor.operation_state(operation_id),
        Some(SpeechOperationState::TextOnly)
    );

    assert!(
        actor
            .run(
                Uuid::now_v7(),
                segment("unconfirmed must stay silent"),
                SpeechOwner::Segment(Uuid::now_v7()),
                true,
                None,
                &ProviderCallContext::new(Duration::from_secs(5)),
            )
            .await
            .is_err()
    );
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    assert_eq!(credentials.loads.load(Ordering::SeqCst), 0);
    assert_eq!(playback.plays.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn tts_provider_request_is_closed_bounded_and_secret_free() {
    let clock = FakeClock::new(1_000);
    let transport = Arc::new(FakeTransport::new(vec![FakeResponse::Immediate(Ok(mp3()))]));
    let credentials = Arc::new(FakeCredentials::new());
    let playback = Arc::new(FakePlayback::new(false));
    let actor = actor(
        Arc::clone(&transport),
        Arc::clone(&credentials),
        Arc::clone(&playback),
        &clock,
    );
    let operation_id = Uuid::now_v7();
    let outcome = actor
        .run(
            operation_id,
            segment("  hello   radio "),
            SpeechOwner::Segment(Uuid::now_v7()),
            true,
            Some(SpeechAuthorization::ConfirmedProgram(Uuid::now_v7())),
            &ProviderCallContext::new(Duration::from_secs(60)),
        )
        .await
        .expect("synthesis succeeds");
    assert!(matches!(outcome, SpeechRunOutcome::Spoken { .. }));
    assert_eq!(credentials.loads.load(Ordering::SeqCst), 1);
    assert_eq!(playback.plays.load(Ordering::SeqCst), 1);
    let captured = transport.captured.lock().expect("capture lock");
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].text, "hello radio");
    assert_eq!(captured[0].voice_id, "alloy");
    assert_eq!(captured[0].model_id, "gpt-4o-mini-tts");
    assert_eq!(captured[0].format, AudioFormat::Mp3);
    assert!((captured[0].speed - 1.0).abs() < f32::EPSILON);
    assert_eq!(captured[0].locale, "zh-CN");
    assert_eq!(captured[0].provenance, SpeechProvenance::LocalProgram);
    assert!(captured[0].timeout <= Duration::from_secs(45));

    let request_json = provider::serialized_request_for_test(&segment("hello radio"));
    let body: serde_json::Value = serde_json::from_slice(&request_json).expect("request json");
    assert_eq!(body.as_object().expect("object").len(), 5);
    assert_eq!(body["response_format"], "mp3");
    assert!(!String::from_utf8_lossy(&request_json).contains("sk-test-canary"));
    ReqwestSpeechTransport::new(
        &CanonicalOrigin::parse("https://api.openai.com").expect("canonical origin"),
    )
    .expect("production client constructs without network");
}

#[tokio::test]
async fn tts_actor_success_cache_hit_has_owner_refs_and_releases_playback_lease() {
    let clock = FakeClock::new(1_000);
    let transport = Arc::new(FakeTransport::new(vec![FakeResponse::Immediate(Ok(mp3()))]));
    let credentials = Arc::new(FakeCredentials::new());
    let playback = Arc::new(FakePlayback::new(false));
    let cache = SpeechCache::new(8, 1_024);
    let provider = Arc::new(OpenAiTtsProvider::new(
        Arc::clone(&transport) as Arc<dyn SpeechTransport>,
        Arc::clone(&credentials) as Arc<dyn SpeechCredentialSource>,
    ));
    let actor = SpeechActor::new(
        provider,
        cache.clone(),
        Arc::clone(&playback) as Arc<dyn SpeechPlayback>,
        Arc::new(clock),
        32,
    );
    let input = segment("cache me");
    for _ in 0..2 {
        actor
            .run(
                Uuid::now_v7(),
                input.clone(),
                SpeechOwner::Segment(Uuid::now_v7()),
                true,
                Some(SpeechAuthorization::ConfirmedProgram(Uuid::now_v7())),
                &ProviderCallContext::new(Duration::from_secs(5)),
            )
            .await
            .expect("run succeeds");
    }
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    assert_eq!(credentials.loads.load(Ordering::SeqCst), 1);
    assert_eq!(playback.plays.load(Ordering::SeqCst), 2);
    let metadata = cache.snapshot_metadata();
    assert_eq!(metadata.len(), 1);
    assert_eq!(metadata[0].owners.len(), 2);
    assert_eq!(metadata[0].active_lease_count, 0);
}

#[tokio::test]
async fn tts_actor_failure_is_once_only_and_preserves_visible_text() {
    let clock = FakeClock::new(1_000);
    let transport = Arc::new(FakeTransport::new(vec![FakeResponse::Immediate(Err(
        ProviderFailure::new(ProviderFailureCategory::Timeout),
    ))]));
    let credentials = Arc::new(FakeCredentials::new());
    let playback = Arc::new(FakePlayback::new(false));
    let actor = actor(
        Arc::clone(&transport),
        credentials,
        Arc::clone(&playback),
        &clock,
    );
    let operation_id = Uuid::now_v7();
    let outcome = actor
        .run(
            operation_id,
            segment("still readable"),
            SpeechOwner::Segment(Uuid::now_v7()),
            true,
            Some(SpeechAuthorization::ConfirmedProgram(Uuid::now_v7())),
            &ProviderCallContext::new(Duration::from_secs(5)),
        )
        .await
        .expect("failure degrades to text");
    assert_eq!(
        outcome,
        SpeechRunOutcome::TextOnly {
            text: "still readable".to_owned(),
            reason: SpeechFallbackReason::Provider(ProviderFailureCategory::Timeout),
        }
    );
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    assert_eq!(playback.plays.load(Ordering::SeqCst), 0);
    assert_eq!(
        actor.cancel(operation_id),
        SpeechCancelState::AlreadyTerminal
    );
}

#[tokio::test]
async fn tts_actor_cancel_during_synthesis_is_idempotent_and_never_plays_or_caches() {
    let clock = FakeClock::new(1_000);
    let release = Arc::new(tokio::sync::Notify::new());
    let transport = Arc::new(FakeTransport::new(vec![FakeResponse::Blocked {
        release: Arc::clone(&release),
        response: Ok(mp3()),
    }]));
    let credentials = Arc::new(FakeCredentials::new());
    let playback = Arc::new(FakePlayback::new(false));
    let cache = SpeechCache::new(8, 1_024);
    let actor = Arc::new(SpeechActor::new(
        Arc::new(OpenAiTtsProvider::new(
            Arc::clone(&transport) as Arc<dyn SpeechTransport>,
            credentials,
        )),
        cache.clone(),
        Arc::clone(&playback) as Arc<dyn SpeechPlayback>,
        Arc::new(clock),
        32,
    ));
    let operation_id = Uuid::now_v7();
    let task_actor = Arc::clone(&actor);
    let task = tokio::spawn(async move {
        task_actor
            .run(
                operation_id,
                segment("cancel before sound"),
                SpeechOwner::Segment(Uuid::now_v7()),
                true,
                Some(SpeechAuthorization::ConfirmedProgram(Uuid::now_v7())),
                &ProviderCallContext::new(Duration::from_secs(5)),
            )
            .await
    });
    while transport.calls.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    assert_eq!(actor.cancel(operation_id), SpeechCancelState::Cancelled);
    assert_eq!(
        actor.cancel(operation_id),
        SpeechCancelState::AlreadyTerminal
    );
    release.notify_waiters();
    let outcome = task.await.expect("task joins").expect("actor outcome");
    assert_eq!(
        outcome,
        SpeechRunOutcome::TextOnly {
            text: "cancel before sound".to_owned(),
            reason: SpeechFallbackReason::Cancelled,
        }
    );
    assert_eq!(playback.plays.load(Ordering::SeqCst), 0);
    assert!(cache.snapshot_metadata().is_empty());
}

#[tokio::test]
async fn tts_actor_second_click_cancel_stops_exact_playback_once() {
    let clock = FakeClock::new(1_000);
    let transport = Arc::new(FakeTransport::new(vec![FakeResponse::Immediate(Ok(mp3()))]));
    let credentials = Arc::new(FakeCredentials::new());
    let playback = Arc::new(FakePlayback::new(true));
    let actor = Arc::new(actor(transport, credentials, Arc::clone(&playback), &clock));
    let operation_id = Uuid::now_v7();
    let task_actor = Arc::clone(&actor);
    let task = tokio::spawn(async move {
        task_actor
            .run(
                operation_id,
                SpeechInput::voice_preview(
                    "alloy".to_owned(),
                    "gpt-4o-mini-tts".to_owned(),
                    1.0,
                    "zh-CN".to_owned(),
                )
                .expect("preview"),
                SpeechOwner::Preview(operation_id),
                true,
                Some(SpeechAuthorization::UserRequestedPreview(operation_id)),
                &ProviderCallContext::new(Duration::from_secs(5)),
            )
            .await
    });
    while playback.plays.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    assert_eq!(actor.cancel(operation_id), SpeechCancelState::Cancelled);
    assert_eq!(
        actor.cancel(operation_id),
        SpeechCancelState::AlreadyTerminal
    );
    let outcome = task.await.expect("task joins").expect("actor result");
    assert!(matches!(
        outcome,
        SpeechRunOutcome::TextOnly {
            reason: SpeechFallbackReason::Cancelled,
            ..
        }
    ));
    assert_eq!(playback.stops.load(Ordering::SeqCst), 1);
}

#[test]
fn tts_cache_lru_never_evicts_an_active_lease() {
    let clock = FakeClock::new(1_000);
    let cache = SpeechCache::new(1, 64);
    let first = segment("first");
    let second = segment("second");
    let first_owner = SpeechOwner::Segment(Uuid::now_v7());
    cache
        .insert(
            &first,
            SpeechArtifact::new(&first, mp3().bytes.len()).expect("artifact"),
            mp3().bytes,
            first_owner,
            clock.now_ms(),
        )
        .expect("first insert");
    let lease = cache
        .acquire(first.cache_key(), first_owner, clock.now_ms())
        .expect("lease");
    assert!(
        cache
            .insert(
                &second,
                SpeechArtifact::new(&second, mp3().bytes.len()).expect("artifact"),
                mp3().bytes,
                SpeechOwner::Segment(Uuid::now_v7()),
                clock.now_ms(),
            )
            .is_err()
    );
    assert_eq!(cache.snapshot_metadata()[0].active_lease_count, 1);
    drop(lease);
    cache
        .insert(
            &second,
            SpeechArtifact::new(&second, mp3().bytes.len()).expect("artifact"),
            mp3().bytes,
            SpeechOwner::Segment(Uuid::now_v7()),
            clock.now_ms(),
        )
        .expect("second insert after lease release");
    assert_eq!(cache.snapshot_metadata().len(), 1);
    assert_eq!(
        cache.snapshot_metadata()[0].content_hash,
        second.cache_key().to_hex()
    );
    clock.advance(SPEECH_CACHE_TTL_MS);
    cache.prune(clock.now_ms());
    assert!(cache.snapshot_metadata().is_empty());
}

#[test]
fn tts_provider_rejects_mime_and_framing_mismatch() {
    assert!(provider::validate_mp3(&[0xff, 0xfb, 0x90, 0x64], "audio/mpeg").is_ok());
    assert!(provider::validate_mp3(b"<html>error</html>", "audio/mpeg").is_err());
    assert!(provider::validate_mp3(&[0xff, 0xfb, 0x90, 0x64], "text/html").is_err());
    assert!(provider::validate_mp3(b"ID3\x04\0\0\0\0\0\0", "audio/mpeg").is_err());
}
