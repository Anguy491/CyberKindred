use super::error::{ApiError, InternalReason, PublicField};

/// Monotonic aggregate revision with stale-write protection.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct Revision(u64);

impl Revision {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn current(self) -> u64 {
        self.0
    }

    /// Rejects a stale client intent and includes only the safe current revision.
    ///
    /// # Errors
    ///
    /// Returns `ERR-1003` when `expected` is not current.
    pub fn ensure_expected(self, expected: u64) -> Result<(), ApiError> {
        if expected == self.0 {
            Ok(())
        } else {
            Err(ApiError::from_reason(InternalReason::RevisionConflict)
                .with_field(PublicField::ExpectedRevision)
                .with_current_revision(self.0))
        }
    }

    /// Advances the revision without wrapping.
    ///
    /// # Errors
    ///
    /// Returns the stable internal error when the counter is exhausted.
    pub fn advance(&mut self) -> Result<u64, ApiError> {
        self.0 = self.0.checked_add(1).ok_or_else(ApiError::unexpected)?;
        Ok(self.0)
    }
}
