//! Narrow safe wrapper around the Win32 Credential Manager generic-credential API.
//!
//! This is the only `CyberKindred` crate allowed to contain `unsafe` code. Its
//! public surface owns or borrows every pointer backing an FFI call, returns no
//! Windows error text, and zeroizes both copied secrets and readable OS buffers.

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(clippy::all, clippy::pedantic)]

use zeroize::Zeroizing;

/// Win32's documented maximum generic credential blob size.
pub const MAX_SECRET_BYTES: usize = 2_560;
const MAX_GENERIC_TARGET_UTF16_UNITS: usize = 32_767;

/// Stable, redacted failures from the credential infrastructure boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialError {
    /// A caller supplied an empty, oversized, or NUL-containing value.
    InvalidInput,
    /// The current logon session has no available credential set.
    Unavailable,
    /// Credential Manager rejected an operation.
    OperationFailed,
    /// Credential Manager returned malformed or unexpected stored data.
    InvalidStoredValue,
}

/// Secret bytes copied from or prepared for Credential Manager.
///
/// The owned allocation is zeroized on drop and deliberately implements
/// neither `Clone`, `Debug`, nor serialization traits.
pub struct CredentialSecret(Zeroizing<Vec<u8>>);

impl CredentialSecret {
    /// Copies a non-empty secret into zeroizing storage.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialError::InvalidInput`] when the Win32 blob limit is
    /// exceeded or the input is empty.
    pub fn new(value: &[u8]) -> Result<Self, CredentialError> {
        if value.is_empty() || value.len() > MAX_SECRET_BYTES {
            return Err(CredentialError::InvalidInput);
        }
        Ok(Self(Zeroizing::new(value.to_vec())))
    }

    /// Temporarily exposes the secret to one non-escaping operation.
    pub fn with_exposed<T>(&self, operation: impl FnOnce(&[u8]) -> T) -> T {
        operation(self.0.as_slice())
    }

    fn with_exposed_mut<T>(&mut self, operation: impl FnOnce(&mut [u8]) -> T) -> T {
        operation(self.0.as_mut_slice())
    }
}

/// Stateless handle to the current user's Windows Credential Manager.
#[derive(Clone, Copy, Default)]
pub struct CredentialStore;

impl CredentialStore {
    /// Constructs a handle without reading or writing Credential Manager.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Atomically creates or replaces one generic credential identified by
    /// the same target name and credential type.
    ///
    /// # Errors
    ///
    /// Returns a stable redacted error for invalid input or an OS failure.
    pub fn write(
        &self,
        target: &str,
        user_name: &str,
        secret: &mut CredentialSecret,
    ) -> Result<(), CredentialError> {
        validate_name(target)?;
        validate_name(user_name)?;
        secret.with_exposed_mut(|bytes| platform::write(target, user_name, bytes))
    }

    /// Reads one generic credential into a zeroizing allocation.
    ///
    /// # Errors
    ///
    /// Returns a stable redacted error for invalid input, malformed stored
    /// data, or an OS failure. A missing target returns `Ok(None)`.
    pub fn read(&self, target: &str) -> Result<Option<CredentialSecret>, CredentialError> {
        validate_name(target)?;
        platform::read(target)
    }

    /// Checks whether an exact generic target exists without copying its secret
    /// blob into a caller-visible value.
    ///
    /// # Errors
    ///
    /// Returns a stable redacted error for invalid input or an OS failure.
    pub fn contains(&self, target: &str) -> Result<bool, CredentialError> {
        validate_name(target)?;
        platform::contains(target)
    }

    /// Idempotently deletes one generic credential.
    ///
    /// # Errors
    ///
    /// Returns a stable redacted error for invalid input or an OS failure.
    pub fn delete(&self, target: &str) -> Result<(), CredentialError> {
        validate_name(target)?;
        platform::delete(target)
    }

