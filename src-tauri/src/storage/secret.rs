use serde::{Deserialize, Deserializer};
use sha2::{Digest, Sha256};
use std::fmt;
use thiserror::Error;
use url::{Host, Url};
use zeroize::Zeroizing;

const CREDENTIAL_PREFIX: &str = "CyberKindred/provider/";
const CREDENTIAL_USER: &str = "api-key";
const MAX_SECRET_BYTES: usize = cyberkindred_windows_credential::MAX_SECRET_BYTES;

/// A canonical HTTPS provider origin without a path, query, fragment, or IP host.
#[derive(Clone, Eq, PartialEq)]
pub struct CanonicalOrigin(String);

impl CanonicalOrigin {
    /// Parses an origin accepted by the provider/credential boundary.
    ///
    /// # Errors
    ///
    /// Returns `InvalidOrigin` unless the input is a path-free HTTPS domain
    /// origin without credentials, query, fragment, or IP literal.
    pub fn parse(value: &str) -> Result<Self, SecretError> {
        let parsed = Url::parse(value).map_err(|_| SecretError::InvalidOrigin)?;
        if parsed.scheme() != "https"
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || parsed.path() != "/"
        {
            return Err(SecretError::InvalidOrigin);
        }
        let Host::Domain(host) = parsed.host().ok_or(SecretError::InvalidOrigin)? else {
            return Err(SecretError::InvalidOrigin);
        };
        if host.is_empty() {
            return Err(SecretError::InvalidOrigin);
        }
        let port = parsed
            .port()
            .map_or_else(String::new, |value| format!(":{value}"));
        Ok(Self(format!("https://{host}{port}")))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for CanonicalOrigin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("CanonicalOrigin")
            .field(&self.0)
            .finish()
    }
}

/// The non-secret Credential Manager resource name for one canonical origin.
#[derive(Clone, Eq, PartialEq)]
pub struct CredentialTarget(String);

impl CredentialTarget {
    #[must_use]
    pub fn openai(origin: &CanonicalOrigin) -> Self {
        let digest = Sha256::digest(origin.as_str().as_bytes());
        Self(format!(
            "{CREDENTIAL_PREFIX}openai/{}/api-key",
            hex::encode(digest)
        ))
    }

    #[must_use]
    pub fn as_resource(&self) -> &str {
        &self.0
    }
}

/// Secret material whose allocation is zeroized on drop.
///
/// This type intentionally implements neither `Debug`, `Clone`, nor `Serialize`.
pub struct SecretValue(Zeroizing<String>);

impl SecretValue {
    /// Wraps non-empty, bounded secret material for zeroization on drop.
    ///
    /// # Errors
    ///
    /// Returns `InvalidValue` for empty or oversized values.
    pub fn new(value: String) -> Result<Self, SecretError> {
        if value.is_empty() || value.len() > MAX_SECRET_BYTES {
            return Err(SecretError::InvalidValue);
        }
        Ok(Self(Zeroizing::new(value)))
    }

    pub fn with_exposed<T>(&self, operation: impl FnOnce(&str) -> T) -> T {
        operation(self.0.as_str())
    }
}

impl<'de> Deserialize<'de> for SecretValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Stable credential boundary errors that never contain a value or OS message.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum SecretError {
    #[error("provider origin is invalid")]
    InvalidOrigin,
    #[error("secret value is invalid")]
    InvalidValue,
    #[error("credential manager is unavailable")]
    Unavailable,
    #[error("credential operation failed")]
    OperationFailed,
    #[error("credential replacement failed; previous value restored")]
    ReplacementFailedRestored,
    #[error("credential replacement and restoration failed; state unknown")]
    CredentialStateUnknown,
}

/// Secret store boundary used by provider code and fake-backed default tests.
pub trait SecretVault: Send {
    /// Replaces the exact origin-scoped credential.
    ///
    /// # Errors
    ///
    /// Returns a stable credential error if the OS/fake store cannot write.
    /// `ReplacementFailedRestored` proves the prior value was restored;
    /// `CredentialStateUnknown` explicitly means restoration also failed.
    fn set(&mut self, target: &CredentialTarget, value: SecretValue) -> Result<(), SecretError>;

    /// Retrieves the exact origin-scoped credential into a zeroizing wrapper.
    ///
    /// # Errors
    ///
    /// Returns a stable credential error if the OS/fake store cannot read.
    fn get(&self, target: &CredentialTarget) -> Result<Option<SecretValue>, SecretError>;

    /// Checks the exact origin-scoped credential without returning its value.
    ///
    /// # Errors
    ///
    /// Returns a stable credential error if the OS/fake store cannot query it.
    fn contains(&self, target: &CredentialTarget) -> Result<bool, SecretError> {
        self.get(target).map(|value| value.is_some())
    }

