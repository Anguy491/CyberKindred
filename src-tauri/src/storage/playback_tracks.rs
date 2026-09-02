//! Internal, path-bearing facts used to authorize local playback.

use std::path::PathBuf;

use sqlx::{QueryBuilder, Row, Sqlite};
use uuid::Uuid;

use super::{Repository, StorageError, StorageReason};

const MAX_TRACKS_PER_PROGRAM: usize = 200;
const MAX_JS_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// One enabled, playable library row. This intentionally implements neither
/// `Debug` nor serialization because it contains local path information.
pub(crate) struct StoredPlaybackTrack {
    pub(crate) track_id: String,
    pub(crate) authorized_root: PathBuf,
    pub(crate) relative_path: PathBuf,
    pub(crate) title: String,
    pub(crate) artist: Option<String>,
    pub(crate) album: Option<String>,
    pub(crate) duration_ms: u64,
}

impl Repository {
    /// Loads a bounded set of path-bearing playback facts. Missing, disabled,
    /// malformed, or unavailable rows fail closed rather than shrinking the
    /// requested queue silently.
    pub(crate) async fn load_playback_tracks(
        &self,
        track_ids: &[String],
    ) -> Result<Vec<StoredPlaybackTrack>, StorageError> {
        validate_track_ids(track_ids)?;
        let mut builder = QueryBuilder::<Sqlite>::new(
            "SELECT t.id, r.canonical_path, t.relative_path, t.title, t.artist, t.album, t.duration_ms \
             FROM tracks t JOIN library_roots r ON r.id = t.root_id \
             WHERE r.enabled = 1 AND t.availability = 'available' AND t.id IN (",
        );
        {
            let mut separated = builder.separated(", ");
            for track_id in track_ids {
                separated.push_bind(track_id);
            }
        }
        builder.push(")");
        let rows = builder
            .build()
            .fetch_all(&self.writer)
            .await
            .map_err(|_| read_error())?;
        if rows.len() != track_ids.len() {
            return Err(StorageError::new(StorageReason::EntityNotFound));
        }

        let mut decoded = Vec::with_capacity(rows.len());
        for row in rows {
            let track_id: String = row.try_get("id").map_err(|_| integrity_error())?;
            let parsed = Uuid::parse_str(&track_id).map_err(|_| integrity_error())?;
            if parsed.is_nil() || parsed.get_version_num() != 7 || parsed.to_string() != track_id {
                return Err(integrity_error());
            }
            let duration_ms = u64::try_from(
                row.try_get::<i64, _>("duration_ms")
                    .map_err(|_| integrity_error())?,
            )
            .map_err(|_| integrity_error())?;
            if !(1..=MAX_JS_SAFE_INTEGER).contains(&duration_ms) {
                return Err(integrity_error());
            }
            let title = row
                .try_get::<Option<String>, _>("title")
                .map_err(|_| integrity_error())?
                .unwrap_or_else(|| "未命名曲目".to_owned());
            decoded.push(StoredPlaybackTrack {
                track_id,
                authorized_root: PathBuf::from(
                    row.try_get::<String, _>("canonical_path")
                        .map_err(|_| integrity_error())?,
                ),
                relative_path: PathBuf::from(
                    row.try_get::<String, _>("relative_path")
                        .map_err(|_| integrity_error())?,
                ),
                title,
                artist: row.try_get("artist").map_err(|_| integrity_error())?,
                album: row.try_get("album").map_err(|_| integrity_error())?,
                duration_ms,
            });
        }
        decoded.sort_by_key(|track| {
            track_ids
                .iter()
                .position(|requested| requested == &track.track_id)
                .unwrap_or(track_ids.len())
        });
        if decoded
            .iter()
            .zip(track_ids)
            .any(|(track, requested)| &track.track_id != requested)
        {
            return Err(integrity_error());
        }
        Ok(decoded)
    }
}

