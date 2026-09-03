//! Rust-owned persistence, app-path, retention, and credential boundaries.
//!
//! The module deliberately exposes neither database handles nor absolute paths to
//! the WebView-facing IPC layer.

mod database;
mod error;
mod library_roots;
mod onboarding;
mod operation_outbox;
mod paths;
mod playback_tracks;
mod program_candidates;
mod program_runs;
mod provider_settings;
mod provider_status;
mod repository;
mod retention;
mod scanner;
mod secret;
pub(crate) mod track_catalog;
mod understanding;
mod weather;

pub use database::{APPLICATION_ID, LATEST_SCHEMA_VERSION, Storage};
pub use error::{StorageError, StorageReason};
pub(crate) use library_roots::{StoredLibraryRoot, StoredLibraryRootAddition, StoredLibraryRoots};
pub(crate) use onboarding::{
    OnboardingStepWrite, OnboardingWriteError, StoredOnboardingDocument, StoredOnboardingProfile,
    StoredOnboardingSnapshot,
};
pub use operation_outbox::OperationTerminalRecord;
pub use paths::{AppPaths, CacheArea, resolve_read_only_library_path};
pub(crate) use playback_tracks::StoredPlaybackTrack;
pub(crate) use program_runs::{StoredProgramSegmentStatus, StoredProgramStatus};
pub(crate) use provider_settings::{StoredProviderSettings, StoredWeatherLocation};
pub(crate) use provider_status::ProviderStatusPromotion;
pub use provider_status::{
    NewProviderOutcome, ProviderOutcomeStatus, ProviderRequestKind, ProviderStatusSnapshot,
    ProviderUsageProvider,
};
pub use repository::{ChatRole, NewChatMessage, Repository, RetentionResult};
pub(crate) use scanner::{
    AuthorizedScanOperation, AuthorizedScanRoot, ScanCancelOutcome, ScanCounters,
    ScanOperationState, ScanProgress, ScanTerminalRecord, ScanTerminalState, ScanTrackAvailability,
    ScanTrackRecord,
};
pub use secret::{
    CanonicalOrigin, CredentialTarget, SecretError, SecretValue, SecretVault,
    WindowsCredentialVault,
};
pub(crate) use understanding::{
    ChatContextSnapshot, NewMemoryProposal, StoredMemory, StoredMemoryKind, StoredMemoryStatus,
};
#[cfg(test)]
pub(crate) use understanding::{ContextMemory, ContextTurn};
pub(crate) use weather::StoredWeatherCache;
