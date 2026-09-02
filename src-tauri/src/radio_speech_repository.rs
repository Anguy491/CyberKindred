//! Current-settings and origin-scoped credential adapter for program speech.

use std::{
    future::{Future, pending},
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};

use tokio::sync::watch;
use uuid::Uuid;

use crate::{
    ipc::{ApiError, InternalReason},
    providers::{ProviderCallContext, ProviderFailure, ProviderFailureCategory, SystemClock},
    radio::{ConfirmedProgramStart, ProgramSpeech, ProgramSpeechOutcome},
    speech::{
        CallScopedOpenAiTtsProvider, ReqwestSpeechTransportFactory, RodioSpeechPlayback,
        SpeechActor, SpeechArtifact, SpeechAuthorization, SpeechCache, SpeechInput, SpeechOwner,
        SpeechPlayback, SpeechProvenance, SpeechRunOutcome, SpeechSink, SpeechTransportFactory,
        TtsProvider,
    },
    storage::{
        CanonicalOrigin, CredentialTarget, Repository, SecretVault, StorageError, StorageReason,
    },
};

const PROGRAM_SPEECH_DEADLINE: Duration = Duration::from_secs(60);
const PROGRAM_CACHE_ENTRIES: usize = 128;
const PROGRAM_CACHE_BYTES: usize = 64 * 1_024 * 1_024;
const PROGRAM_TERMINAL_CAPACITY: usize = 512;

/// Repository-backed program speech that materializes credentials only for one
/// enabled, authorized segment call.
///
/// This type deliberately implements neither `Debug` nor serialization.
pub(crate) struct RepositoryProgramSpeech {
    repository: Repository,
    vault: Mutex<Box<dyn SecretVault>>,
    actor: Arc<SpeechActor>,
    transports: Arc<dyn SpeechTransportFactory>,
}

impl RepositoryProgramSpeech {
    /// Creates an injected adapter without reading settings/credentials or
    /// touching the network/audio device.
    #[must_use]
    pub(crate) fn new(
        repository: Repository,
        vault: Box<dyn SecretVault>,
        actor: Arc<SpeechActor>,
        transports: Arc<dyn SpeechTransportFactory>,
    ) -> Self {
        Self {
            repository,
            vault: Mutex::new(vault),
            actor,
            transports,
        }
    }

    /// Creates the production adapter without reading a credential, opening a
    /// connection, or opening an audio device. Those effects remain inside an
    /// explicitly confirmed `present` call.
    #[must_use]
    pub(crate) fn production(repository: Repository, vault: Box<dyn SecretVault>) -> Self {
        let actor = Arc::new(SpeechActor::new(
            Arc::new(CallScopedOnlyProvider),
            SpeechCache::new(PROGRAM_CACHE_ENTRIES, PROGRAM_CACHE_BYTES),
            Arc::new(RodioSpeechPlayback::new()) as Arc<dyn SpeechPlayback>,
            Arc::new(SystemClock),
            PROGRAM_TERMINAL_CAPACITY,
        ));
        Self::new(
            repository,
            vault,
            actor,
            Arc::new(ReqwestSpeechTransportFactory),
        )
    }

    fn load_secret(&self, target: &CredentialTarget) -> Option<crate::storage::SecretValue> {
        let vault = self.vault.lock().ok()?;
        vault.get(target).ok().flatten()
    }
}