fn validate_track_ids(track_ids: &[String]) -> Result<(), StorageError> {
    if !(1..=MAX_TRACKS_PER_PROGRAM).contains(&track_ids.len()) {
        return Err(invalid_setting());
    }
    let mut unique = std::collections::HashSet::with_capacity(track_ids.len());
    for track_id in track_ids {
        let parsed = Uuid::parse_str(track_id).map_err(|_| invalid_setting())?;
        if parsed.is_nil()
            || parsed.get_version_num() != 7
            || parsed.to_string() != *track_id
            || !unique.insert(track_id)
        {
            return Err(invalid_setting());
        }
    }
    Ok(())
}

const fn invalid_setting() -> StorageError {
    StorageError::new(StorageReason::InvalidSetting)
}

const fn read_error() -> StorageError {
    StorageError::new(StorageReason::StorageReadFailed)
}

const fn integrity_error() -> StorageError {
    StorageError::new(StorageReason::StorageIntegrityFailed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{AppPaths, Storage};

    #[tokio::test]
    async fn playback_tracks_require_enabled_available_rows_and_preserve_request_order() {
        let temp = tempfile::tempdir().expect("temporary root");
        let music = temp.path().join("Music");
        std::fs::create_dir_all(&music).expect("music root");
        std::fs::write(music.join("one.wav"), b"fixture one").expect("first fixture");
        std::fs::write(music.join("two.wav"), b"fixture two").expect("second fixture");
        let paths = AppPaths::create(
            temp.path().join("data"),
            temp.path().join("cache"),
            temp.path().join("logs"),
        )
        .expect("paths");
        let storage = Storage::open(&paths, "0.3.0").await.expect("storage");
        let repository = storage.repository();
        let root = repository
            .add_library_root(&music, 1)
            .await
            .expect("authorized root");
        let scan_id = Uuid::now_v7();
        sqlx::query("INSERT INTO scan_jobs(id, root_id, status, app_version) VALUES(?, ?, 'completed', '0.3.0')")
            .bind(scan_id.to_string())
            .bind(root.root.root_id.to_string())
            .execute(&repository.writer)
            .await
            .expect("scan job");
        let first = Uuid::now_v7();
        let second = Uuid::now_v7();
        for (id, relative, title) in [(first, "one.wav", "One"), (second, "two.wav", "Two")] {
            sqlx::query("INSERT INTO tracks(id, root_id, relative_path, relative_path_key, availability, format, file_size_bytes, modified_at_ms, duration_ms, last_seen_scan_id, title, metadata_confidence, created_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, 'available', 'wav', 11, 1, 60000, ?, ?, 1.0, 1, 1)")
                .bind(id.to_string())
                .bind(root.root.root_id.to_string())
                .bind(relative)
                .bind(relative)
                .bind(scan_id.to_string())
                .bind(title)
                .execute(&repository.writer)
                .await
                .expect("track");
        }

        let requested = vec![second.to_string(), first.to_string()];
        let tracks = repository
            .load_playback_tracks(&requested)
            .await
            .expect("playback facts");
        assert_eq!(
            tracks
                .iter()
                .map(|track| track.track_id.as_str())
                .collect::<Vec<_>>(),
            requested.iter().map(String::as_str).collect::<Vec<_>>()
        );

        repository
            .remove_library_root(root.root.root_id, root.revision, 2)
            .await
            .expect("disable root");
        let Err(error) = repository.load_playback_tracks(&requested).await else {
            panic!("disabled root must fail closed");
        };
        assert_eq!(error.reason(), StorageReason::EntityNotFound);
        storage.close().await;
    }

    #[tokio::test]
    async fn playback_tracks_reject_duplicate_or_non_v7_ids_before_query() {
        let temp = tempfile::tempdir().expect("temporary root");
        let paths = AppPaths::create(
            temp.path().join("data"),
            temp.path().join("cache"),
            temp.path().join("logs"),
        )
        .expect("paths");
        let storage = Storage::open(&paths, "0.3.0").await.expect("storage");
        let repository = storage.repository();
        for ids in [
            vec!["not-an-id".to_owned()],
            vec![Uuid::now_v7().to_string(); 2],
        ] {
            let Err(error) = repository.load_playback_tracks(&ids).await else {
                panic!("invalid ids must fail before storage query");
            };
            assert_eq!(error.reason(), StorageReason::InvalidSetting);
        }
        storage.close().await;
    }
}