    /// Deletes all generic credentials whose target starts with `prefix`.
    ///
    /// # Errors
    ///
    /// Returns a stable redacted error for invalid input or an OS failure.
    pub fn delete_prefix(&self, prefix: &str) -> Result<u32, CredentialError> {
        validate_prefix(prefix)?;
        platform::delete_prefix(prefix)
    }

    /// Counts generic credentials whose target starts with `prefix` without
    /// returning target names or secret material.
    ///
    /// # Errors
    ///
    /// Returns a stable redacted error for invalid input or an OS failure.
    pub fn count_prefix(&self, prefix: &str) -> Result<u32, CredentialError> {
        validate_prefix(prefix)?;
        platform::count_prefix(prefix)
    }
}

fn validate_name(value: &str) -> Result<(), CredentialError> {
    let utf16_units = value.encode_utf16().count();
    if value.is_empty()
        || value.contains('*')
        || value.as_bytes().contains(&0)
        || utf16_units > MAX_GENERIC_TARGET_UTF16_UNITS
    {
        Err(CredentialError::InvalidInput)
    } else {
        Ok(())
    }
}

fn validate_prefix(value: &str) -> Result<(), CredentialError> {
    validate_name(value)?;
    if value.encode_utf16().count() == MAX_GENERIC_TARGET_UTF16_UNITS {
        Err(CredentialError::InvalidInput)
    } else {
        Ok(())
    }
}

#[cfg(windows)]
mod platform {
    use super::{CredentialError, CredentialSecret, MAX_SECRET_BYTES};
    use std::{ffi::c_void, ptr::NonNull};
    use windows::{
        Win32::{
            Foundation::{ERROR_NO_SUCH_LOGON_SESSION, ERROR_NOT_FOUND, FILETIME},
            Security::Credentials::{
                CRED_FLAGS, CRED_MAX_CREDENTIAL_BLOB_SIZE, CRED_PERSIST_LOCAL_MACHINE,
                CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredEnumerateW, CredFree, CredReadW,
                CredWriteW,
            },
        },
        core::{Error, HRESULT, PCWSTR, PWSTR},
    };
    use zeroize::Zeroize;

    pub(super) fn write(
        target: &str,
        user_name: &str,
        secret: &mut [u8],
    ) -> Result<(), CredentialError> {
        debug_assert_eq!(MAX_SECRET_BYTES, CRED_MAX_CREDENTIAL_BLOB_SIZE as usize);
        let mut target = wide_string(target)?;
        let mut user_name = wide_string(user_name)?;
        let blob_size = u32::try_from(secret.len()).map_err(|_| CredentialError::InvalidInput)?;
        let credential = CREDENTIALW {
            Flags: CRED_FLAGS(0),
            Type: CRED_TYPE_GENERIC,
            TargetName: PWSTR::from_raw(target.as_mut_ptr()),
            Comment: PWSTR::null(),
            LastWritten: FILETIME::default(),
            CredentialBlobSize: blob_size,
            CredentialBlob: secret.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            AttributeCount: 0,
            Attributes: std::ptr::null_mut(),
            TargetAlias: PWSTR::null(),
            UserName: PWSTR::from_raw(user_name.as_mut_ptr()),
        };

        // SAFETY: every pointer in `credential` is null or points to a live,
        // correctly sized allocation retained until the synchronous call
        // returns. Win32 documents the structure as input-only for CredWriteW.
        unsafe { CredWriteW(&raw const credential, 0) }.map_err(|error| map_error(&error))
    }

    pub(super) fn read(target: &str) -> Result<Option<CredentialSecret>, CredentialError> {
        let target = wide_string(target)?;
        let mut credential = std::ptr::null_mut();
        // SAFETY: `target` is NUL-terminated and lives for the call; the output
        // pointer is initialized to null and is owned by CredFree on success.
        match unsafe {
            CredReadW(
                PCWSTR::from_raw(target.as_ptr()),
                CRED_TYPE_GENERIC,
                None,
                &raw mut credential,
            )
        } {
            Ok(()) => {
                let buffer = CredentialBuffer::new(credential)?;
                buffer.copy_secret().map(Some)
            }
            Err(error) if is_not_found(&error) => Ok(None),
            Err(error) => Err(map_error(&error)),
        }
    }

