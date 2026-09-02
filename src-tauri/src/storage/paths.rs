use super::{StorageError, StorageReason};
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

/// An allowlisted cache subtree owned by `CyberKindred`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheArea {
    Tts,
    Covers,
    Staging,
}

/// Absolute directories supplied by Tauri's per-user path resolver.
///
/// This type intentionally does not implement `Debug` or `Serialize`: absolute
/// local paths must not accidentally cross logging or IPC boundaries.
pub struct AppPaths {
    data: PathBuf,
    cache: PathBuf,
    logs: PathBuf,
}

impl AppPaths {
    /// Validates and creates application-owned roots resolved by Tauri.
    ///
    /// # Errors
    ///
    /// Returns a path-scoped error for relative, broad, aliased, or inaccessible
    /// roots.
    pub fn create(
        app_data: impl AsRef<Path>,
        app_cache: impl AsRef<Path>,
        app_logs: impl AsRef<Path>,
    ) -> Result<Self, StorageError> {
        let app_data = prepare_root(app_data.as_ref())?;
        let app_cache = prepare_root(app_cache.as_ref())?;
        let app_logs = prepare_root(app_logs.as_ref())?;

        if app_data == app_cache || app_data == app_logs || app_cache == app_logs {
            return Err(StorageError::new(StorageReason::PathOutsideScope));
        }

        let paths = Self {
            data: app_data,
            cache: app_cache,
            logs: app_logs,
        };
        for path in [
            paths.backups_dir(),
            paths.exports_dir(),
            paths.cache_dir(CacheArea::Tts),
            paths.cache_dir(CacheArea::Covers),
            paths.cache_dir(CacheArea::Staging),
        ] {
            fs::create_dir_all(&path).map_err(|_| StorageError::new(StorageReason::PathDenied))?;
            reject_reparse_point(&path)?;
        }
        Ok(paths)
    }

    #[must_use]
    pub(crate) fn database_path(&self) -> PathBuf {
        self.data.join("cyberkindred.sqlite3")
    }

    #[must_use]
    pub(crate) fn backups_dir(&self) -> PathBuf {
        self.data.join("backups")
    }

    #[must_use]
    pub(crate) fn exports_dir(&self) -> PathBuf {
        self.data.join("exports")
    }

    #[must_use]
    pub(crate) fn cache_dir(&self, area: CacheArea) -> PathBuf {
        let child = match area {
            CacheArea::Tts => "tts",
            CacheArea::Covers => "covers",
            CacheArea::Staging => "staging",
        };
        self.cache.join(child)
    }

    /// Generates an allowlisted pre-migration backup path.
    pub(crate) fn migration_backup_path(
        &self,
        current_version: i64,
        utc_label: &str,
    ) -> Result<PathBuf, StorageError> {
        if current_version.is_negative()
            || utc_label.is_empty()
            || !utc_label.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            return Err(StorageError::new(StorageReason::PathOutsideScope));
        }
        join_relative(
            &self.data,
            Path::new("backups").join(format!(
                "pre-migration-v{current_version}-{utc_label}.sqlite3"
            )),
        )
    }

    /// Resolves a Rust-generated cache-relative path without permitting escape.
    ///
    /// # Errors
    ///
    /// Returns `path_outside_root` if `relative` is absolute, empty, or contains
    /// a non-normal component.
    pub fn cache_artifact(
        &self,
        area: CacheArea,
        relative: impl AsRef<Path>,
    ) -> Result<PathBuf, StorageError> {
        join_relative(&self.cache_dir(area), relative.as_ref())
    }

    #[must_use]
    pub fn log_directory(&self) -> &Path {
        &self.logs
    }
}

/// Resolves an existing file under a native-picker-authorized library root.
///
/// The returned path is for read-only open at the time of use. Scanner/player
/// code must call this again rather than trusting a stored concatenation.
///
/// # Errors
///
/// Returns a path-scoped error if the root is unavailable, the relative path
/// escapes it, or either path is a reparse point.
pub fn resolve_read_only_library_path(
    authorized_root: &Path,
    relative: &Path,
) -> Result<PathBuf, StorageError> {
    if !authorized_root.is_absolute() || !authorized_root.is_dir() {
        return Err(StorageError::new(StorageReason::PathDenied));
    }
    validate_relative(relative)?;
    reject_reparse_point(authorized_root)?;
    let canonical_root = fs::canonicalize(authorized_root)
        .map_err(|_| StorageError::new(StorageReason::PathDenied))?;

    let mut candidate = canonical_root.clone();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            return Err(StorageError::new(StorageReason::PathOutsideScope));
        };
        candidate.push(part);
        reject_reparse_point(&candidate)?;
    }
    let canonical_target =
        fs::canonicalize(candidate).map_err(|_| StorageError::new(StorageReason::PathDenied))?;
    if !canonical_target.starts_with(&canonical_root) {
        return Err(StorageError::new(StorageReason::PathOutsideScope));
    }
    Ok(canonical_target)
}

