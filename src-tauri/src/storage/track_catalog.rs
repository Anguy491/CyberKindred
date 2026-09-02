//! Path-free read model for the local-library catalogue.

use chrono::{SecondsFormat, TimeZone, Utc};
use sqlx::{QueryBuilder, Row, Sqlite, sqlite::SqliteRow};
use uuid::Uuid;

use super::{Repository, StorageError, StorageReason};

const MAX_PAGE_SIZE: u32 = 200;
const MAX_QUERY_CHARS: usize = 300;
const MAX_JS_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StoredTrackSort {
    Title,
    Artist,
    Album,
    Recent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StoredTrackAvailabilityFilter {
    Playable,
    Missing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StoredTrackMatchFilter {
    Matched,
    Unmatched,
    Review,
}

pub(crate) struct StoredTrackCatalogQuery {
    pub(crate) offset: u64,
    pub(crate) limit: u32,
    pub(crate) query: Option<String>,
    pub(crate) sort: StoredTrackSort,
    pub(crate) availability: Option<StoredTrackAvailabilityFilter>,
    pub(crate) match_status: Option<StoredTrackMatchFilter>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StoredTrackTags {
    pub(crate) title: Option<String>,
    pub(crate) artist: Option<String>,
    pub(crate) album: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StoredTrackEnrichment {
    pub(crate) tags: StoredTrackTags,
    pub(crate) confidence: f64,
    pub(crate) fetched_at: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StoredTrackAvailability {
    Playable,
    Missing,
    Corrupt,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StoredTrackMatchStatus {
    Matched,
    Unmatched,
    Review,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StoredTrackCatalogItem {
    pub(crate) track_id: Uuid,
    pub(crate) availability: StoredTrackAvailability,
    pub(crate) duration_ms: u64,
    pub(crate) artwork_available: bool,
    pub(crate) original: StoredTrackTags,
    pub(crate) enriched: Option<StoredTrackEnrichment>,
    pub(crate) match_status: StoredTrackMatchStatus,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StoredTrackCatalogPage {
    pub(crate) items: Vec<StoredTrackCatalogItem>,
    pub(crate) next_offset: Option<u64>,
}

impl Repository {
    /// Returns one bounded, path-free page from enabled local-library roots.
    pub(crate) async fn list_track_catalog(
        &self,
        query: StoredTrackCatalogQuery,
    ) -> Result<StoredTrackCatalogPage, StorageError> {
        validate_query(&query)?;
        let mut builder = QueryBuilder::<Sqlite>::new(
            "SELECT t.id, t.availability, t.duration_ms, t.embedded_cover_hash, \
             t.title, t.artist, t.album, m.normalized_title, m.normalized_artist, \
             m.normalized_album, m.confidence, m.match_status, m.fetched_at_ms, \
             EXISTS(SELECT 1 FROM cover_art_cache c WHERE c.track_id = t.id) AS cached_artwork \
             FROM tracks t JOIN library_roots r ON r.id = t.root_id \
             LEFT JOIN track_external_metadata m ON m.track_id = t.id AND m.provider = 'musicbrainz' \
             WHERE r.enabled = 1",
        );

        if let Some(availability) = query.availability {
            builder
                .push(" AND t.availability = ")
                .push_bind(match availability {
                    StoredTrackAvailabilityFilter::Playable => "available",
                    StoredTrackAvailabilityFilter::Missing => "missing",
                });
        }
        if let Some(match_status) = query.match_status {
            match match_status {
                StoredTrackMatchFilter::Matched => {
                    builder.push(" AND m.match_status = 'adopted'");
                }
                StoredTrackMatchFilter::Review => {
                    builder.push(" AND m.match_status = 'suggested'");
                }
                StoredTrackMatchFilter::Unmatched => {
                    builder.push(" AND (m.match_status IS NULL OR m.match_status = 'no_match')");
                }
            }
        }
        if let Some(search) = query.query.as_deref() {
            let pattern = format!("%{}%", escape_like(search));
            builder
                .push(" AND (COALESCE(t.title, '') LIKE ")
                .push_bind(pattern.clone())
                .push(" ESCAPE '\\' COLLATE NOCASE OR COALESCE(t.artist, '') LIKE ")
                .push_bind(pattern.clone())
                .push(" ESCAPE '\\' COLLATE NOCASE OR COALESCE(t.album, '') LIKE ")
                .push_bind(pattern.clone())
                .push(" ESCAPE '\\' COLLATE NOCASE OR COALESCE(m.normalized_title, '') LIKE ")
                .push_bind(pattern.clone())
                .push(" ESCAPE '\\' COLLATE NOCASE OR COALESCE(m.normalized_artist, '') LIKE ")
                .push_bind(pattern.clone())
                .push(" ESCAPE '\\' COLLATE NOCASE OR COALESCE(m.normalized_album, '') LIKE ")
                .push_bind(pattern)
                .push(" ESCAPE '\\' COLLATE NOCASE)");
        }

        match query.sort {
            StoredTrackSort::Title => builder.push(
                " ORDER BY lower(COALESCE(t.title, '')) ASC, lower(COALESCE(t.artist, '')) ASC, t.id ASC",
            ),
            StoredTrackSort::Artist => builder.push(
                " ORDER BY lower(COALESCE(t.artist, '')) ASC, lower(COALESCE(t.title, '')) ASC, t.id ASC",
            ),
            StoredTrackSort::Album => builder.push(
                " ORDER BY lower(COALESCE(t.album, '')) ASC, lower(COALESCE(t.title, '')) ASC, t.id ASC",
            ),
            StoredTrackSort::Recent => builder.push(
                " ORDER BY t.last_played_at_ms IS NULL ASC, t.last_played_at_ms DESC, t.id ASC",
            ),
        };
        builder
            .push(" LIMIT ")
            .push_bind(i64::from(query.limit) + 1)
            .push(" OFFSET ")
            .push_bind(i64::try_from(query.offset).map_err(|_| invalid_setting())?);

        let rows = builder
            .build()
            .fetch_all(&self.writer)
            .await
            .map_err(|_| StorageError::new(StorageReason::StorageReadFailed))?;
        let has_more = rows.len() > query.limit as usize;
        let items = rows
            .iter()
            .take(query.limit as usize)
            .map(decode_track)
            .collect::<Result<Vec<_>, _>>()?;
        let next_offset = if has_more {
            Some(
                query
                    .offset
                    .checked_add(u64::from(query.limit))
                    .filter(|value| *value <= MAX_JS_SAFE_INTEGER)
                    .ok_or_else(integrity_error)?,
            )
        } else {
            None
        };
        Ok(StoredTrackCatalogPage { items, next_offset })
    }
}

fn validate_query(query: &StoredTrackCatalogQuery) -> Result<(), StorageError> {
    if !(1..=MAX_PAGE_SIZE).contains(&query.limit)
        || query.offset > MAX_JS_SAFE_INTEGER
        || query.query.as_deref().is_some_and(|value| {
            value.is_empty()
                || value.chars().count() > MAX_QUERY_CHARS
                || value.chars().any(char::is_control)
        })
    {
        return Err(invalid_setting());
    }
    Ok(())
}

fn decode_track(row: &SqliteRow) -> Result<StoredTrackCatalogItem, StorageError> {
    let track_id_text: String = row.try_get("id").map_err(|_| integrity_error())?;
    let track_id = Uuid::parse_str(&track_id_text).map_err(|_| integrity_error())?;
    if track_id.is_nil() || track_id.get_version_num() != 7 || track_id.to_string() != track_id_text
    {
        return Err(integrity_error());
    }
    let duration_ms = row
        .try_get::<i64, _>("duration_ms")
        .map_err(|_| integrity_error())?;
    let duration_ms = u64::try_from(duration_ms).map_err(|_| integrity_error())?;
    if duration_ms > MAX_JS_SAFE_INTEGER {
        return Err(integrity_error());
    }
    let availability = match row
        .try_get::<String, _>("availability")
        .map_err(|_| integrity_error())?
        .as_str()
    {
        "available" => StoredTrackAvailability::Playable,
        "missing" => StoredTrackAvailability::Missing,
        "corrupt" => StoredTrackAvailability::Corrupt,
        "unsupported" => StoredTrackAvailability::Unsupported,
        _ => return Err(integrity_error()),
    };
    let match_status: Option<String> =
        row.try_get("match_status").map_err(|_| integrity_error())?;
    let (match_status, enriched) = match match_status.as_deref() {
        Some("adopted" | "suggested") => {
            let confidence: f64 = row.try_get("confidence").map_err(|_| integrity_error())?;
            let fetched_at_ms: i64 = row
                .try_get("fetched_at_ms")
                .map_err(|_| integrity_error())?;
            if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) || fetched_at_ms < 0 {
                return Err(integrity_error());
            }
            let fetched_at = Utc
                .timestamp_millis_opt(fetched_at_ms)
                .single()
                .ok_or_else(integrity_error)?
                .to_rfc3339_opts(SecondsFormat::Millis, true);
            let status = if match_status.as_deref() == Some("adopted") {
                StoredTrackMatchStatus::Matched
            } else {
                StoredTrackMatchStatus::Review
            };
            (
                status,
                Some(StoredTrackEnrichment {
                    tags: StoredTrackTags {
                        title: row
                            .try_get("normalized_title")
                            .map_err(|_| integrity_error())?,
                        artist: row
                            .try_get("normalized_artist")
                            .map_err(|_| integrity_error())?,
                        album: row
                            .try_get("normalized_album")
                            .map_err(|_| integrity_error())?,
                    },
                    confidence,
                    fetched_at,
                }),
            )
        }
        None | Some("no_match") => (StoredTrackMatchStatus::Unmatched, None),
        Some(_) => return Err(integrity_error()),
    };
    let embedded_cover_hash: Option<String> = row
        .try_get("embedded_cover_hash")
        .map_err(|_| integrity_error())?;
    let cached_artwork: i64 = row
        .try_get("cached_artwork")
        .map_err(|_| integrity_error())?;
    if !matches!(cached_artwork, 0 | 1) {
        return Err(integrity_error());
    }
    Ok(StoredTrackCatalogItem {
        track_id,
        availability,
        duration_ms,
        artwork_available: embedded_cover_hash.is_some() || cached_artwork == 1,
        original: StoredTrackTags {
            title: row.try_get("title").map_err(|_| integrity_error())?,
            artist: row.try_get("artist").map_err(|_| integrity_error())?,
            album: row.try_get("album").map_err(|_| integrity_error())?,
        },
        enriched,
        match_status,
    })
}

fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

const fn invalid_setting() -> StorageError {
    StorageError::new(StorageReason::InvalidSetting)
}

const fn integrity_error() -> StorageError {
    StorageError::new(StorageReason::StorageIntegrityFailed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{AppPaths, Storage};

    async fn fixture() -> (tempfile::TempDir, Storage, Repository, Uuid, Uuid, Uuid) {
        let temp = tempfile::tempdir().expect("temporary root");
        let root = temp.path().join("Music");
        std::fs::create_dir_all(&root).expect("library root");
        let paths = AppPaths::create(
            root.parent().expect("fixture parent").join("data"),
            root.parent().expect("fixture parent").join("cache"),
            root.parent().expect("fixture parent").join("logs"),
        )
        .expect("app paths");
        let storage = Storage::open(&paths, "0.3.0").await.expect("storage");
        let repository = storage.repository();
        let root_id = repository
            .add_library_root(&root, 1)
            .await
            .expect("root")
            .root
            .root_id;
        let scan_id = Uuid::now_v7();
        sqlx::query("INSERT INTO scan_jobs(id, root_id, status, files_seen, tracks_indexed, errors_count, started_at_ms, finished_at_ms, app_version) VALUES(?, ?, 'completed', 2, 2, 0, 2, 3, '0.3.0')")
            .bind(scan_id.to_string())
            .bind(root_id.to_string())
            .execute(&repository.writer)
            .await
            .expect("scan job");
        let first = Uuid::now_v7();
        let second = Uuid::now_v7();
        for (id, path, title, availability, last_played) in [
            (first, "one.wav", "Night 100%", "available", Some(100_i64)),
            (second, "two.wav", "Morning", "missing", None),
        ] {
            sqlx::query("INSERT INTO tracks(id, root_id, relative_path, relative_path_key, availability, format, file_size_bytes, modified_at_ms, duration_ms, last_seen_scan_id, title, artist, album, metadata_confidence, created_at_ms, updated_at_ms, last_played_at_ms) VALUES(?, ?, ?, ?, ?, 'wav', 1, 1, 60000, ?, ?, 'Fixture Artist', 'Fixture Album', 1.0, 1, 1, ?)")
                .bind(id.to_string())
                .bind(root_id.to_string())
                .bind(path)
                .bind(path)
                .bind(availability)
                .bind(scan_id.to_string())
                .bind(title)
                .bind(last_played)
                .execute(&repository.writer)
                .await
                .expect("track");
        }
        sqlx::query("INSERT INTO track_external_metadata(id, track_id, provider, external_id, normalized_title, normalized_artist, normalized_album, confidence, match_status, fetched_at_ms) VALUES(?, ?, 'musicbrainz', ?, 'Enriched Night', 'Enriched Artist', 'Enriched Album', 0.8, 'suggested', 1000)")
            .bind(Uuid::now_v7().to_string())
            .bind(first.to_string())
            .bind(Uuid::now_v7().to_string())
            .execute(&repository.writer)
            .await
            .expect("metadata");
        (temp, storage, repository, root_id, first, second)
    }

    fn query() -> StoredTrackCatalogQuery {
        StoredTrackCatalogQuery {
            offset: 0,
            limit: 200,
            query: None,
            sort: StoredTrackSort::Title,
            availability: None,
            match_status: None,
        }
    }

    #[tokio::test]
    async fn list_tracks_is_path_free_maps_enrichment_and_escapes_search_wildcards() {
        let (_temp, storage, repository, _root, first, _second) = fixture().await;
        let page = repository
            .list_track_catalog(query())
            .await
            .expect("catalogue");
        assert_eq!(page.items.len(), 2);
        let enriched = page
            .items
            .iter()
            .find(|item| item.track_id == first)
            .expect("first");
        assert_eq!(enriched.match_status, StoredTrackMatchStatus::Review);
        assert_eq!(
            enriched.enriched.as_ref().expect("enrichment").fetched_at,
            "1970-01-01T00:00:01.000Z"
        );
        assert!(
            !serde_json::to_string(&format!("{page:?}"))
                .expect("debug JSON")
                .contains("one.wav")
        );

        let wildcard = repository
            .list_track_catalog(StoredTrackCatalogQuery {
                query: Some("100%".to_owned()),
                ..query()
            })
            .await
            .expect("literal wildcard search");
        assert_eq!(wildcard.items.len(), 1);
        assert_eq!(wildcard.items[0].track_id, first);
        storage.close().await;
    }

    #[tokio::test]
    async fn list_tracks_combines_filters_and_returns_bounded_cursor_offset() {
        let (_temp, storage, repository, _root, first, _second) = fixture().await;
        let page = repository
            .list_track_catalog(StoredTrackCatalogQuery {
                limit: 1,
                availability: Some(StoredTrackAvailabilityFilter::Playable),
                match_status: Some(StoredTrackMatchFilter::Review),
                ..query()
            })
            .await
            .expect("filtered catalogue");
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].track_id, first);
        assert_eq!(page.next_offset, None);
        storage.close().await;
    }
}