    pub(super) fn delete(target: &str) -> Result<(), CredentialError> {
        let target = wide_string(target)?;
        // SAFETY: `target` is a live NUL-terminated UTF-16 allocation for the
        // duration of the synchronous call; flags are reserved and set to zero.
        match unsafe { CredDeleteW(PCWSTR::from_raw(target.as_ptr()), CRED_TYPE_GENERIC, None) } {
            Ok(()) => Ok(()),
            Err(error) if is_not_found(&error) => Ok(()),
            Err(error) => Err(map_error(&error)),
        }
    }

    pub(super) fn contains(target: &str) -> Result<bool, CredentialError> {
        enumerate_generic_targets(target).map(|targets| targets.iter().any(|value| value == target))
    }

    pub(super) fn delete_prefix(prefix: &str) -> Result<u32, CredentialError> {
        let targets = enumerate_generic_targets(prefix)?;
        let deleted = u32::try_from(targets.len()).map_err(|_| CredentialError::OperationFailed)?;
        for target in targets {
            delete(&target)?;
        }
        Ok(deleted)
    }

    pub(super) fn count_prefix(prefix: &str) -> Result<u32, CredentialError> {
        u32::try_from(enumerate_generic_targets(prefix)?.len())
            .map_err(|_| CredentialError::OperationFailed)
    }

    fn enumerate_generic_targets(prefix: &str) -> Result<Vec<String>, CredentialError> {
        let filter_value = format!("{prefix}*");
        let filter = wide_string(&filter_value)?;
        let mut count = 0_u32;
        let mut credentials = std::ptr::null_mut();
        // SAFETY: `filter` is a live NUL-terminated prefix filter; output
        // pointers are initialized and the returned single block is wrapped in
        // `EnumerationBuffer`, which always calls CredFree.
        match unsafe {
            CredEnumerateW(
                PCWSTR::from_raw(filter.as_ptr()),
                None,
                &raw mut count,
                &raw mut credentials,
            )
        } {
            Ok(()) => {}
            Err(error) if is_not_found(&error) => return Ok(Vec::new()),
            Err(error) => return Err(map_error(&error)),
        }
        let buffer = EnumerationBuffer::new(credentials, count)?;
        buffer.generic_targets(prefix)
    }

    fn wide_string(value: &str) -> Result<Vec<u16>, CredentialError> {
        let mut wide: Vec<u16> = value.encode_utf16().collect();
        if wide.contains(&0) {
            return Err(CredentialError::InvalidInput);
        }
        wide.push(0);
        Ok(wide)
    }

    fn is_not_found(error: &Error) -> bool {
        error.code() == HRESULT::from_win32(ERROR_NOT_FOUND.0)
    }

    pub(super) fn map_error(error: &Error) -> CredentialError {
        if error.code() == HRESULT::from_win32(ERROR_NO_SUCH_LOGON_SESSION.0) {
            CredentialError::Unavailable
        } else {
            CredentialError::OperationFailed
        }
    }

    struct CredentialBuffer(NonNull<CREDENTIALW>);

    impl CredentialBuffer {
        fn new(pointer: *mut CREDENTIALW) -> Result<Self, CredentialError> {
            NonNull::new(pointer)
                .map(Self)
                .ok_or(CredentialError::InvalidStoredValue)
        }

        fn copy_secret(&self) -> Result<CredentialSecret, CredentialError> {
            // SAFETY: CredReadW returned one live CREDENTIALW block owned by
            // `self`; the documented blob size is bounded and validated before
            // constructing the byte slice.
            let credential = unsafe { self.0.as_ref() };
            let length = usize::try_from(credential.CredentialBlobSize)
                .map_err(|_| CredentialError::InvalidStoredValue)?;
            if length == 0 || length > MAX_SECRET_BYTES || credential.CredentialBlob.is_null() {
                return Err(CredentialError::InvalidStoredValue);
            }
            // SAFETY: the pointer and validated size describe the credential
            // blob within the live buffer returned by CredReadW.
            let bytes = unsafe { std::slice::from_raw_parts(credential.CredentialBlob, length) };
            CredentialSecret::new(bytes).map_err(|_| CredentialError::InvalidStoredValue)
        }
    }

