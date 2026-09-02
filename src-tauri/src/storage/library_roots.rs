use std::path::{Path, PathBuf};

use sqlx::{Row, Sqlite, Transaction, sqlite::SqliteRow};
use uuid::Uuid;

use super::{Repository, StorageError, StorageReason};

const LIBRARY_ROOTS_REVISION_KEY: &str = "ui.library_roots_revision";
const SETTINGS_SCHEMA_VERSION: i64 = 1;
const MAX_JS_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_PATH_CHARS: usize = 32_767;
const MAX_DISPLAY_NAME_CHARS: usize = 255;

/// Path-free representation returned by the storage boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StoredLibraryRoot {
    pub root_id: Uuid,
    pub display_name: String,
    pub available: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StoredLibraryRoots {
    pub roots: Vec<StoredLibraryRoot>,
    pub revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StoredLibraryRootAddition {
    pub root: StoredLibraryRoot,
    pub revision: u64,
}

struct ValidatedLibraryPath {
    canonical_text: String,
    path_key: String,
    display_name: String,
}

impl Repository {
    /// Returns enabled library roots without exposing their absolute paths.
    ///
    /// # Errors
    ///
    /// Returns a stable storage error if persisted state is unavailable or invalid.
    pub(crate) async fn load_library_roots(&self) -> Result<StoredLibraryRoots, StorageError> {
        let mut transaction = self.writer.begin().await.map_err(|_| read_error())?;
        let revision = load_revision_in_transaction(&mut transaction).await?;
        let rows = sqlx::query(
            "SELECT id, canonical_path, path_key, display_name, enabled FROM library_roots ORDER BY created_at_ms ASC, id ASC",
        )
        .fetch_all(&mut *transaction)
        .await
        .map_err(|_| read_error())?;

        let mut roots = Vec::with_capacity(rows.len());
        for row in rows {
            let root = decode_stored_row(&row)?;
            if let Some(root) = root {
                roots.push(root);
            }
        }
        transaction.commit().await.map_err(|_| read_error())?;
        Ok(StoredLibraryRoots { roots, revision })
    }

    /// Canonicalizes and persists one native-picker-selected directory.
    ///
    /// An already-enabled canonical path is a no-op. Re-adding a disabled path
    /// enables its existing opaque identity and advances the collection revision.
    ///
    /// # Errors
    ///
    /// Returns a stable path or storage error. No path text is retained in errors.
    pub(crate) async fn add_library_root(
        &self,
        selected_path: &Path,
        created_at_ms: i64,
    ) -> Result<StoredLibraryRootAddition, StorageError> {
        if created_at_ms < 0 {
            return Err(write_error());
        }
        let validated = validate_selected_path(selected_path)?;
        let mut transaction = self.writer.begin().await.map_err(|_| write_error())?;
        let revision = load_revision_in_transaction(&mut transaction).await?;
        let existing = sqlx::query(
            "SELECT id, canonical_path, path_key, display_name, enabled FROM library_roots WHERE path_key = ?",
        )
        .bind(&validated.path_key)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| write_error())?;

        if let Some(row) = existing {
            let root_id_text: String = row.try_get("id").map_err(|_| integrity_error())?;
            let canonical_path: String = row
                .try_get("canonical_path")
                .map_err(|_| integrity_error())?;
            let path_key: String = row.try_get("path_key").map_err(|_| integrity_error())?;
            let display_name: String =
                row.try_get("display_name").map_err(|_| integrity_error())?;
            let enabled: i64 = row.try_get("enabled").map_err(|_| integrity_error())?;
            validate_stored_row(
                &root_id_text,
                &canonical_path,
                &path_key,
                display_name,
                enabled,
            )?;
            let root_id = parse_root_id(&root_id_text)?;
            if enabled == 1 {
                transaction.commit().await.map_err(|_| write_error())?;
                return Ok(StoredLibraryRootAddition {
                    root: StoredLibraryRoot {
                        root_id,
                        display_name: validated.display_name,
                        available: true,
                    },
                    revision,
                });
            }

            let next_revision = advance_revision(revision)?;
            let updated = sqlx::query(
                "UPDATE library_roots SET canonical_path = ?, display_name = ?, enabled = 1 WHERE id = ? AND enabled = 0",
            )
            .bind(&validated.canonical_text)
            .bind(&validated.display_name)
            .bind(root_id.to_string())
            .execute(&mut *transaction)
            .await
            .map_err(|_| write_error())?;
            if updated.rows_affected() != 1 {
                return Err(write_error());
            }
            save_revision(&mut transaction, next_revision, created_at_ms).await?;
            transaction.commit().await.map_err(|_| write_error())?;
            return Ok(StoredLibraryRootAddition {
                root: StoredLibraryRoot {
                    root_id,
                    display_name: validated.display_name,
                    available: true,
                },
                revision: next_revision,
            });
        }

        let root_id = Uuid::now_v7();
        let next_revision = advance_revision(revision)?;
        sqlx::query(
            "INSERT INTO library_roots(id, canonical_path, path_key, display_name, enabled, created_at_ms) VALUES(?, ?, ?, ?, 1, ?)",
        )
        .bind(root_id.to_string())
        .bind(&validated.canonical_text)
        .bind(&validated.path_key)
        .bind(&validated.display_name)
        .bind(created_at_ms)
        .execute(&mut *transaction)
        .await
        .map_err(|_| write_error())?;
        save_revision(&mut transaction, next_revision, created_at_ms).await?;
        transaction.commit().await.map_err(|_| write_error())?;
        Ok(StoredLibraryRootAddition {
            root: StoredLibraryRoot {
                root_id,
                display_name: validated.display_name,
                available: true,
            },
            revision: next_revision,
        })
    }

    /// Soft-disables one root using the independent library-root revision.
    ///
    /// # Errors
    ///
    /// Returns a stable conflict, not-found, or storage error. Source files and
    /// track rows are never deleted by this operation.
    pub(crate) async fn remove_library_root(
        &self,
        root_id: Uuid,
        expected_revision: u64,
        updated_at_ms: i64,
    ) -> Result<u64, StorageError> {
        if updated_at_ms < 0 {
            return Err(write_error());
        }
        let mut transaction = self.writer.begin().await.map_err(|_| write_error())?;
        let revision = load_revision_in_transaction(&mut transaction).await?;
        if expected_revision != revision {
            return Err(StorageError::new(StorageReason::RevisionConflict));
        }
        let next_revision = advance_revision(revision)?;
        let updated =
            sqlx::query("UPDATE library_roots SET enabled = 0 WHERE id = ? AND enabled = 1")
                .bind(root_id.to_string())
                .execute(&mut *transaction)
                .await
                .map_err(|_| write_error())?;
        if updated.rows_affected() != 1 {
            return Err(StorageError::new(StorageReason::EntityNotFound));
        }
        save_revision(&mut transaction, next_revision, updated_at_ms).await?;
        transaction.commit().await.map_err(|_| write_error())?;
        Ok(next_revision)
    }
}