impl ProgramSpeech for RepositoryProgramSpeech {
    fn present<'a>(
        &'a self,
        program_id: Uuid,
        segment_id: Uuid,
        text: &'a str,
        _authorization: ConfirmedProgramStart,
        mut cancellation: watch::Receiver<bool>,
    ) -> Pin<Box<dyn Future<Output = Result<ProgramSpeechOutcome, ApiError>> + Send + 'a>> {
        Box::pin(async move {
            ensure_not_cancelled(&cancellation)?;
            let load_settings = self.repository.load_provider_settings();
            tokio::pin!(load_settings);
            let settings = tokio::select! {
                biased;
                () = wait_for_cancellation(&mut cancellation) => {
                    return Err(cancelled_error());
                }
                result = &mut load_settings => result.map_err(|error| map_storage_error(&error))?,
            };
            ensure_not_cancelled(&cancellation)?;
            if !settings.tts_enabled {
                return Ok(ProgramSpeechOutcome::TextOnly);
            }

            let Ok(input) = SpeechInput::segment(
                text,
                settings.tts_voice_id,
                settings.tts_model_id,
                1.0,
                "zh-CN".to_owned(),
                SpeechProvenance::LocalProgram,
            ) else {
                return Ok(ProgramSpeechOutcome::TextOnly);
            };
            let Ok(origin) = CanonicalOrigin::parse(&settings.provider_origin) else {
                return Ok(ProgramSpeechOutcome::TextOnly);
            };
            ensure_not_cancelled(&cancellation)?;
            let Some(secret) = self.load_secret(&CredentialTarget::openai(&origin)) else {
                return Ok(ProgramSpeechOutcome::TextOnly);
            };
            ensure_not_cancelled(&cancellation)?;
            let Ok(transport) = self.transports.for_origin(&origin) else {
                return Ok(ProgramSpeechOutcome::TextOnly);
            };
            let provider = CallScopedOpenAiTtsProvider::new(transport.as_ref(), &secret);
            let context = ProviderCallContext::new(PROGRAM_SPEECH_DEADLINE);
            let caller_cancellation = context.cancellation.clone();
            let run = self.actor.run_with_provider(
                &provider,
                segment_id,
                input,
                SpeechOwner::Segment(segment_id),
                true,
                Some(SpeechAuthorization::ConfirmedProgram(program_id)),
                &context,
            );
            tokio::pin!(run);
            tokio::select! {
                biased;
                () = wait_for_cancellation(&mut cancellation) => {
                    caller_cancellation.cancel();
                    let _ = self.actor.cancel(segment_id);
                    Err(cancelled_error())
                }
                result = &mut run => {
                    match result.map_err(|failure| failure.into_api_error(false))? {
                        SpeechRunOutcome::Spoken { .. } => Ok(ProgramSpeechOutcome::Spoken),
                        SpeechRunOutcome::TextOnly { .. } => Ok(ProgramSpeechOutcome::TextOnly),
                    }
                }
            }
        })
    }

    fn cancel(&self, segment_id: Uuid) {
        let _ = self.actor.cancel(segment_id);
    }
}

struct CallScopedOnlyProvider;

impl TtsProvider for CallScopedOnlyProvider {
    fn synthesize<'a>(
        &'a self,
        _input: SpeechInput,
        _sink: &'a mut dyn SpeechSink,
        _context: &'a ProviderCallContext,
    ) -> Pin<Box<dyn Future<Output = Result<SpeechArtifact, ProviderFailure>> + Send + 'a>> {
        Box::pin(async { Err(ProviderFailure::new(ProviderFailureCategory::Unavailable)) })
    }
}

fn ensure_not_cancelled(cancellation: &watch::Receiver<bool>) -> Result<(), ApiError> {
    if *cancellation.borrow() {
        Err(cancelled_error())
    } else {
        Ok(())
    }
}

async fn wait_for_cancellation(cancellation: &mut watch::Receiver<bool>) {
    loop {
        if *cancellation.borrow() {
            return;
        }
        if cancellation.changed().await.is_err() {
            pending::<()>().await;
        }
    }
}

fn cancelled_error() -> ApiError {
    ApiError::from_reason(InternalReason::OperationCancelled)
}