    impl Drop for CredentialBuffer {
        fn drop(&mut self) {
            // SAFETY: this RAII guard uniquely owns the CredReadW allocation.
            // The blob is within that allocation under the Win32 contract; it
            // is wiped before the containing block is released exactly once.
            unsafe {
                let credential = self.0.as_mut();
                wipe_blob(credential);
                CredFree(self.0.as_ptr().cast::<c_void>());
            }
        }
    }

    struct EnumerationBuffer {
        pointer: NonNull<*mut CREDENTIALW>,
        count: usize,
    }

    impl EnumerationBuffer {
        fn new(pointer: *mut *mut CREDENTIALW, count: u32) -> Result<Self, CredentialError> {
            let count = usize::try_from(count).map_err(|_| CredentialError::InvalidStoredValue)?;
            if count == 0 {
                if !pointer.is_null() {
                    // SAFETY: a non-null pointer returned by a successful
                    // CredEnumerateW call is a single CredFree allocation even
                    // when the accompanying count is unexpectedly zero.
                    unsafe { CredFree(pointer.cast::<c_void>()) };
                }
                return Err(CredentialError::InvalidStoredValue);
            }
            let pointer = NonNull::new(pointer).ok_or(CredentialError::InvalidStoredValue)?;
            Ok(Self { pointer, count })
        }

        fn credentials(&self) -> &[*mut CREDENTIALW] {
            // SAFETY: CredEnumerateW returned an array containing `count`
            // credential pointers inside the one allocation owned by `self`.
            unsafe { std::slice::from_raw_parts(self.pointer.as_ptr(), self.count) }
        }

        fn generic_targets(&self, prefix: &str) -> Result<Vec<String>, CredentialError> {
            let mut targets = Vec::new();
            for pointer in self.credentials() {
                let pointer = NonNull::new(*pointer).ok_or(CredentialError::InvalidStoredValue)?;
                // SAFETY: each pointer and its NUL-terminated strings are
                // contained in the live block returned by CredEnumerateW.
                let credential = unsafe { pointer.as_ref() };
                let blob_size = usize::try_from(credential.CredentialBlobSize)
                    .map_err(|_| CredentialError::InvalidStoredValue)?;
                if credential.Type != CRED_TYPE_GENERIC
                    || blob_size > MAX_SECRET_BYTES
                    || credential.TargetName.is_null()
                {
                    continue;
                }
                // SAFETY: the OS-owned TargetName is NUL-terminated and remains
                // live until this enumeration guard is dropped.
                let target = unsafe { credential.TargetName.to_string() }
                    .map_err(|_| CredentialError::InvalidStoredValue)?;
                if target.starts_with(prefix) {
                    targets.push(target);
                }
            }
            Ok(targets)
        }
    }

    impl Drop for EnumerationBuffer {
        fn drop(&mut self) {
            // SAFETY: this guard uniquely owns the single CredEnumerateW block.
            // Every readable secret blob is wiped before the block is released.
            unsafe {
                for pointer in self.credentials() {
                    if let Some(mut pointer) = NonNull::new(*pointer) {
                        wipe_blob(pointer.as_mut());
                    }
                }
                CredFree(self.pointer.as_ptr().cast::<c_void>());
            }
        }
    }