async fn load_revision_in_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
) -> Result<u64, StorageError> {
    let row = sqlx::query("SELECT value_json, schema_version FROM app_settings WHERE key = ?")
        .bind(LIBRARY_ROOTS_REVISION_KEY)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|_| read_error())?;
    let Some(row) = row else {
        return Ok(0);
    };
    let schema_version: i64 = row
        .try_get("schema_version")
        .map_err(|_| integrity_error())?;
    if schema_version != SETTINGS_SCHEMA_VERSION {
        return Err(integrity_error());
    }
    let value: String = row.try_get("value_json").map_err(|_| integrity_error())?;
    parse_revision(Some(&value))
}

/// Returns whether the transaction contains at least one structurally valid,
/// enabled root. Every row is decoded before returning so corruption cannot be
/// hidden behind an earlier valid row. Filesystem availability is deliberately
/// not an authorization prerequisite: an authorized removable root may be
/// temporarily disconnected.
pub(super) async fn has_valid_enabled_library_root_in_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
) -> Result<bool, StorageError> {
    let rows = sqlx::query(
        "SELECT id, canonical_path, path_key, display_name, enabled FROM library_roots ORDER BY created_at_ms ASC, id ASC",
    )
    .fetch_all(&mut **transaction)
    .await
    .map_err(|_| read_error())?;
    let mut has_enabled = false;
    for row in rows {
        has_enabled |= decode_stored_row(&row)?.is_some();
    }
    Ok(has_enabled)
}