fn prepare_root(path: &Path) -> Result<PathBuf, StorageError> {
    if !path.is_absolute() || path.parent().is_none() || contains_parent_component(path) {
        return Err(StorageError::new(StorageReason::PathOutsideScope));
    }
    if path.exists() {
        reject_reparse_point(path)?;
    }
    fs::create_dir_all(path).map_err(|_| StorageError::new(StorageReason::PathDenied))?;
    reject_reparse_point(path)?;
    fs::canonicalize(path).map_err(|_| StorageError::new(StorageReason::PathDenied))
}

fn join_relative(base: &Path, relative: impl AsRef<Path>) -> Result<PathBuf, StorageError> {
    let relative = relative.as_ref();
    validate_relative(relative)?;
    let target = base.join(relative);
    if !target.starts_with(base) {
        return Err(StorageError::new(StorageReason::PathOutsideScope));
    }
    Ok(target)
}

fn validate_relative(path: &Path) -> Result<(), StorageError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(StorageError::new(StorageReason::PathOutsideScope));
    }
    Ok(())
}

fn contains_parent_component(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, Component::ParentDir))
}

#[cfg(windows)]
fn reject_reparse_point(path: &Path) -> Result<(), StorageError> {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    let metadata =
        fs::symlink_metadata(path).map_err(|_| StorageError::new(StorageReason::PathDenied))?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(StorageError::new(StorageReason::UnsafeReparsePoint));
    }
    Ok(())
}

#[cfg(not(windows))]
fn reject_reparse_point(path: &Path) -> Result<(), StorageError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| StorageError::new(StorageReason::PathDenied))?;
    if metadata.file_type().is_symlink() {
        return Err(StorageError::new(StorageReason::UnsafeReparsePoint));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_scope_rejects_relative_roots_and_parent_segments() {
        let Err(relative_error) = AppPaths::create("relative", "cache", "logs") else {
            panic!("relative roots must fail");
        };
        assert_eq!(relative_error.reason(), StorageReason::PathOutsideScope);

        let temp = tempfile::tempdir().expect("temporary root");
        let paths = AppPaths::create(
            temp.path().join("data"),
            temp.path().join("cache"),
            temp.path().join("logs"),
        )
        .expect("absolute scoped roots");
        assert_eq!(
            paths
                .cache_artifact(CacheArea::Tts, Path::new("..\\escape.mp3"))
                .expect_err("parent traversal must fail")
                .reason(),
            StorageReason::PathOutsideScope
        );
    }

    #[test]
    fn path_scope_creates_only_allowlisted_children() {
        let temp = tempfile::tempdir().expect("temporary root");
        let data = temp.path().join("data");
        let cache = temp.path().join("cache");
        let logs = temp.path().join("logs");
        let paths = AppPaths::create(&data, &cache, &logs).expect("scoped paths");

        assert_eq!(
            paths.database_path(),
            fs::canonicalize(&data)
                .expect("canonical data")
                .join("cyberkindred.sqlite3")
        );
        assert!(data.join("backups").is_dir());
        assert!(data.join("exports").is_dir());
        assert!(cache.join("tts").is_dir());
        assert!(cache.join("covers").is_dir());
        assert!(cache.join("staging").is_dir());
        assert_eq!(
            paths.log_directory(),
            fs::canonicalize(logs).expect("canonical logs")
        );
    }

    #[test]
    fn path_scope_library_resolution_never_escapes_the_authorized_root() {
        let temp = tempfile::tempdir().expect("temporary root");
        let library = temp.path().join("library");
        fs::create_dir_all(library.join("album")).expect("fixture directory");
        fs::write(
            library.join("album").join("track.wav"),
            b"generated fixture",
        )
        .expect("fixture file");

        let relative = Path::new("album").join("track.wav");
        let resolved = resolve_read_only_library_path(&library, &relative).expect("contained file");
        assert!(resolved.starts_with(fs::canonicalize(&library).expect("canonical library")));
        assert_eq!(
            resolve_read_only_library_path(&library, Path::new("..\\outside.wav"))
                .expect_err("escape must fail")
                .reason(),
            StorageReason::PathOutsideScope
        );
    }
}