    unsafe fn wipe_blob(credential: &mut CREDENTIALW) {
        let length = usize::try_from(credential.CredentialBlobSize)
            .map_or(MAX_SECRET_BYTES, |value| value.min(MAX_SECRET_BYTES));
        if length > 0 && !credential.CredentialBlob.is_null() {
            // SAFETY: callers hold the live OS allocation and the official API
            // contract bounds every credential blob to MAX_SECRET_BYTES.
            let bytes =
                unsafe { std::slice::from_raw_parts_mut(credential.CredentialBlob, length) };
            bytes.zeroize();
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{CredentialError, CredentialSecret};

    pub(super) fn write(
        _target: &str,
        _user_name: &str,
        _secret: &mut [u8],
    ) -> Result<(), CredentialError> {
        Err(CredentialError::Unavailable)
    }

    pub(super) fn read(_target: &str) -> Result<Option<CredentialSecret>, CredentialError> {
        Err(CredentialError::Unavailable)
    }

    pub(super) fn contains(_target: &str) -> Result<bool, CredentialError> {
        Err(CredentialError::Unavailable)
    }

    pub(super) fn delete(_target: &str) -> Result<(), CredentialError> {
        Err(CredentialError::Unavailable)
    }

    pub(super) fn delete_prefix(_prefix: &str) -> Result<u32, CredentialError> {
        Err(CredentialError::Unavailable)
    }

    pub(super) fn count_prefix(_prefix: &str) -> Result<u32, CredentialError> {
        Err(CredentialError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_secret_is_bounded_and_only_explicitly_exposed() {
        assert_eq!(
            CredentialSecret::new(&[]).err(),
            Some(CredentialError::InvalidInput)
        );
        assert_eq!(
            CredentialSecret::new(&vec![b'x'; MAX_SECRET_BYTES + 1]).err(),
            Some(CredentialError::InvalidInput)
        );
        let secret = CredentialSecret::new(b"secret-canary").expect("bounded fixture secret");
        assert!(secret.with_exposed(|value| value == b"secret-canary"));
    }

    #[test]
    fn credential_names_reject_empty_and_embedded_nul() {
        assert_eq!(validate_name(""), Err(CredentialError::InvalidInput));
        assert_eq!(
            validate_name("CyberKindred\0other"),
            Err(CredentialError::InvalidInput)
        );
        assert_eq!(
            validate_name("CyberKindred/*"),
            Err(CredentialError::InvalidInput)
        );
        assert_eq!(validate_name("CyberKindred/provider/fixture"), Ok(()));
    }

    #[cfg(windows)]
    #[test]
    fn win32_hresult_mapping_is_stable_and_redacted() {
        use windows::{
            Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_NO_SUCH_LOGON_SESSION},
            core::{Error, HRESULT},
        };

        let unavailable = Error::from_hresult(HRESULT::from_win32(ERROR_NO_SUCH_LOGON_SESSION.0));
        let denied = Error::from_hresult(HRESULT::from_win32(ERROR_ACCESS_DENIED.0));
        assert_eq!(
            super::platform::map_error(&unavailable),
            CredentialError::Unavailable
        );
        assert_eq!(
            super::platform::map_error(&denied),
            CredentialError::OperationFailed
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "explicit manual canary: writes, atomically replaces, reads, and removes test credentials"]
    fn windows_credential_live_canary_is_atomic_and_self_cleaning() {
        let store = CredentialStore::new();
        let prefix = format!("CyberKindred/manual-canary/{}/", std::process::id());
        let target = format!("{prefix}atomic-replacement");
        let _ = store.delete_prefix(&prefix);

        let mut initial = CredentialSecret::new(b"initial-canary").expect("fixture secret");
        store
            .write(&target, "api-key", &mut initial)
            .expect("initial write");
        assert!(store.contains(&target).expect("contains initial"));
        let mut replacement = CredentialSecret::new(b"replacement-canary").expect("fixture secret");
        store
            .write(&target, "api-key", &mut replacement)
            .expect("atomic replacement");

        let retrieved = store
            .read(&target)
            .expect("read replacement")
            .expect("stored replacement");
        assert!(retrieved.with_exposed(|value| value == b"replacement-canary"));
        assert_eq!(store.delete_prefix(&prefix).expect("canary cleanup"), 1);
        assert!(!store.contains(&target).expect("contains after cleanup"));
        assert!(store.read(&target).expect("verify cleanup").is_none());
    }
}