async fn save_revision(
    transaction: &mut Transaction<'_, Sqlite>,
    revision: u64,
    updated_at_ms: i64,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO app_settings(key, value_json, schema_version, updated_at_ms) VALUES(?, ?, ?, ?) ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, schema_version = excluded.schema_version, updated_at_ms = excluded.updated_at_ms",
    )
    .bind(LIBRARY_ROOTS_REVISION_KEY)
    .bind(revision.to_string())
    .bind(SETTINGS_SCHEMA_VERSION)
    .bind(updated_at_ms)
    .execute(&mut **transaction)
    .await
    .map_err(|_| write_error())?;
    Ok(())
}

fn parse_revision(value: Option<&str>) -> Result<u64, StorageError> {
    let Some(value) = value else {
        return Ok(0);
    };
    let revision = serde_json::from_str::<u64>(value).map_err(|_| integrity_error())?;
    if revision > MAX_JS_SAFE_INTEGER {
        return Err(integrity_error());
    }
    Ok(revision)
}

fn advance_revision(revision: u64) -> Result<u64, StorageError> {
    let next = revision.checked_add(1).ok_or_else(integrity_error)?;
    if next > MAX_JS_SAFE_INTEGER {
        return Err(integrity_error());
    }
    Ok(next)
}

fn validate_selected_path(selected_path: &Path) -> Result<ValidatedLibraryPath, StorageError> {
    if !selected_path.is_absolute() {
        return Err(StorageError::new(StorageReason::PathDenied));
    }
    let selected_metadata = std::fs::symlink_metadata(selected_path)
        .map_err(|_| StorageError::new(StorageReason::PathDenied))?;
    if is_reparse_or_symlink(&selected_metadata) {
        return Err(StorageError::new(StorageReason::UnsafeReparsePoint));
    }
    if !selected_metadata.is_dir() {
        return Err(StorageError::new(StorageReason::PathDenied));
    }

    let canonical_path = std::fs::canonicalize(selected_path)
        .map_err(|_| StorageError::new(StorageReason::PathDenied))?;
    let canonical_metadata = std::fs::symlink_metadata(&canonical_path)
        .map_err(|_| StorageError::new(StorageReason::PathDenied))?;
    if is_reparse_or_symlink(&canonical_metadata) {
        return Err(StorageError::new(StorageReason::UnsafeReparsePoint));
    }
    if !canonical_metadata.is_dir() || !canonical_path.is_absolute() {
        return Err(StorageError::new(StorageReason::PathDenied));
    }

    let canonical_text = canonical_path
        .to_str()
        .filter(|value| !value.is_empty() && value.chars().count() <= MAX_PATH_CHARS)
        .ok_or_else(|| StorageError::new(StorageReason::PathDenied))?
        .to_owned();
    let display_name = display_name_for_path(&canonical_path)
        .map_err(|_| StorageError::new(StorageReason::PathDenied))?;
    let path_key =
        path_key(&canonical_text).map_err(|_| StorageError::new(StorageReason::PathDenied))?;
    Ok(ValidatedLibraryPath {
        canonical_text,
        path_key,
        display_name,
    })
}

fn validate_stored_row(
    root_id: &str,
    canonical_path: &str,
    stored_path_key: &str,
    display_name: String,
    enabled: i64,
) -> Result<Option<StoredLibraryRoot>, StorageError> {
    let root_id = parse_root_id(root_id)?;
    if canonical_path.is_empty()
        || canonical_path.chars().count() > MAX_PATH_CHARS
        || enabled != 0 && enabled != 1
    {
        return Err(integrity_error());
    }
    let path = PathBuf::from(canonical_path);
    if !path.is_absolute()
        || path_key(canonical_path)? != stored_path_key
        || display_name_for_path(&path)? != display_name
    {
        return Err(integrity_error());
    }
    if enabled == 0 {
        return Ok(None);
    }
    Ok(Some(StoredLibraryRoot {
        root_id,
        display_name,
        available: path_is_available(&path, stored_path_key),
    }))
}

