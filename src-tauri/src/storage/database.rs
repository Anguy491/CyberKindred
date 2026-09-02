use super::{AppPaths, Repository, StorageError, StorageReason};
use chrono::Utc;
use sha2::{Digest, Sha256};
use sqlx::{
    ConnectOptions, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use std::{path::Path, time::Duration};

pub const APPLICATION_ID: i64 = 1_129_008_708;
pub const LATEST_SCHEMA_VERSION: i64 = 2;
const INITIAL_MIGRATION_NAME: &str = "initial_schema";
const INITIAL_MIGRATION_SQL: &str = include_str!("../../migrations/V0001__initial_schema.sql");
const SCAN_OPERATIONS_MIGRATION_NAME: &str = "scan_operations";
const SCAN_OPERATIONS_MIGRATION_SQL: &str =
    include_str!("../../migrations/V0002__scan_operations.sql");

#[derive(Clone, Copy)]
struct Migration {
    version: i64,
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: [Migration; 2] = [
    Migration {
        version: 1,
        name: INITIAL_MIGRATION_NAME,
        sql: INITIAL_MIGRATION_SQL,
    },
    Migration {
        version: 2,
        name: SCAN_OPERATIONS_MIGRATION_NAME,
        sql: SCAN_OPERATIONS_MIGRATION_SQL,
    },
];

/// Open, verified storage. A value is returned only after every migration and
/// post-migration integrity check succeeds.
pub struct Storage {
    writer: SqlitePool,
    reader: SqlitePool,
}

impl Storage {
    /// Opens the app database under validated Tauri-resolved paths.
    ///
    /// # Errors
    ///
    /// Returns a stable storage or migration error when identity, integrity,
    /// version, checksum, backup, migration, or pool initialization fails.
    pub async fn open(paths: &AppPaths, app_version: &str) -> Result<Self, StorageError> {
        if app_version.is_empty() || app_version.len() > 100 {
            return Err(StorageError::new(StorageReason::MigrationFailed));
        }
        let database_path = paths.database_path();
        if database_path.exists() {
            let validation = connect_validation_pool(&database_path).await?;
            let result = preflight_existing(&validation).await;
            validation.close().await;
            result?;
        }
        let writer = connect_pool(&database_path, 1, true).await?;
        if let Err(error) = initialize(&writer, paths, app_version).await {
            writer.close().await;
            return Err(error);
        }

        let reader = match connect_pool(&database_path, 4, false).await {
            Ok(pool) => pool,
            Err(error) => {
                writer.close().await;
                return Err(error);
            }
        };
        Ok(Self { writer, reader })
    }

    #[must_use]
    pub fn repository(&self) -> Repository {
        Repository::new(self.writer.clone())
    }

    /// Runs a bounded SQLite integrity check through the read pool.
    ///
    /// # Errors
    ///
    /// Returns `storage_integrity_failed` when SQLite does not report `ok`.
    pub async fn verify_integrity(&self) -> Result<(), StorageError> {
        verify_integrity(&self.reader).await
    }

    /// Performs the normal-exit passive WAL checkpoint.
    ///
    /// # Errors
    ///
    /// Returns `storage_write_failed` if SQLite cannot checkpoint the WAL.
    pub async fn passive_checkpoint(&self) -> Result<(), StorageError> {
        let _: (i64, i64, i64) = sqlx::query_as("PRAGMA wal_checkpoint(PASSIVE)")
            .fetch_one(&self.writer)
            .await
            .map_err(|_| StorageError::new(StorageReason::StorageWriteFailed))?;
        Ok(())
    }

    /// Performs the truncate checkpoint required before a package or backup.
    ///
    /// # Errors
    ///
    /// Returns `storage_write_failed` if SQLite cannot checkpoint the WAL.
    pub async fn truncate_checkpoint(&self) -> Result<(), StorageError> {
        let _: (i64, i64, i64) = sqlx::query_as("PRAGMA wal_checkpoint(TRUNCATE)")
            .fetch_one(&self.writer)
            .await
            .map_err(|_| StorageError::new(StorageReason::StorageWriteFailed))?;
        Ok(())
    }

    pub async fn close(self) {
        self.reader.close().await;
        self.writer.close().await;
    }
}

async fn connect_validation_pool(database_path: &Path) -> Result<SqlitePool, StorageError> {
    let options = SqliteConnectOptions::new()
        .filename(database_path)
        .read_only(true)
        .immutable(true)
        .foreign_keys(true)
        .busy_timeout(Duration::from_millis(3_000))
        .pragma("temp_store", "MEMORY")
        .disable_statement_logging();

    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .map_err(|_| StorageError::new(StorageReason::StorageIntegrityFailed))
}

async fn connect_pool(
    database_path: &Path,
    max_connections: u32,
    create_if_missing: bool,
) -> Result<SqlitePool, StorageError> {
    let options = SqliteConnectOptions::new()
        .filename(database_path)
        .create_if_missing(create_if_missing)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_millis(3_000))
        .pragma("temp_store", "MEMORY")
        .disable_statement_logging();

    SqlitePoolOptions::new()
        .max_connections(max_connections)
        .connect_with(options)
        .await
        .map_err(|_| StorageError::new(StorageReason::StorageIntegrityFailed))
}

async fn initialize(
    pool: &SqlitePool,
    paths: &AppPaths,
    app_version: &str,
) -> Result<(), StorageError> {
    verify_integrity(pool).await?;
    let application_id = pragma_i64(pool, "PRAGMA application_id").await?;
    let user_version = pragma_i64(pool, "PRAGMA user_version").await?;

    if application_id != 0 && application_id != APPLICATION_ID {
        return Err(StorageError::new(StorageReason::ForeignDatabase));
    }
    if user_version > LATEST_SCHEMA_VERSION {
        return Err(StorageError::new(StorageReason::DatabaseVersionUnsupported));
    }

    if user_version == 0 {
        let table_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        )
        .fetch_one(pool)
        .await
        .map_err(|_| StorageError::new(StorageReason::StorageReadFailed))?;
        if table_count != 0 {
            return Err(StorageError::new(StorageReason::ForeignDatabase));
        }
    }
    if user_version > 0 {
        verify_schema_at_version(pool, user_version).await?;
    }

    let mut current_version = user_version;
    while current_version < LATEST_SCHEMA_VERSION {
        let migration = migration_for_version(current_version + 1)?;
        create_verified_backup(pool, paths, current_version).await?;
        apply_migration(pool, app_version, migration).await?;
        current_version = migration.version;
        verify_schema_at_version(pool, current_version).await?;
    }
    verify_current_schema(pool).await
}

