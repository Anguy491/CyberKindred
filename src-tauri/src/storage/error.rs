use std::fmt;
use thiserror::Error;

/// Stable, non-sensitive storage failure reasons.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum StorageReason {
    PathDenied,
    PathOutsideScope,
    UnsafeReparsePoint,
    StorageReadFailed,
    StorageWriteFailed,
    StorageIntegrityFailed,
    MigrationFailed,
    DatabaseVersionUnsupported,
    ForeignDatabase,
    InvalidSetting,
    EntityNotFound,
    RevisionConflict,
    ResourceBusy,
}

impl StorageReason {
    /// Returns the API contract's stable internal reason string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PathDenied => "path_denied",
            Self::PathOutsideScope => "path_outside_root",
            Self::UnsafeReparsePoint => "unsafe_reparse_point",
            Self::StorageReadFailed => "storage_read_failed",
            Self::StorageWriteFailed | Self::InvalidSetting => "storage_write_failed",
            Self::StorageIntegrityFailed | Self::ForeignDatabase => "storage_integrity_failed",
            Self::MigrationFailed => "migration_failed",
            Self::DatabaseVersionUnsupported => "database_version_unsupported",
            Self::EntityNotFound => "entity_not_found",
            Self::RevisionConflict => "revision_conflict",
            Self::ResourceBusy => "resource_busy",
        }
    }
}

impl fmt::Display for StorageReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// An error safe to map to an `ApiError` without including SQL or a path.
#[derive(Clone, Debug, Error)]
#[error("本地数据操作失败（{reason}）")]
pub struct StorageError {
    reason: StorageReason,
}

impl StorageError {
    #[must_use]
    pub(crate) const fn new(reason: StorageReason) -> Self {
        Self { reason }
    }

    #[must_use]
    pub const fn reason(&self) -> StorageReason {
        self.reason
    }
}