fn decode_stored_row(row: &SqliteRow) -> Result<Option<StoredLibraryRoot>, StorageError> {
    let root_id: String = row.try_get("id").map_err(|_| integrity_error())?;
    let canonical_path: String = row
        .try_get("canonical_path")
        .map_err(|_| integrity_error())?;
    let stored_path_key: String = row.try_get("path_key").map_err(|_| integrity_error())?;
    validate_stored_row(
        &root_id,
        &canonical_path,
        &stored_path_key,
        row.try_get("display_name").map_err(|_| integrity_error())?,
        row.try_get("enabled").map_err(|_| integrity_error())?,
    )
}

fn parse_root_id(value: &str) -> Result<Uuid, StorageError> {
    let id = Uuid::parse_str(value).map_err(|_| integrity_error())?;
    if id.get_version_num() != 7 || id.to_string() != value {
        return Err(integrity_error());
    }
    Ok(id)
}

fn display_name_for_path(path: &Path) -> Result<String, StorageError> {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("Local library");
    if name.chars().count() > MAX_DISPLAY_NAME_CHARS
        || name.chars().any(char::is_control)
        || name.contains(['/', '\\'])
    {
        return Err(integrity_error());
    }
    Ok(name.to_owned())
}

fn path_key(canonical_path: &str) -> Result<String, StorageError> {
    if canonical_path.is_empty() || canonical_path.chars().any(char::is_control) {
        return Err(integrity_error());
    }
    let normalized = canonical_path.replace('\\', "/").to_lowercase();
    let trimmed = normalized.trim_end_matches('/');
    Ok(if trimmed.is_empty() {
        "/".to_owned()
    } else {
        trimmed.to_owned()
    })
}

fn path_is_available(path: &Path, stored_path_key: &str) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return false;
    };
    if !metadata.is_dir() || is_reparse_or_symlink(&metadata) {
        return false;
    }
    let Ok(current_canonical) = std::fs::canonicalize(path) else {
        return false;
    };
    if !current_canonical.is_absolute() {
        return false;
    }
    let Ok(canonical_metadata) = std::fs::symlink_metadata(&current_canonical) else {
        return false;
    };
    if !canonical_metadata.is_dir() || is_reparse_or_symlink(&canonical_metadata) {
        return false;
    }
    current_canonical
        .to_str()
        .filter(|value| !value.is_empty() && value.chars().count() <= MAX_PATH_CHARS)
        .and_then(|value| path_key(value).ok())
        .is_some_and(|current_path_key| current_path_key == stored_path_key)
}