/// Verifies an existing file through a read-only connection before any writer
/// or WAL configuration is opened against it.
async fn preflight_existing(pool: &SqlitePool) -> Result<(), StorageError> {
    verify_integrity(pool).await?;
    let application_id = pragma_i64(pool, "PRAGMA application_id").await?;
    let user_version = pragma_i64(pool, "PRAGMA user_version").await?;
    if application_id != 0 && application_id != APPLICATION_ID {
        return Err(StorageError::new(StorageReason::ForeignDatabase));
    }
    if user_version > LATEST_SCHEMA_VERSION {
        return Err(StorageError::new(StorageReason::DatabaseVersionUnsupported));
    }
    if user_version == 0 {
        let table_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        )
        .fetch_one(pool)
        .await
        .map_err(|_| StorageError::new(StorageReason::StorageReadFailed))?;
        return if table_count == 0 {
            Ok(())
        } else {
            Err(StorageError::new(StorageReason::ForeignDatabase))
        };
    }
    verify_schema_at_version(pool, user_version).await
}

async fn apply_migration(
    pool: &SqlitePool,
    app_version: &str,
    migration: Migration,
) -> Result<(), StorageError> {
    let mut transaction = pool
        .begin()
        .await
        .map_err(|_| StorageError::new(StorageReason::MigrationFailed))?;
    if sqlx::raw_sql(migration.sql)
        .execute(&mut *transaction)
        .await
        .is_err()
    {
        let _ = transaction.rollback().await;
        return Err(StorageError::new(StorageReason::MigrationFailed));
    }
    let checksum = migration_checksum(migration.sql);
    let now_ms = Utc::now().timestamp_millis();
    if sqlx::query(
        "INSERT INTO schema_migrations(version, name, checksum_sha256, applied_at_ms, app_version) VALUES(?, ?, ?, ?, ?)",
    )
    .bind(migration.version)
    .bind(migration.name)
    .bind(checksum)
    .bind(now_ms)
    .bind(app_version)
    .execute(&mut *transaction)
    .await
    .is_err()
        || sqlx::query("PRAGMA application_id = 1129008708")
            .execute(&mut *transaction)
            .await
            .is_err()
        || set_user_version(&mut transaction, migration.version).await.is_err()
    {
        let _ = transaction.rollback().await;
        return Err(StorageError::new(StorageReason::MigrationFailed));
    }
    transaction
        .commit()
        .await
        .map_err(|_| StorageError::new(StorageReason::MigrationFailed))
}

