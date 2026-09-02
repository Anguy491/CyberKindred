use sha2::{Digest, Sha256};
use std::fmt;
use thiserror::Error;
use url::{Host, Url};
use zeroize::Zeroizing;

const CREDENTIAL_PREFIX: &str = "CyberKindred/provider/";
const CREDENTIAL_USER: &str = "api-key";
const MAX_SECRET_BYTES: usize = 16 * 1024;

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
}

/// Secret store boundary used by provider code and fake-backed default tests.
pub trait SecretVault {
    /// Replaces the exact origin-scoped credential.
    ///
    /// # Errors
    ///
    /// Returns a stable credential error if the OS/fake store cannot write.
    fn set(&mut self, target: &CredentialTarget, value: SecretValue) -> Result<(), SecretError>;

    /// Retrieves the exact origin-scoped credential into a zeroizing wrapper.
    ///
    /// # Errors
    ///
    /// Returns a stable credential error if the OS/fake store cannot read.
    fn get(&self, target: &CredentialTarget) -> Result<Option<SecretValue>, SecretError>;

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
}

#[cfg(windows)]
mod platform {
    use super::{
        CREDENTIAL_PREFIX, CREDENTIAL_USER, CredentialTarget, SecretError, SecretValue, SecretVault,
    };
    use windows::{
        Security::Credentials::{PasswordCredential, PasswordVault},
        core::HSTRING,
    };

    const HRESULT_NOT_FOUND: u32 = 0x8007_0490;

    /// Windows Credential Manager adapter. Construction and all writes are explicit.
    pub struct WindowsCredentialVault {
        vault: PasswordVault,
    }

    impl WindowsCredentialVault {
        /// Opens the current user's Windows Credential Manager vault.
        ///
        /// # Errors
        ///
        /// Returns `Unavailable` if Windows cannot construct the vault.
        pub fn new() -> Result<Self, SecretError> {
            PasswordVault::new()
                .map(|vault| Self { vault })
                .map_err(|_| SecretError::Unavailable)
        }

        fn retrieve(
            &self,
            target: &CredentialTarget,
        ) -> Result<Option<PasswordCredential>, SecretError> {
            let resource = HSTRING::from(target.as_resource());
            let user = HSTRING::from(CREDENTIAL_USER);
            match self.vault.Retrieve(&resource, &user) {
                Ok(credential) => Ok(Some(credential)),
                Err(error) if error.code().0.cast_unsigned() == HRESULT_NOT_FOUND => Ok(None),
                Err(_) => Err(SecretError::OperationFailed),
            }
        }
    }

    impl SecretVault for WindowsCredentialVault {
        fn set(
            &mut self,
            target: &CredentialTarget,
            value: SecretValue,
        ) -> Result<(), SecretError> {
            let resource = HSTRING::from(target.as_resource());
            let user = HSTRING::from(CREDENTIAL_USER);
            let credential = value.with_exposed(|secret| {
                let password = HSTRING::from(secret);
                PasswordCredential::CreatePasswordCredential(&resource, &user, &password)
            });
            let credential = credential.map_err(|_| SecretError::OperationFailed)?;
            let previous = self.retrieve(target)?;
            if let Some(previous) = &previous {
                // Load the old value before removal so a failed replacement can
                // restore it instead of silently losing a valid credential.
                previous
                    .RetrievePassword()
                    .map_err(|_| SecretError::OperationFailed)?;
                self.vault
                    .Remove(previous)
                    .map_err(|_| SecretError::OperationFailed)?;
            }
            if self.vault.Add(&credential).is_ok() {
                return Ok(());
            }
            if let Some(previous) = previous {
                let _ = self.vault.Add(&previous);
            }
            Err(SecretError::OperationFailed)
        }

        fn get(&self, target: &CredentialTarget) -> Result<Option<SecretValue>, SecretError> {
            let Some(credential) = self.retrieve(target)? else {
                return Ok(None);
            };
            credential
                .RetrievePassword()
                .map_err(|_| SecretError::OperationFailed)?;
            let password = credential
                .Password()
                .map_err(|_| SecretError::OperationFailed)?;
            SecretValue::new(password.to_string()).map(Some)
        }

        fn delete(&mut self, target: &CredentialTarget) -> Result<(), SecretError> {
            let Some(credential) = self.retrieve(target)? else {
                return Ok(());
            };
            self.vault
                .Remove(&credential)
                .map_err(|_| SecretError::OperationFailed)
        }

        fn delete_cyberkindred_namespace(&mut self) -> Result<u32, SecretError> {
            let credentials = self
                .vault
                .RetrieveAll()
                .map_err(|_| SecretError::OperationFailed)?;
            let size = credentials
                .Size()
                .map_err(|_| SecretError::OperationFailed)?;
            let mut matching = Vec::new();
            for index in 0..size {
                let credential = credentials
                    .GetAt(index)
                    .map_err(|_| SecretError::OperationFailed)?;
                let resource = credential
                    .Resource()
                    .map_err(|_| SecretError::OperationFailed)?;
                if resource.to_string().starts_with(CREDENTIAL_PREFIX) {
                    matching.push(credential);
                }
            }
            let count = u32::try_from(matching.len()).map_err(|_| SecretError::OperationFailed)?;
            for credential in matching {
                self.vault
                    .Remove(&credential)
                    .map_err(|_| SecretError::OperationFailed)?;
            }
            Ok(count)
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
        let retrieved = vault
            .get(&target)
            .expect("fake read")
            .expect("stored value");
        assert!(retrieved.with_exposed(|value| value == canary));
        assert!(!format!("{}", SecretError::OperationFailed).contains(canary));
        vault.delete(&target).expect("fake delete");
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
        let deleted = vault.delete(&target).is_ok();
        assert!(exists && deleted);
        assert!(vault.get(&target).expect("verify cleanup").is_none());
    }
}