#[cfg(windows)]
fn is_reparse_or_symlink(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_or_symlink(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn read_error() -> StorageError {
    StorageError::new(StorageReason::StorageReadFailed)
}

fn write_error() -> StorageError {
    StorageError::new(StorageReason::StorageWriteFailed)
}

fn integrity_error() -> StorageError {
    StorageError::new(StorageReason::StorageIntegrityFailed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{AppPaths, Storage};

    async fn fixture() -> (tempfile::TempDir, AppPaths, Storage, Repository) {
        let temp = tempfile::tempdir().expect("temporary root");
        let paths = AppPaths::create(
            temp.path().join("data"),
            temp.path().join("cache"),
            temp.path().join("logs"),
        )
        .expect("scoped paths");
        let storage = Storage::open(&paths, "0.1.0").await.expect("storage");
        let repository = storage.repository();
        (temp, paths, storage, repository)
    }

    #[tokio::test]
    async fn library_root_duplicate_and_soft_remove_keep_one_identity() {
        let (temp, _paths, storage, repository) = fixture().await;
        let music = temp.path().join("Music");
        std::fs::create_dir(&music).expect("music directory");

        let first = repository
            .add_library_root(&music, 100)
            .await
            .expect("first add");
        let duplicate = repository
            .add_library_root(&music, 200)
            .await
            .expect("duplicate no-op");
        assert_eq!(duplicate, first);
        assert_eq!(first.revision, 1);
        assert_eq!(
            repository
                .load_library_roots()
                .await
                .expect("list")
                .roots
                .len(),
            1
        );

        assert_eq!(
            repository
                .remove_library_root(first.root.root_id, 1, 300)
                .await
                .expect("soft remove"),
            2
        );
        assert!(
            repository
                .load_library_roots()
                .await
                .expect("disabled list")
                .roots
                .is_empty()
        );
        let retained: (i64, i64) =
            sqlx::query_as("SELECT count(*), sum(enabled) FROM library_roots WHERE id = ?")
                .bind(first.root.root_id.to_string())
                .fetch_one(&repository.writer)
                .await
                .expect("retained root row");
        assert_eq!(retained, (1, 0));

        let readded = repository
            .add_library_root(&music, 400)
            .await
            .expect("re-enable root");
        assert_eq!(readded.root.root_id, first.root.root_id);
        assert_eq!(readded.revision, 3);
        storage.close().await;
    }

    #[tokio::test]
    async fn library_root_revision_conflict_is_atomic() {
        let (temp, _paths, storage, repository) = fixture().await;
        let music = temp.path().join("music");
        std::fs::create_dir(&music).expect("music directory");
        let added = repository
            .add_library_root(&music, 100)
            .await
            .expect("add root");

        let error = repository
            .remove_library_root(added.root.root_id, 0, 200)
            .await
            .expect_err("stale revision");
        assert_eq!(error.reason(), StorageReason::RevisionConflict);
        let snapshot = repository
            .load_library_roots()
            .await
            .expect("unchanged root");
        assert_eq!(snapshot.revision, 1);
        assert_eq!(snapshot.roots, vec![added.root]);
        storage.close().await;
    }

    #[tokio::test]
    async fn library_root_restart_restores_safe_view() {
        let (temp, paths, storage, repository) = fixture().await;
        let music = temp.path().join("重启曲库");
        std::fs::create_dir(&music).expect("music directory");
        let added = repository
            .add_library_root(&music, 100)
            .await
            .expect("add root");
        drop(repository);
        storage.close().await;

        let reopened = Storage::open(&paths, "0.1.0").await.expect("reopen");
        assert_eq!(
            reopened
                .repository()
                .load_library_roots()
                .await
                .expect("restart snapshot"),
            StoredLibraryRoots {
                roots: vec![added.root],
                revision: 1,
            }
        );
        reopened.close().await;
    }

    #[tokio::test]
    async fn library_root_revision_rejects_future_schema_without_rewriting() {
        let (_temp, _paths, storage, repository) = fixture().await;
        let schema_version = 2_i64;
        sqlx::query(
            "INSERT INTO app_settings(key, value_json, schema_version, updated_at_ms) VALUES(?, ?, ?, ?)",
        )
        .bind(LIBRARY_ROOTS_REVISION_KEY)
        .bind("7")
        .bind(schema_version)
        .bind(123_i64)
        .execute(&repository.writer)
        .await
        .expect("revision fixture");

        let error = repository
            .load_library_roots()
            .await
            .expect_err("future revision schema must fail closed");
        assert_eq!(error.reason(), StorageReason::StorageIntegrityFailed);
        let retained: (String, i64, i64) = sqlx::query_as(
            "SELECT value_json, schema_version, updated_at_ms FROM app_settings WHERE key = ?",
        )
        .bind(LIBRARY_ROOTS_REVISION_KEY)
        .fetch_one(&repository.writer)
        .await
        .expect("retained revision fixture");
        assert_eq!(retained, ("7".to_owned(), schema_version, 123));
        storage.close().await;
    }

    #[tokio::test]
    async fn enabled_root_authorization_counts_disconnect_but_rejects_any_corrupt_row() {
        let (temp, _paths, storage, repository) = fixture().await;
        let music = temp.path().join("removable-music");
        std::fs::create_dir(&music).expect("music directory");
        repository
            .add_library_root(&music, 100)
            .await
            .expect("authorized root");
        std::fs::remove_dir(&music).expect("simulate disconnected root");

        let mut transaction = repository.writer.begin().await.expect("read transaction");
        assert!(
            has_valid_enabled_library_root_in_transaction(&mut transaction)
                .await
                .expect("disconnected but structurally valid root")
        );
        transaction
            .rollback()
            .await
            .expect("rollback read transaction");

        sqlx::query(
            "INSERT INTO library_roots(id, canonical_path, path_key, display_name, enabled, created_at_ms) VALUES(?, ?, ?, ?, 1, ?)",
        )
        .bind("corrupt-root-id")
        .bind("relative/corrupt/path")
        .bind("relative/corrupt/path")
        .bind("corrupt")
        .bind(200_i64)
        .execute(&repository.writer)
        .await
        .expect("corrupt row fixture");
        let mut transaction = repository.writer.begin().await.expect("read transaction");
        let error = has_valid_enabled_library_root_in_transaction(&mut transaction)
            .await
            .expect_err("corruption after a valid row must not be hidden");
        assert_eq!(error.reason(), StorageReason::StorageIntegrityFailed);
        transaction
            .rollback()
            .await
            .expect("rollback read transaction");
        storage.close().await;
    }

    #[tokio::test]
    async fn library_root_rejects_non_directory_and_symlink() {
        let (temp, _paths, storage, repository) = fixture().await;
        let file = temp.path().join("not-a-directory");
        std::fs::write(&file, b"fixture").expect("fixture file");
        let file_error = repository
            .add_library_root(&file, 100)
            .await
            .expect_err("file cannot be a root");
        assert_eq!(file_error.reason(), StorageReason::PathDenied);

        let target = temp.path().join("target");
        let link = temp.path().join("linked-root");
        std::fs::create_dir(&target).expect("target directory");
        create_directory_reparse(&target, &link).expect("directory reparse fixture");
        let link_error = repository
            .add_library_root(&link, 200)
            .await
            .expect_err("symlink cannot be a root");
        assert_eq!(link_error.reason(), StorageReason::UnsafeReparsePoint);
        assert_eq!(
            repository
                .load_library_roots()
                .await
                .expect("empty roots")
                .revision,
            0
        );
        storage.close().await;
    }

    #[tokio::test]
    async fn library_root_ancestor_reparse_redirect_becomes_unavailable() {
        let (temp, _paths, storage, repository) = fixture().await;
        let selected_parent = temp.path().join("selected-parent");
        let music = selected_parent.join("Music");
        std::fs::create_dir_all(&music).expect("selected music directory");
        let added = repository
            .add_library_root(&music, 100)
            .await
            .expect("add selected root");

        let original_parent = temp.path().join("original-parent");
        std::fs::rename(&selected_parent, &original_parent).expect("relocate original tree");
        let redirected_parent = temp.path().join("redirected-parent");
        std::fs::create_dir_all(redirected_parent.join("Music"))
            .expect("redirected music directory");
        create_directory_reparse(&redirected_parent, &selected_parent)
            .expect("ancestor reparse fixture");

        let snapshot = repository
            .load_library_roots()
            .await
            .expect("redirected root snapshot");
        assert_eq!(snapshot.revision, 1);
        assert_eq!(snapshot.roots.len(), 1);
        assert_eq!(snapshot.roots[0].root_id, added.root.root_id);
        assert_eq!(snapshot.roots[0].display_name, added.root.display_name);
        assert!(
            !snapshot.roots[0].available,
            "an ancestor redirect must never authorize the redirected tree"
        );
        remove_directory_reparse(&selected_parent).expect("remove ancestor reparse fixture");
        storage.close().await;
    }

    #[cfg(windows)]
    fn create_directory_reparse(target: &Path, link: &Path) -> std::io::Result<()> {
        let output = std::process::Command::new("cmd")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other("junction fixture creation failed"))
        }
    }

    #[cfg(unix)]
    fn create_directory_reparse(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn remove_directory_reparse(link: &Path) -> std::io::Result<()> {
        std::fs::remove_dir(link)
    }

    #[cfg(unix)]
    fn remove_directory_reparse(link: &Path) -> std::io::Result<()> {
        std::fs::remove_file(link)
    }
}
