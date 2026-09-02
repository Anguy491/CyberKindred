//! Rust-owned persistence, app-path, retention, and credential boundaries.
//!
//! The module deliberately exposes neither database handles nor absolute paths to
//! the WebView-facing IPC layer.

mod database;
mod error;
mod paths;
mod repository;
mod retention;
mod secret;

pub use database::{APPLICATION_ID, LATEST_SCHEMA_VERSION, Storage};
pub use error::{StorageError, StorageReason};
pub use paths::{AppPaths, CacheArea, resolve_read_only_library_path};
pub use repository::{ChatRole, NewChatMessage, Repository, RetentionResult};
pub use secret::{
    CanonicalOrigin, CredentialTarget, SecretError, SecretValue, SecretVault,
    WindowsCredentialVault,
};