async fn set_user_version(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    version: i64,
) -> Result<(), StorageError> {
    let statement = match version {
        1 => "PRAGMA user_version = 1",
        2 => "PRAGMA user_version = 2",
        _ => return Err(StorageError::new(StorageReason::MigrationFailed)),
    };
    sqlx::query(statement)
        .execute(&mut **transaction)
        .await
        .map(|_| ())
        .map_err(|_| StorageError::new(StorageReason::MigrationFailed))
}

fn migration_for_version(version: i64) -> Result<Migration, StorageError> {
    MIGRATIONS
        .iter()
        .find(|migration| migration.version == version)
        .copied()
        .ok_or_else(|| StorageError::new(StorageReason::MigrationFailed))
}

async fn verify_current_schema(pool: &SqlitePool) -> Result<(), StorageError> {
    verify_integrity(pool).await?;
    let application_id = pragma_i64(pool, "PRAGMA application_id").await?;
    let user_version = pragma_i64(pool, "PRAGMA user_version").await?;
    if application_id != APPLICATION_ID {
        return Err(StorageError::new(StorageReason::ForeignDatabase));
    }
    if user_version != LATEST_SCHEMA_VERSION {
        return Err(StorageError::new(StorageReason::DatabaseVersionUnsupported));
    }

    verify_schema_at_version(pool, user_version).await
}

async fn verify_schema_at_version(
    pool: &SqlitePool,
    user_version: i64,
) -> Result<(), StorageError> {
    if !(1..=LATEST_SCHEMA_VERSION).contains(&user_version) {
        return Err(StorageError::new(StorageReason::DatabaseVersionUnsupported));
    }
    let application_id = pragma_i64(pool, "PRAGMA application_id").await?;
    if application_id != APPLICATION_ID {
        return Err(StorageError::new(StorageReason::ForeignDatabase));
    }
    let rows: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT version, name, checksum_sha256 FROM schema_migrations ORDER BY version",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| StorageError::new(StorageReason::MigrationFailed))?;
    let expected_len = usize::try_from(user_version)
        .map_err(|_| StorageError::new(StorageReason::MigrationFailed))?;
    if rows.len() != expected_len {
        return Err(StorageError::new(StorageReason::MigrationFailed));
    }
    for (row, migration) in rows.iter().zip(MIGRATIONS.iter()) {
        if row.0 != migration.version
            || row.1 != migration.name
            || row.2 != migration_checksum(migration.sql)
        {
            return Err(StorageError::new(StorageReason::MigrationFailed));
        }
    }
    Ok(())
}

async fn verify_integrity(pool: &SqlitePool) -> Result<(), StorageError> {
    let result: String = sqlx::query_scalar("PRAGMA integrity_check(1)")
        .fetch_one(pool)
        .await
        .map_err(|_| StorageError::new(StorageReason::StorageIntegrityFailed))?;
    if result != "ok" {
        return Err(StorageError::new(StorageReason::StorageIntegrityFailed));
    }
    Ok(())
}

async fn pragma_i64(pool: &SqlitePool, statement: &'static str) -> Result<i64, StorageError> {
    sqlx::query_scalar(statement)
        .fetch_one(pool)
        .await
        .map_err(|_| StorageError::new(StorageReason::StorageReadFailed))
}

async fn create_verified_backup(
    pool: &SqlitePool,
    paths: &AppPaths,
    current_version: i64,
) -> Result<(), StorageError> {
    let utc_label = Utc::now().format("%Y%m%dT%H%M%S%3fZ").to_string();
    let backup_path = paths.migration_backup_path(current_version, &utc_label)?;
    let backup_text = backup_path
        .to_str()
        .ok_or_else(|| StorageError::new(StorageReason::PathDenied))?;
    sqlx::query("VACUUM INTO ?")
        .bind(backup_text)
        .execute(pool)
        .await
        .map_err(|_| StorageError::new(StorageReason::MigrationFailed))?;

    let backup_pool = connect_pool(&backup_path, 1, false).await?;
    let verification = verify_integrity(&backup_pool).await;
    backup_pool.close().await;
    verification
}