    /// Deletes the exact origin-scoped credential and is idempotent if absent.
    ///
    /// # Errors
    ///
    /// Returns a stable credential error if the OS/fake store cannot delete.
    fn delete(&mut self, target: &CredentialTarget) -> Result<(), SecretError>;

    /// Deletes every credential under the `CyberKindred/provider/` prefix.
    ///
    /// # Errors
    ///
    /// Returns a stable credential error if enumeration or any deletion fails.
    fn delete_cyberkindred_namespace(&mut self) -> Result<u32, SecretError>;

    /// Counts every credential under the `CyberKindred` namespace without
    /// returning identifiers or secret material.
    ///
    /// # Errors
    ///
    /// Returns a stable credential error if enumeration is unavailable.
    fn count_cyberkindred_namespace(&self) -> Result<u32, SecretError> {
        Err(SecretError::Unavailable)
    }
}

#[cfg(windows)]
mod platform {
    use super::{
        CREDENTIAL_PREFIX, CREDENTIAL_USER, CredentialTarget, SecretError, SecretValue, SecretVault,
    };
    use cyberkindred_windows_credential::{CredentialError, CredentialSecret, CredentialStore};

    /// Windows Credential Manager adapter. Construction and all writes are explicit.
    pub struct WindowsCredentialVault {
        store: CredentialStore,
    }

    impl WindowsCredentialVault {
        /// Opens the current user's Windows Credential Manager vault.
        ///
        /// # Errors
        ///
        /// Returns `Unavailable` if Windows cannot construct the vault.
        pub fn new() -> Result<Self, SecretError> {
            Ok(Self {
                store: CredentialStore::new(),
            })
        }
    }

    impl SecretVault for WindowsCredentialVault {
        fn set(
            &mut self,
            target: &CredentialTarget,
            value: SecretValue,
        ) -> Result<(), SecretError> {
            let mut credential = value
                .with_exposed(|secret| CredentialSecret::new(secret.as_bytes()))
                .map_err(map_credential_error)?;
            self.store
                .write(target.as_resource(), CREDENTIAL_USER, &mut credential)
                .map_err(map_credential_error)
        }

        fn get(&self, target: &CredentialTarget) -> Result<Option<SecretValue>, SecretError> {
            let Some(credential) = self
                .store
                .read(target.as_resource())
                .map_err(map_credential_error)?
            else {
                return Ok(None);
            };
            credential
                .with_exposed(|bytes| {
                    std::str::from_utf8(bytes)
                        .map_err(|_| SecretError::OperationFailed)
                        .and_then(|value| SecretValue::new(value.to_owned()))
                })
                .map(Some)
        }

        fn contains(&self, target: &CredentialTarget) -> Result<bool, SecretError> {
            self.store
                .contains(target.as_resource())
                .map_err(map_credential_error)
        }

        fn delete(&mut self, target: &CredentialTarget) -> Result<(), SecretError> {
            self.store
                .delete(target.as_resource())
                .map_err(map_credential_error)
        }

        fn delete_cyberkindred_namespace(&mut self) -> Result<u32, SecretError> {
            self.store
                .delete_prefix(CREDENTIAL_PREFIX)
                .map_err(map_credential_error)
        }

        fn count_cyberkindred_namespace(&self) -> Result<u32, SecretError> {
            self.store
                .count_prefix(CREDENTIAL_PREFIX)
                .map_err(map_credential_error)
        }
    }

    const fn map_credential_error(error: CredentialError) -> SecretError {
        match error {
            CredentialError::InvalidInput => SecretError::InvalidValue,
            CredentialError::Unavailable => SecretError::Unavailable,
            CredentialError::OperationFailed | CredentialError::InvalidStoredValue => {
                SecretError::OperationFailed
            }
        }
    }
}

#[cfg(windows)]
pub use platform::WindowsCredentialVault;

#[cfg(not(windows))]
pub struct WindowsCredentialVault;

