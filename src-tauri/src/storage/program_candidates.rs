//! Path-free repository projection for deterministic local program planning.

use sqlx::Row;

use crate::program::{
    CandidateLoadRequest, CandidateTrack, MAX_CANDIDATE_SOURCE_ROWS, ProgramError, ProgramFuture,
    ProgramRepository,
};

use super::Repository;

impl ProgramRepository for Repository {
    fn load_candidate_tracks(
        &self,
        request: CandidateLoadRequest,
    ) -> ProgramFuture<'_, Result<Vec<CandidateTrack>, ProgramError>> {
        Box::pin(async move {
            if !(1..=MAX_CANDIDATE_SOURCE_ROWS + 1).contains(&request.maximum_rows) {
                return Err(ProgramError::InvalidSelectionRequest);
            }
            let rows = sqlx::query(
                "SELECT t.id, \
                 CASE WHEN m.match_status = 'adopted' THEN COALESCE(m.normalized_title, t.title) ELSE t.title END AS title, \
                 CASE WHEN m.match_status = 'adopted' THEN COALESCE(m.normalized_artist, t.artist) ELSE t.artist END AS artist, \
                 CASE WHEN m.match_status = 'adopted' THEN COALESCE(m.normalized_album, t.album) ELSE t.album END AS album, \
                 t.duration_ms, t.genre_json, \
                 CASE WHEN m.match_status = 'adopted' THEN m.tags_json ELSE '[]' END AS external_tags_json, \
                 t.last_played_at_ms, \
                 (SELECT count(*) FROM feedback f WHERE f.track_id = t.id AND f.feedback_type IN ('like', 'more_like_this') AND f.revoked_at_ms IS NULL) AS like_count, \
                 (SELECT count(*) FROM feedback f WHERE f.track_id = t.id AND f.feedback_type = 'skip' AND f.revoked_at_ms IS NULL) AS skip_count \
                 FROM tracks t \
                 JOIN library_roots r ON r.id = t.root_id \
                 LEFT JOIN track_external_metadata m ON m.track_id = t.id AND m.provider = 'musicbrainz' \
                 WHERE r.enabled = 1 AND t.availability = 'available' \
                 ORDER BY t.id ASC LIMIT ?",
            )
            .bind(i64::try_from(request.maximum_rows).map_err(|_| ProgramError::InvalidSelectionRequest)?)
            .fetch_all(&self.writer)
            .await
            .map_err(|_| ProgramError::RepositoryUnavailable)?;

            rows.iter().map(decode_candidate).collect()
        })
    }
}

