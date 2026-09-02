use crate::ipc::{ApiError, InternalReason};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderFailureCategory {
    Authentication,
    RateLimit,
    Timeout,
    Unavailable,
    InvalidResponse,
}

/// Provider failure stripped of upstream bodies, headers, URLs, and credentials.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderFailure {
    pub category: ProviderFailureCategory,
    pub retry_after_ms: Option<u64>,
}

impl ProviderFailure {
    #[must_use]
    pub const fn new(category: ProviderFailureCategory) -> Self {
        Self {
            category,
            retry_after_ms: None,
        }
    }

    #[must_use]
    pub const fn rate_limited(retry_after_ms: Option<u64>) -> Self {
        Self {
            category: ProviderFailureCategory::RateLimit,
            retry_after_ms,
        }
    }

    #[must_use]
    pub fn into_api_error(self, candidate_secret_validation: bool) -> ApiError {
        let reason = match self.category {
            ProviderFailureCategory::Authentication if candidate_secret_validation => {
                InternalReason::SecretValidationRejected
            }
            ProviderFailureCategory::Authentication => InternalReason::ProviderAuthentication,
            ProviderFailureCategory::RateLimit => InternalReason::ProviderRateLimit,
            ProviderFailureCategory::Timeout => InternalReason::ProviderTimeout,
            ProviderFailureCategory::Unavailable => InternalReason::ProviderUnavailable,
            ProviderFailureCategory::InvalidResponse => InternalReason::ProviderInvalidResponse,
        };
        let error = ApiError::from_reason(reason);
        match self.retry_after_ms {
            Some(retry_after_ms) => error.with_retry_after_ms(retry_after_ms),
            None => error,
        }
    }
}
