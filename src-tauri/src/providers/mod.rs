//! Provider configuration and explicit user-triggered provider operations.
//!
//! Concrete network adapters are intentionally outside this M2 foundation. The
//! service accepts injected validators/probes/previewers so default tests remain
//! hermetic and no construction path performs network, credential, or audio work.

pub mod commands;
mod dto;
mod error;
pub mod events;
mod registry;
mod runtime;
mod service;
mod traits;

pub use dto::{
    Ack, AudioOutputBehavior, DeleteSecretRequest, DeleteSecretResponse, Integration,
    IntegrationState, IntegrationStatus, ListVoicesRequest, NarrationDensity, OperationAccepted,
    OriginSecretStatus, PreviewVoiceRequest, ProviderTestKind, SecretKind, SecretStatus,
    SettingsPatch, SettingsView, TestProviderRequest, TestProviderResponse, UpdateSettingsRequest,
    ValidateSecretRequest, ValidateSecretResponse, VoiceProvider, VoiceView, VoicesResponse,
    WeatherLocation, WeatherLocationAction,
};
pub use error::{ProviderFailure, ProviderFailureCategory};
pub use runtime::ProviderRuntime;
pub use service::{PREVIEW_PHRASE_V1, ProviderService};
pub use traits::{
    CancellationFlag, CandidateSecretValidator, Clock, ProviderCallContext, ProviderHealthProbe,
    ProviderTestInput, SecretValidationInput, SystemClock, VoicePreviewEventSink,
    VoicePreviewInput, VoicePreviewTerminal, VoicePreviewer,
};
