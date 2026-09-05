use super::{ProviderFailure, ProviderTestKind, VoiceView};
use crate::ipc::ApiError;
use crate::storage::{CanonicalOrigin, SecretValue};
use chrono::{DateTime, SecondsFormat, Utc};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use uuid::Uuid;

pub type ProviderFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppBehaviorSettings {
    pub minimize_to_tray: bool,
    pub launch_at_startup: bool,
}

/// Applies user-visible OS settings at the same commit boundary as SQLite settings.
pub trait AppSettingsEffect: Send + Sync {
    /// Applies `next`, or restores `previous` when called in the reverse direction.
    ///
    /// # Errors
    ///
    /// Returns a stable, redacted error when the Windows integration rejects the change.
    fn apply(
        &self,
        previous: AppBehaviorSettings,
        next: AppBehaviorSettings,
    ) -> Result<(), ApiError>;
}

#[derive(Clone, Default)]
pub struct CancellationFlag(Arc<AtomicBool>);

impl CancellationFlag {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// Common bounded context passed to every injected provider boundary.
pub struct ProviderCallContext {
    pub correlation_id: Uuid,
    pub locale: &'static str,
    pub deadline: Instant,
    pub cancellation: CancellationFlag,
}

impl ProviderCallContext {
    #[must_use]
    pub fn new(timeout: Duration) -> Self {
        Self {
            correlation_id: Uuid::now_v7(),
            locale: "zh-CN",
            deadline: Instant::now() + timeout,
            cancellation: CancellationFlag::default(),
        }
    }
}

pub struct SecretValidationInput<'a> {
    pub origin: &'a CanonicalOrigin,
    pub candidate: &'a SecretValue,
}

pub trait CandidateSecretValidator: Send + Sync {
    fn validate<'a>(
        &'a self,
        input: SecretValidationInput<'a>,
        context: &'a ProviderCallContext,
    ) -> ProviderFuture<'a, Result<(), ProviderFailure>>;
}

pub enum ProviderTestInput<'a> {
    OpenAi {
        kind: ProviderTestKind,
        origin: &'a CanonicalOrigin,
        secret: &'a SecretValue,
        model_id: &'a str,
    },
    Metadata,
    Weather,
}

pub trait ProviderHealthProbe: Send + Sync {
    fn test<'a>(
        &'a self,
        input: ProviderTestInput<'a>,
        context: &'a ProviderCallContext,
    ) -> ProviderFuture<'a, Result<u64, ProviderFailure>>;
}

pub struct VoicePreviewInput<'a> {
    pub operation_id: Uuid,
    pub origin: &'a CanonicalOrigin,
    pub secret: &'a SecretValue,
    pub model_id: &'a str,
    pub voice_id: &'a str,
    pub text: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VoicePreviewCancelDisposition {
    Cancelled,
    AlreadyTerminal,
    NotFound,
}

pub trait VoicePreviewer: Send + Sync {
    fn voices(&self) -> Vec<VoiceView>;

    fn preview<'a>(
        &'a self,
        input: VoicePreviewInput<'a>,
        context: &'a ProviderCallContext,
    ) -> ProviderFuture<'a, Result<(), ProviderFailure>>;

    /// Idempotently cancels and stops output owned by this exact operation.
    fn cancel(&self, _operation_id: Uuid) -> VoicePreviewCancelDisposition {
        VoicePreviewCancelDisposition::NotFound
    }

    fn is_cancelled(&self, _operation_id: Uuid) -> bool {
        false
    }
}

/// Terminal result of an accepted voice preview operation. The command layer
/// maps `error: None` to EVT-008 and `Some(ApiError)` to EVT-009.
#[derive(Clone)]
pub struct VoicePreviewTerminal {
    pub operation_id: Uuid,
    pub occurred_at: String,
    pub error: Option<ApiError>,
    pub cancelled: bool,
}

/// Required observer for one authoritative terminal outcome per accepted preview.
pub trait VoicePreviewEventSink: Send + Sync {
    /// Publishes a terminal outcome. Transport delivery may repeat at least
    /// once; consumers deduplicate by operation ID against the authoritative
    /// stored outcome.
    ///
    /// # Errors
    ///
    /// Returns a stable, redacted failure so the service can expose a degraded
    /// authoritative snapshot instead of silently losing the terminal signal.
    fn publish(&self, terminal: VoicePreviewTerminal) -> Result<(), ApiError>;
}

pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;

    fn now_rfc3339(&self) -> String {
        self.now().to_rfc3339_opts(SecondsFormat::Millis, true)
    }

    fn now_ms(&self) -> i64 {
        self.now().timestamp_millis()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}