fn map_storage_error(error: &StorageError) -> ApiError {
    let reason = match error.reason() {
        StorageReason::PathDenied => InternalReason::PathDenied,
        StorageReason::PathOutsideScope => InternalReason::PathOutsideRoot,
        StorageReason::UnsafeReparsePoint => InternalReason::UnsafeReparsePoint,
        StorageReason::StorageReadFailed => InternalReason::StorageReadFailed,
        StorageReason::StorageWriteFailed | StorageReason::InvalidSetting => {
            InternalReason::StorageWriteFailed
        }
        StorageReason::StorageIntegrityFailed | StorageReason::ForeignDatabase => {
            InternalReason::StorageIntegrityFailed
        }
        StorageReason::MigrationFailed => InternalReason::MigrationFailed,
        StorageReason::DatabaseVersionUnsupported => InternalReason::DatabaseVersionUnsupported,
        StorageReason::EntityNotFound => InternalReason::EntityNotFound,
        StorageReason::RevisionConflict => InternalReason::RevisionConflict,
        StorageReason::ResourceBusy => InternalReason::ResourceBusy,
    };
    ApiError::from_reason(reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use crate::{
        ipc::ErrorId,
        providers::CancellationFlag,
        speech::{SpeechTransport, SpeechTransportRequest, SpeechTransportResponse},
        storage::{AppPaths, SecretError, SecretValue, Storage},
    };

    const SECRET_CANARY: &str = "sk-program-speech-canary-never-visible";

    struct FakeVault {
        secret: Option<String>,
        gets: Arc<AtomicUsize>,
    }

    impl SecretVault for FakeVault {
        fn set(
            &mut self,
            _target: &CredentialTarget,
            value: SecretValue,
        ) -> Result<(), SecretError> {
            self.secret = Some(value.with_exposed(str::to_owned));
            Ok(())
        }

        fn get(&self, _target: &CredentialTarget) -> Result<Option<SecretValue>, SecretError> {
            self.gets.fetch_add(1, Ordering::SeqCst);
            self.secret
                .as_ref()
                .map(|value| SecretValue::new(value.clone()))
                .transpose()
        }

        fn contains(&self, _target: &CredentialTarget) -> Result<bool, SecretError> {
            Ok(self.secret.is_some())
        }

        fn delete(&mut self, _target: &CredentialTarget) -> Result<(), SecretError> {
            self.secret = None;
            Ok(())
        }

        fn delete_cyberkindred_namespace(&mut self) -> Result<u32, SecretError> {
            let removed = u32::from(self.secret.take().is_some());
            Ok(removed)
        }
    }

    struct FakeTransport {
        calls: Arc<AtomicUsize>,
        secret_matched: Arc<AtomicBool>,
    }

    impl SpeechTransport for FakeTransport {
        fn send<'a>(
            &'a self,
            _request: &'a SpeechTransportRequest,
            secret: &'a SecretValue,
            _timeout: Duration,
        ) -> Pin<
            Box<dyn Future<Output = Result<SpeechTransportResponse, ProviderFailure>> + Send + 'a>,
        > {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.secret_matched.store(
                secret.with_exposed(|value| value == SECRET_CANARY),
                Ordering::SeqCst,
            );
            Box::pin(async {
                Ok(SpeechTransportResponse {
                    bytes: vec![0xff, 0xfb, 0x90, 0x64, 1, 2, 3, 4],
                    declared_mime: "audio/mpeg".to_owned(),
                })
            })
        }
    }

    struct FakeTransportFactory {
        calls: Arc<AtomicUsize>,
        transport: Arc<FakeTransport>,
    }

    impl SpeechTransportFactory for FakeTransportFactory {
        fn for_origin(
            &self,
            _origin: &CanonicalOrigin,
        ) -> Result<Arc<dyn SpeechTransport>, ProviderFailure> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::clone(&self.transport) as Arc<dyn SpeechTransport>)
        }
    }

    struct FakePlayback {
        blocked: bool,
        plays: AtomicUsize,
        completions: AtomicUsize,
        stops: AtomicUsize,
        stopped: Mutex<Vec<Uuid>>,
        started: tokio::sync::Notify,
        release: Arc<tokio::sync::Notify>,
    }

    impl FakePlayback {
        fn new(blocked: bool) -> Self {
            Self {
                blocked,
                plays: AtomicUsize::new(0),
                completions: AtomicUsize::new(0),
                stops: AtomicUsize::new(0),
                stopped: Mutex::new(Vec::new()),
                started: tokio::sync::Notify::new(),
                release: Arc::new(tokio::sync::Notify::new()),
            }
        }
    }

    impl SpeechPlayback for FakePlayback {
        fn play(
            &self,
            _operation_id: Uuid,
            _bytes: Arc<[u8]>,
            _authorization: SpeechAuthorization,
            operation_cancellation: CancellationFlag,
            caller_cancellation: CancellationFlag,
        ) -> Pin<Box<dyn Future<Output = Result<(), ProviderFailure>> + Send + '_>> {
            self.plays.fetch_add(1, Ordering::SeqCst);
            self.started.notify_one();
            let blocked = self.blocked;
            let release = Arc::clone(&self.release);
            Box::pin(async move {
                if blocked {
                    release.notified().await;
                }
                if operation_cancellation.is_cancelled() || caller_cancellation.is_cancelled() {
                    return Err(ProviderFailure::new(ProviderFailureCategory::Unavailable));
                }
                self.completions.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }

        fn stop(&self, operation_id: Uuid) {
            self.stops.fetch_add(1, Ordering::SeqCst);
            if let Ok(mut stopped) = self.stopped.lock() {
                stopped.push(operation_id);
            }
            self.release.notify_waiters();
        }
    }

    async fn fixture() -> (tempfile::TempDir, Storage, Repository) {
        let temp = tempfile::tempdir().expect("temporary root");
        let paths = AppPaths::create(
            temp.path().join("data"),
            temp.path().join("cache"),
            temp.path().join("logs"),
        )
        .expect("app paths");
        let storage = Storage::open(&paths, "0.3.0").await.expect("storage");
        let repository = storage.repository();
        (temp, storage, repository)
    }

    async fn enable_tts(repository: &Repository) {
        let mut settings = repository.load_provider_settings().await.expect("settings");
        settings.tts_enabled = true;
        repository
            .save_provider_settings(0, &settings, 1, false, None)
            .await
            .expect("enable tts");
    }

    fn source(
        repository: Repository,
        secret: Option<&str>,
        blocked_playback: bool,
    ) -> (
        Arc<RepositoryProgramSpeech>,
        Arc<AtomicUsize>,
        Arc<AtomicUsize>,
        Arc<FakeTransport>,
        Arc<FakePlayback>,
    ) {
        let vault_gets = Arc::new(AtomicUsize::new(0));
        let factory_calls = Arc::new(AtomicUsize::new(0));
        let transport = Arc::new(FakeTransport {
            calls: Arc::new(AtomicUsize::new(0)),
            secret_matched: Arc::new(AtomicBool::new(false)),
        });
        let factory = Arc::new(FakeTransportFactory {
            calls: Arc::clone(&factory_calls),
            transport: Arc::clone(&transport),
        });
        let playback = Arc::new(FakePlayback::new(blocked_playback));
        let actor = Arc::new(SpeechActor::new(
            Arc::new(CallScopedOnlyProvider),
            SpeechCache::new(8, 1_024),
            Arc::clone(&playback) as Arc<dyn SpeechPlayback>,
            Arc::new(SystemClock),
            32,
        ));
        let adapter = RepositoryProgramSpeech::new(
            repository,
            Box::new(FakeVault {
                secret: secret.map(str::to_owned),
                gets: Arc::clone(&vault_gets),
            }),
            actor,
            factory,
        );
        (
            Arc::new(adapter),
            vault_gets,
            factory_calls,
            transport,
            playback,
        )
    }

    #[tokio::test]
    async fn repository_program_speech_disabled_or_missing_secret_has_zero_transport_and_audio() {
        let (_temp, storage, repository) = fixture().await;
        let (disabled, disabled_gets, disabled_factory, disabled_transport, disabled_playback) =
            source(repository.clone(), Some(SECRET_CANARY), false);
        let (_sender, receiver) = watch::channel(false);
        let outcome = disabled
            .present(
                Uuid::now_v7(),
                Uuid::now_v7(),
                "fixture opening",
                ConfirmedProgramStart::Manual,
                receiver,
            )
            .await
            .expect("disabled fallback");
        assert_eq!(outcome, ProgramSpeechOutcome::TextOnly);
        assert_eq!(disabled_gets.load(Ordering::SeqCst), 0);
        assert_eq!(disabled_factory.load(Ordering::SeqCst), 0);
        assert_eq!(disabled_transport.calls.load(Ordering::SeqCst), 0);
        assert_eq!(disabled_playback.plays.load(Ordering::SeqCst), 0);

        enable_tts(&repository).await;
        let (missing, missing_gets, missing_factory, missing_transport, missing_playback) =
            source(repository, None, false);
        let (_sender, receiver) = watch::channel(false);
        let outcome = missing
            .present(
                Uuid::now_v7(),
                Uuid::now_v7(),
                "fixture bridge",
                ConfirmedProgramStart::Manual,
                receiver,
            )
            .await
            .expect("missing-secret fallback");
        assert_eq!(outcome, ProgramSpeechOutcome::TextOnly);
        assert_eq!(missing_gets.load(Ordering::SeqCst), 1);
        assert_eq!(missing_factory.load(Ordering::SeqCst), 0);
        assert_eq!(missing_transport.calls.load(Ordering::SeqCst), 0);
        assert_eq!(missing_playback.plays.load(Ordering::SeqCst), 0);
        storage.close().await;
    }

    #[tokio::test]
    async fn repository_program_speech_enabled_uses_one_call_and_returns_no_secret_or_path() {
        let (temp, storage, repository) = fixture().await;
        enable_tts(&repository).await;
        let (source, vault_gets, factory_calls, transport, playback) =
            source(repository, Some(SECRET_CANARY), false);
        let (_sender, receiver) = watch::channel(false);
        let outcome = source
            .present(
                Uuid::now_v7(),
                Uuid::now_v7(),
                "fixture bridge",
                ConfirmedProgramStart::Manual,
                receiver,
            )
            .await
            .expect("spoken outcome");
        assert_eq!(outcome, ProgramSpeechOutcome::Spoken);
        assert_eq!(vault_gets.load(Ordering::SeqCst), 1);
        assert_eq!(factory_calls.load(Ordering::SeqCst), 1);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
        assert!(transport.secret_matched.load(Ordering::SeqCst));
        assert_eq!(playback.plays.load(Ordering::SeqCst), 1);
        assert_eq!(playback.stops.load(Ordering::SeqCst), 0);
        let response_debug = format!("{outcome:?}");
        assert!(!response_debug.contains(SECRET_CANARY));
        assert!(!response_debug.contains(temp.path().to_string_lossy().as_ref()));
        storage.close().await;
    }

    #[tokio::test]
    async fn repository_program_speech_cancellation_stops_exact_segment_without_late_output() {
        let (_temp, storage, repository) = fixture().await;
        enable_tts(&repository).await;
        let (source, _vault_gets, _factory_calls, transport, playback) =
            source(repository, Some(SECRET_CANARY), true);
        let (sender, receiver) = watch::channel(false);
        let segment_id = Uuid::now_v7();
        let program_id = Uuid::now_v7();
        let task = tokio::spawn(async move {
            source
                .present(
                    program_id,
                    segment_id,
                    "fixture cancellable bridge",
                    ConfirmedProgramStart::Manual,
                    receiver,
                )
                .await
        });
        playback.started.notified().await;
        sender
            .send(true)
            .expect("cancellation receiver remains live");
        let error = task
            .await
            .expect("task join")
            .expect_err("cancelled operation");
        assert_eq!(error.error_id, ErrorId::OperationCancelled);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
        assert_eq!(playback.plays.load(Ordering::SeqCst), 1);
        assert_eq!(playback.stops.load(Ordering::SeqCst), 1);
        assert_eq!(playback.completions.load(Ordering::SeqCst), 0);
        assert_eq!(
            playback
                .stopped
                .lock()
                .expect("stopped operations")
                .as_slice(),
            [segment_id]
        );
        storage.close().await;
    }
}
