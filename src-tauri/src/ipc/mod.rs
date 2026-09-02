//! Stable, side-effect-free primitives for the Tauri IPC boundary.
//!
//! This module deliberately does not register Tauri commands. The application
//! composition root owns registration after it has injected trusted services.

mod capabilities;
mod command;
mod error;
mod events;
mod idempotency;
mod revision;
mod timeout;

pub use capabilities::{
    AppCapabilities, AppFeatures, CapabilitiesService, EmptyRequest, Platform, ProviderKind,
    SourceCapabilities, SourceKind, SourceSummary,
};
pub use command::parse_command_request;
pub(crate) use error::IPC_SCHEMA_VERSION;
pub use error::{
    ApiError, ApiErrorDetails, CapabilityName, ErrorId, InternalReason, PublicField,
    RedactedDiagnostic,
};
pub use events::{EventEnvelope, ProcessSequence, SequenceObservation, SequenceTracker};
pub use idempotency::{IdempotencyStore, IdempotencyWindow, RequestHash, canonical_request_hash};
pub use revision::Revision;
pub use timeout::{CommandTimeout, Deadline};