fn migration_checksum(migration_sql: &str) -> String {
    hex::encode(Sha256::digest(migration_sql.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn temporary_paths() -> (tempfile::TempDir, AppPaths) {
        let temp = tempfile::tempdir().expect("temporary root");
        let paths = AppPaths::create(
            temp.path().join("data"),
            temp.path().join("cache"),
            temp.path().join("logs"),
        )
        .expect("scoped app paths");
        (temp, paths)
    }

    fn database_artifacts(paths: &AppPaths) -> BTreeMap<String, String> {
        let database_path = paths.database_path();
        let prefix = database_path
            .file_name()
            .and_then(|value| value.to_str())
            .expect("database filename");
        std::fs::read_dir(database_path.parent().expect("database parent"))
            .expect("data directory")
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                if entry.file_type().ok()?.is_file() && name.starts_with(prefix) {
                    let bytes = std::fs::read(entry.path()).expect("database artifact");
                    Some((name, hex::encode(Sha256::digest(bytes))))
                } else {
                    None
                }
            })
            .collect()
    }

    #[tokio::test]
    async fn repository_migrates_empty_database_and_reopens_idempotently() {
        let (_temp, paths) = temporary_paths();
        let storage = Storage::open(&paths, "0.1.0").await.expect("initial open");
        assert_eq!(
            pragma_i64(&storage.reader, "PRAGMA application_id")
                .await
                .expect("application id"),
            APPLICATION_ID
        );
        assert_eq!(
            pragma_i64(&storage.reader, "PRAGMA user_version")
                .await
                .expect("user version"),
            LATEST_SCHEMA_VERSION
        );
        storage.close().await;

        let reopened = Storage::open(&paths, "0.1.0")
            .await
            .expect("idempotent reopen");
        reopened.verify_integrity().await.expect("integrity");
        reopened.close().await;
    }

    #[tokio::test]
    async fn repository_initial_schema_contains_every_documented_table() {
        let (_temp, paths) = temporary_paths();
        let storage = Storage::open(&paths, "0.1.0").await.expect("open");
        let names: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .fetch_all(&storage.reader)
        .await
        .expect("table list");
        for required in [
            "app_settings",
            "chat_sessions",
            "cover_art_cache",
            "cover_art_negative_cache",
            "feedback",
            "library_roots",
            "memories",
            "memory_proposal_sources",
            "memory_proposals",
            "memory_revisions",
            "messages",
            "os_integration_state",
            "outbox_events",
            "playback_events",
            "program_runs",
            "program_segments",
            "provider_usage",
            "scan_jobs",
            "scan_operation_roots",
            "scan_operations",
            "schedule_occurrences",
            "schedule_rules",
            "schema_migrations",
            "session_summaries",
            "track_external_metadata",
            "tracks",
            "tts_cache_entries",
            "tts_cache_leases",
            "tts_cache_references",
            "user_profile",
            "weather_cache",
        ] {
            assert!(
                names.iter().any(|name| name == required),
                "missing {required}"
            );
        }
        storage.close().await;
    }

    #[tokio::test]
    async fn repository_rejects_checksum_tampering_without_opening_state() {
        let (_temp, paths) = temporary_paths();
        let storage = Storage::open(&paths, "0.1.0").await.expect("open");
        sqlx::query("UPDATE schema_migrations SET checksum_sha256 = ? WHERE version = 1")
            .bind("0".repeat(64))
            .execute(&storage.writer)
            .await
            .expect("tamper fixture");
        storage.close().await;
        let before = database_artifacts(&paths);

        let Err(error) = Storage::open(&paths, "0.1.0").await else {
            panic!("checksum mismatch must fail closed");
        };
        assert_eq!(error.reason(), StorageReason::MigrationFailed);
        assert_eq!(database_artifacts(&paths), before);
    }

    #[tokio::test]
    async fn repository_migration_failure_rolls_back_and_preserves_verified_backup() {
        let (_temp, paths) = temporary_paths();
        let pool = connect_pool(&paths.database_path(), 1, true)
            .await
            .expect("fixture pool");
        create_verified_backup(&pool, &paths, 0)
            .await
            .expect("verified empty backup");
        let failure = apply_migration(
            &pool,
            "0.1.0",
            Migration {
                version: 1,
                name: INITIAL_MIGRATION_NAME,
                sql: "CREATE TABLE partial(id INTEGER); THIS IS NOT VALID SQL;",
            },
        )
        .await
        .expect_err("invalid migration must fail");
        assert_eq!(failure.reason(), StorageReason::MigrationFailed);
        let partial_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'partial'",
        )
        .fetch_one(&pool)
        .await
        .expect("schema inspection");
        assert_eq!(partial_count, 0);
        pool.close().await;

        let backups = std::fs::read_dir(paths.backups_dir())
            .expect("backup directory")
            .collect::<Result<Vec<_>, _>>()
            .expect("backup entries");
        assert_eq!(backups.len(), 1);
    }

    #[tokio::test]
    async fn repository_v0002_upgrade_preserves_v0001_data_and_verified_backup() {
        let (_temp, paths) = temporary_paths();
        let pool = connect_pool(&paths.database_path(), 1, true)
            .await
            .expect("fixture pool");
        apply_migration(&pool, "0.1.0", MIGRATIONS[0])
            .await
            .expect("V0001 fixture");
        sqlx::query(
            "INSERT INTO library_roots(id, canonical_path, path_key, display_name, enabled, created_at_ms) VALUES('root-preserved', 'fixture-path-never-exposed', 'fixture-key', 'Fixture', 1, 1)",
        )
        .execute(&pool)
        .await
        .expect("preserved root");
        sqlx::query(
            "INSERT INTO tracks(id, root_id, relative_path, relative_path_key, availability, format, file_size_bytes, modified_at_ms, duration_ms, genre_json, metadata_confidence, created_at_ms, updated_at_ms) VALUES('track-preserved', 'root-preserved', 'song.mp3', 'song.mp3', 'available', 'mp3', 1, 1, 1, '[]', 1.0, 1, 1)",
        )
        .execute(&pool)
        .await
        .expect("preserved track");
        pool.close().await;

        let storage = Storage::open(&paths, "0.2.0").await.expect("V0002 upgrade");
        assert_eq!(
            pragma_i64(&storage.reader, "PRAGMA user_version")
                .await
                .expect("V0002 user version"),
            2
        );
        let preserved: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM library_roots), (SELECT count(*) FROM tracks)",
        )
        .fetch_one(&storage.reader)
        .await
        .expect("preserved V0001 data");
        assert_eq!(preserved, (1, 1));
        let migrations: Vec<i64> =
            sqlx::query_scalar("SELECT version FROM schema_migrations ORDER BY version")
                .fetch_all(&storage.reader)
                .await
                .expect("migration history");
        assert_eq!(migrations, vec![1, 2]);
        storage.close().await;

        let backups = std::fs::read_dir(paths.backups_dir())
            .expect("backup directory")
            .collect::<Result<Vec<_>, _>>()
            .expect("backup entries");
        assert_eq!(backups.len(), 1);
        let backup = connect_validation_pool(&backups[0].path())
            .await
            .expect("backup pool");
        assert_eq!(
            pragma_i64(&backup, "PRAGMA user_version")
                .await
                .expect("backup version"),
            1
        );
        let backup_tracks: i64 = sqlx::query_scalar("SELECT count(*) FROM tracks")
            .fetch_one(&backup)
            .await
            .expect("backup tracks");
        assert_eq!(backup_tracks, 1);
        backup.close().await;
    }

    #[tokio::test]
    async fn repository_v0002_failure_rolls_back_without_touching_v0001_data() {
        let (_temp, paths) = temporary_paths();
        let pool = connect_pool(&paths.database_path(), 1, true)
            .await
            .expect("fixture pool");
        apply_migration(&pool, "0.1.0", MIGRATIONS[0])
            .await
            .expect("V0001 fixture");
        sqlx::query(
            "INSERT INTO library_roots(id, canonical_path, path_key, display_name, enabled, created_at_ms) VALUES('root-preserved', 'fixture-path-never-exposed', 'fixture-key', 'Fixture', 1, 1)",
        )
        .execute(&pool)
        .await
        .expect("preserved root");
        create_verified_backup(&pool, &paths, 1)
            .await
            .expect("verified V0001 backup");
        let failure = apply_migration(
            &pool,
            "0.2.0",
            Migration {
                version: 2,
                name: SCAN_OPERATIONS_MIGRATION_NAME,
                sql: "CREATE TABLE partial_v2(id INTEGER); THIS IS NOT VALID SQL;",
            },
        )
        .await
        .expect_err("invalid V0002 must fail");
        assert_eq!(failure.reason(), StorageReason::MigrationFailed);
        assert_eq!(
            pragma_i64(&pool, "PRAGMA user_version")
                .await
                .expect("still V0001"),
            1
        );
        let preserved: i64 = sqlx::query_scalar("SELECT count(*) FROM library_roots")
            .fetch_one(&pool)
            .await
            .expect("preserved root");
        assert_eq!(preserved, 1);
        let partial: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'partial_v2'",
        )
        .fetch_one(&pool)
        .await
        .expect("partial table check");
        assert_eq!(partial, 0);
        let migration_count: i64 = sqlx::query_scalar("SELECT count(*) FROM schema_migrations")
            .fetch_one(&pool)
            .await
            .expect("migration history");
        assert_eq!(migration_count, 1);
        pool.close().await;
        assert_eq!(
            std::fs::read_dir(paths.backups_dir())
                .expect("backup directory")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn repository_rejects_foreign_application_id_without_rewriting_it() {
        let (_temp, paths) = temporary_paths();
        let pool = connect_pool(&paths.database_path(), 1, true)
            .await
            .expect("fixture pool");
        sqlx::query("PRAGMA application_id = 42")
            .execute(&pool)
            .await
            .expect("foreign application id fixture");
        pool.close().await;
        let before = database_artifacts(&paths);

        let Err(error) = Storage::open(&paths, "0.1.0").await else {
            panic!("foreign database must fail closed");
        };
        assert_eq!(error.reason(), StorageReason::ForeignDatabase);
        assert_eq!(database_artifacts(&paths), before);
        let verification = connect_pool(&paths.database_path(), 1, false)
            .await
            .expect("verification pool");
        assert_eq!(
            pragma_i64(&verification, "PRAGMA application_id")
                .await
                .expect("preserved id"),
            42
        );
        verification.close().await;
    }

    #[tokio::test]
    async fn repository_rejects_future_schema_version() {
        let (_temp, paths) = temporary_paths();
        let pool = connect_pool(&paths.database_path(), 1, true)
            .await
            .expect("fixture pool");
        sqlx::query("PRAGMA application_id = 1129008708")
            .execute(&pool)
            .await
            .expect("application id fixture");
        sqlx::query("PRAGMA user_version = 3")
            .execute(&pool)
            .await
            .expect("future version fixture");
        pool.close().await;

        let Err(error) = Storage::open(&paths, "0.1.0").await else {
            panic!("future database must fail closed");
        };
        assert_eq!(error.reason(), StorageReason::DatabaseVersionUnsupported);
    }

    #[tokio::test]
    async fn repository_rejects_corrupt_database_bytes() {
        let (_temp, paths) = temporary_paths();
        std::fs::write(paths.database_path(), b"not a sqlite database")
            .expect("corrupt database fixture");
        let Err(error) = Storage::open(&paths, "0.1.0").await else {
            panic!("corrupt database must fail closed");
        };
        assert_eq!(error.reason(), StorageReason::StorageIntegrityFailed);
    }

    #[tokio::test]
    async fn repository_schema_enforces_foreign_keys_checks_and_active_scan_uniqueness() {
        let (_temp, paths) = temporary_paths();
        let storage = Storage::open(&paths, "0.1.0").await.expect("open");

        let invalid_foreign_key = sqlx::query(
            "INSERT INTO scan_jobs(id, root_id, status, app_version) VALUES('scan-orphan', 'missing-root', 'queued', '0.1.0')",
        )
        .execute(&storage.writer)
        .await;
        assert!(invalid_foreign_key.is_err());

        let invalid_json = sqlx::query(
            "INSERT INTO app_settings(key, value_json, schema_version, updated_at_ms) VALUES('ui.fixture', 'not-json', 1, 0)",
        )
        .execute(&storage.writer)
        .await;
        assert!(invalid_json.is_err());

        sqlx::query(
            "INSERT INTO library_roots(id, canonical_path, path_key, display_name, enabled, created_at_ms) VALUES('root-1', 'fixture-path-never-exposed', 'fixture-path-key', 'Fixture', 1, 0)",
        )
        .execute(&storage.writer)
        .await
        .expect("library root fixture");
        sqlx::query(
            "INSERT INTO scan_jobs(id, root_id, status, app_version) VALUES('scan-1', 'root-1', 'running', '0.1.0')",
        )
        .execute(&storage.writer)
        .await
        .expect("first active scan");
        let duplicate_active = sqlx::query(
            "INSERT INTO scan_jobs(id, root_id, status, app_version) VALUES('scan-2', 'root-1', 'queued', '0.1.0')",
        )
        .execute(&storage.writer)
        .await;
        assert!(duplicate_active.is_err());

        storage.close().await;
    }
}