#[cfg(not(windows))]
impl WindowsCredentialVault {
    /// Reports that the Windows-only credential store is unavailable.
    ///
    /// # Errors
    ///
    /// Always returns `Unavailable` on non-Windows targets.
    pub fn new() -> Result<Self, SecretError> {
        Err(SecretError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct FakeVault {
        values: HashMap<String, SecretValue>,
    }

    impl SecretVault for FakeVault {
        fn set(
            &mut self,
            target: &CredentialTarget,
            value: SecretValue,
        ) -> Result<(), SecretError> {
            self.values.insert(target.as_resource().to_owned(), value);
            Ok(())
        }

        fn get(&self, target: &CredentialTarget) -> Result<Option<SecretValue>, SecretError> {
            self.values
                .get(target.as_resource())
                .map(|value| value.with_exposed(|secret| SecretValue::new(secret.to_owned())))
                .transpose()
        }

        fn contains(&self, target: &CredentialTarget) -> Result<bool, SecretError> {
            Ok(self.values.contains_key(target.as_resource()))
        }

        fn delete(&mut self, target: &CredentialTarget) -> Result<(), SecretError> {
            self.values.remove(target.as_resource());
            Ok(())
        }

        fn delete_cyberkindred_namespace(&mut self) -> Result<u32, SecretError> {
            let count =
                u32::try_from(self.values.len()).map_err(|_| SecretError::OperationFailed)?;
            self.values.clear();
            Ok(count)
        }

        fn count_cyberkindred_namespace(&self) -> Result<u32, SecretError> {
            u32::try_from(self.values.len()).map_err(|_| SecretError::OperationFailed)
        }
    }

    #[test]
    fn secret_origin_is_canonical_and_namespaced_by_sha256() {
        let first = CanonicalOrigin::parse("https://API.OpenAI.com:443").expect("valid origin");
        let second = CanonicalOrigin::parse("https://api.openai.com").expect("valid origin");
        assert_eq!(first, second);
        let target = CredentialTarget::openai(&first);
        assert!(
            target
                .as_resource()
                .starts_with("CyberKindred/provider/openai/")
        );
        assert!(target.as_resource().ends_with("/api-key"));
        let digest = target
            .as_resource()
            .strip_prefix("CyberKindred/provider/openai/")
            .and_then(|value| value.strip_suffix("/api-key"))
            .expect("target shape");
        assert_eq!(digest.len(), 64);
        assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    #[test]
    fn secret_origin_rejects_unsafe_destinations() {
        for invalid in [
            "http://api.openai.com",
            "https://127.0.0.1",
            "https://[::1]",
            "https://user@example.com",
            "https://example.com/path",
            "https://example.com?key=value",
        ] {
            assert_eq!(
                CanonicalOrigin::parse(invalid),
                Err(SecretError::InvalidOrigin)
            );
        }
    }

    #[test]
    fn secret_fake_round_trip_returns_no_value_in_status_or_error() {
        let origin = CanonicalOrigin::parse("https://example.com").expect("valid origin");
        let target = CredentialTarget::openai(&origin);
        let mut vault = FakeVault::default();
        let canary = "ck-secret-canary-never-persist";
        vault
            .set(
                &target,
                SecretValue::new(canary.to_owned()).expect("valid secret"),
            )
            .expect("fake write");
        assert!(vault.contains(&target).expect("fake contains"));
        let retrieved = vault
            .get(&target)
            .expect("fake read")
            .expect("stored value");
        assert!(retrieved.with_exposed(|value| value == canary));
        assert!(!format!("{}", SecretError::OperationFailed).contains(canary));
        vault.delete(&target).expect("fake delete");
        assert!(!vault.contains(&target).expect("fake contains"));
        assert!(vault.get(&target).expect("fake read").is_none());
    }

    #[test]
    fn secret_namespace_reset_removes_every_origin() {
        let mut vault = FakeVault::default();
        for origin in ["https://one.example", "https://two.example"] {
            let target = CredentialTarget::openai(
                &CanonicalOrigin::parse(origin).expect("valid fixture origin"),
            );
            vault
                .set(
                    &target,
                    SecretValue::new("fixture-key".to_owned()).expect("valid secret"),
                )
                .expect("fake write");
        }
        assert_eq!(
            vault
                .delete_cyberkindred_namespace()
                .expect("fake namespace reset"),
            2
        );
        assert!(vault.values.is_empty());
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "explicit live canary: writes then removes one test Credential Manager item"]
    fn secret_windows_live_canary_is_explicit_and_self_cleaning() {
        let origin =
            CanonicalOrigin::parse("https://task-007-canary.invalid").expect("valid canary origin");
        let target = CredentialTarget::openai(&origin);
        let mut vault = WindowsCredentialVault::new().expect("Credential Manager available");
        let _ = vault.delete(&target);
        vault
            .set(
                &target,
                SecretValue::new("ck-task007-live-canary".to_owned()).expect("valid secret"),
            )
            .expect("write canary");
        let exists = vault.get(&target).expect("read canary").is_some();
        assert!(vault.contains(&target).expect("contains canary"));
        vault
            .set(
                &target,
                SecretValue::new("ck-task007-live-replacement".to_owned())
                    .expect("valid replacement"),
            )
            .expect("atomic replacement");
        let replaced = vault
            .get(&target)
            .expect("read replacement")
            .is_some_and(|value| {
                value.with_exposed(|secret| secret == "ck-task007-live-replacement")
            });
        let deleted = vault.delete(&target).is_ok();
        assert!(exists && replaced && deleted);
        assert!(!vault.contains(&target).expect("contains after cleanup"));
        assert!(vault.get(&target).expect("verify cleanup").is_none());
    }
}