fn decode_candidate(row: &sqlx::sqlite::SqliteRow) -> Result<CandidateTrack, ProgramError> {
    let mut normalized_tags: Vec<String> = serde_json::from_str(
        &row.try_get::<String, _>("genre_json")
            .map_err(|_| ProgramError::InvalidCandidateData)?,
    )
    .map_err(|_| ProgramError::InvalidCandidateData)?;
    let external_tags: Vec<String> = serde_json::from_str(
        &row.try_get::<String, _>("external_tags_json")
            .map_err(|_| ProgramError::InvalidCandidateData)?,
    )
    .map_err(|_| ProgramError::InvalidCandidateData)?;
    normalized_tags.extend(external_tags);
    let duration_ms = u64::try_from(
        row.try_get::<i64, _>("duration_ms")
            .map_err(|_| ProgramError::InvalidCandidateData)?,
    )
    .map_err(|_| ProgramError::InvalidCandidateData)?;
    let like_count = u32::try_from(
        row.try_get::<i64, _>("like_count")
            .map_err(|_| ProgramError::InvalidCandidateData)?,
    )
    .map_err(|_| ProgramError::InvalidCandidateData)?;
    let skip_count = u32::try_from(
        row.try_get::<i64, _>("skip_count")
            .map_err(|_| ProgramError::InvalidCandidateData)?,
    )
    .map_err(|_| ProgramError::InvalidCandidateData)?;
    Ok(CandidateTrack {
        track_id: row
            .try_get("id")
            .map_err(|_| ProgramError::InvalidCandidateData)?,
        title: row
            .try_get("title")
            .map_err(|_| ProgramError::InvalidCandidateData)?,
        artist: row
            .try_get("artist")
            .map_err(|_| ProgramError::InvalidCandidateData)?,
        album: row
            .try_get("album")
            .map_err(|_| ProgramError::InvalidCandidateData)?,
        duration_ms,
        normalized_tags,
        time_tags: Vec::new(),
        last_played_at_ms: row
            .try_get("last_played_at_ms")
            .map_err(|_| ProgramError::InvalidCandidateData)?,
        like_count,
        skip_count,
        playable: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{AppPaths, Storage};
    use uuid::Uuid;

    async fn fixture() -> (tempfile::TempDir, Storage, Repository, Uuid, Uuid) {
        let temp = tempfile::tempdir().expect("temporary root");
        let music = temp.path().join("Music");
        std::fs::create_dir_all(&music).expect("library root");
        let paths = AppPaths::create(
            temp.path().join("data"),
            temp.path().join("cache"),
            temp.path().join("logs"),
        )
        .expect("app paths");
        let storage = Storage::open(&paths, "0.3.0").await.expect("storage");
        let repository = storage.repository();
        let root_id = repository
            .add_library_root(&music, 1)
            .await
            .expect("root")
            .root
            .root_id;
        let adopted_id = Uuid::now_v7();
        let original_id = Uuid::now_v7();
        for (id, title, path, genres) in [
            (adopted_id, "Embedded title", "one.wav", r#"["jazz"]"#),
            (original_id, "Original title", "two.wav", r#"["ambient"]"#),
        ] {
            sqlx::query("INSERT INTO tracks(id, root_id, relative_path, relative_path_key, availability, format, file_size_bytes, modified_at_ms, duration_ms, title, artist, album, genre_json, metadata_confidence, created_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, 'available', 'wav', 1, 1, 60000, ?, 'Embedded artist', 'Embedded album', ?, 1.0, 1, 1)")
                .bind(id.to_string())
                .bind(root_id.to_string())
                .bind(path)
                .bind(path)
                .bind(title)
                .bind(genres)
                .execute(&repository.writer)
                .await
                .expect("track");
        }
        sqlx::query("INSERT INTO track_external_metadata(id, track_id, provider, normalized_title, normalized_artist, normalized_album, tags_json, confidence, match_status, fetched_at_ms) VALUES(?, ?, 'musicbrainz', 'Adopted title', 'Adopted artist', 'Adopted album', '[\"night\"]', 0.9, 'adopted', 1)")
            .bind(Uuid::now_v7().to_string())
            .bind(adopted_id.to_string())
            .execute(&repository.writer)
            .await
            .expect("adopted metadata");
        for feedback_type in ["like", "more_like_this", "skip"] {
            sqlx::query("INSERT INTO feedback(id, target_kind, track_id, feedback_type, value_json, created_at_ms) VALUES(?, 'track', ?, ?, '{}', 1)")
                .bind(Uuid::now_v7().to_string())
                .bind(adopted_id.to_string())
                .bind(feedback_type)
                .execute(&repository.writer)
                .await
                .expect("feedback");
        }
        (temp, storage, repository, adopted_id, original_id)
    }

    #[tokio::test]
    async fn candidate_repository_projects_adopted_tags_and_active_feedback_without_paths() {
        let (temp, storage, repository, adopted_id, original_id) = fixture().await;
        let candidates = repository
            .load_candidate_tracks(CandidateLoadRequest { maximum_rows: 10 })
            .await
            .expect("candidates");
        assert_eq!(candidates.len(), 2);
        let adopted = candidates
            .iter()
            .find(|candidate| candidate.track_id == adopted_id.to_string())
            .expect("adopted candidate");
        assert_eq!(adopted.title.as_deref(), Some("Adopted title"));
        assert_eq!(adopted.normalized_tags, ["jazz", "night"]);
        assert_eq!((adopted.like_count, adopted.skip_count), (2, 1));
        let original = candidates
            .iter()
            .find(|candidate| candidate.track_id == original_id.to_string())
            .expect("original candidate");
        assert_eq!(original.title.as_deref(), Some("Original title"));
        assert_eq!(original.normalized_tags, ["ambient"]);
        let debug = format!("{candidates:?}");
        assert!(!debug.contains("one.wav"));
        assert!(!debug.contains(temp.path().to_string_lossy().as_ref()));
        storage.close().await;
    }

    #[tokio::test]
    async fn candidate_repository_rejects_unbounded_reads_before_querying() {
        let (_temp, storage, repository, _adopted_id, _original_id) = fixture().await;
        let error = repository
            .load_candidate_tracks(CandidateLoadRequest {
                maximum_rows: MAX_CANDIDATE_SOURCE_ROWS + 2,
            })
            .await
            .expect_err("unbounded request must fail");
        assert_eq!(error, ProgramError::InvalidSelectionRequest);
        storage.close().await;
    }
}
