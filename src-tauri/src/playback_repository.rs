//! Repository-backed authorization snapshot for the synchronous playback actor.

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use crate::{
    playback::{LocalSourceError, LocalTrack, LocalTrackResolver},
    storage::{
        Repository, StorageError, StorageReason, StoredPlaybackTrack,
        resolve_read_only_library_path,
    },
};

#[derive(Clone)]
pub(crate) struct RepositoryTrackResolver {
    repository: Repository,
    facts: Arc<RwLock<HashMap<String, StoredPlaybackTrack>>>,
}

impl RepositoryTrackResolver {
    pub(crate) fn new(repository: Repository) -> Self {
        Self {
            repository,
            facts: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Replaces the actor's bounded authorization snapshot immediately before
    /// a program queue is installed. Paths remain Rust-only.
    pub(crate) async fn refresh(&self, track_ids: &[String]) -> Result<(), StorageError> {
        let tracks = self.repository.load_playback_tracks(track_ids).await?;
        let next = tracks
            .into_iter()
            .map(|track| (track.track_id.clone(), track))
            .collect();
        *self
            .facts
            .write()
            .map_err(|_| StorageError::new(StorageReason::StorageWriteFailed))? = next;
        Ok(())
    }

    pub(crate) fn clear(&self) {
        if let Ok(mut facts) = self.facts.write() {
            facts.clear();
        }
    }
}

impl LocalTrackResolver for RepositoryTrackResolver {
    fn resolve(&self, track_id: &str) -> Result<LocalTrack, LocalSourceError> {
        let facts = self
            .facts
            .read()
            .map_err(|_| LocalSourceError::Unavailable)?;
        let fact = facts.get(track_id).ok_or(LocalSourceError::NotFound)?;
        let canonical_path =
            resolve_read_only_library_path(&fact.authorized_root, &fact.relative_path)
                .map_err(|_| LocalSourceError::PathDenied)?;
        if !canonical_path.is_file() {
            return Err(LocalSourceError::MediaInvalid);
        }
        LocalTrack::new(
            fact.track_id.clone(),
            fact.title.clone(),
            fact.artist.clone(),
            fact.album.clone(),
            None,
            canonical_path,
            fact.duration_ms,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{AppPaths, Storage};
    use uuid::Uuid;

    #[tokio::test]
    async fn resolver_rechecks_the_authorized_path_on_every_resolution() {
        let temp = tempfile::tempdir().expect("temporary root");
        let music = temp.path().join("Music");
        std::fs::create_dir_all(&music).expect("music root");
        let file = music.join("track.wav");
        std::fs::write(&file, b"fixture").expect("fixture");
        let paths = AppPaths::create(
            temp.path().join("data"),
            temp.path().join("cache"),
            temp.path().join("logs"),
        )
        .expect("paths");
        let storage = Storage::open(&paths, "0.3.0").await.expect("storage");
        let resolver = RepositoryTrackResolver::new(storage.repository());
        let track_id = Uuid::now_v7().to_string();
        resolver.facts.write().expect("facts").insert(
            track_id.clone(),
            StoredPlaybackTrack {
                track_id: track_id.clone(),
                authorized_root: music,
                relative_path: "track.wav".into(),
                title: "Fixture".to_owned(),
                artist: None,
                album: None,
                duration_ms: 60_000,
            },
        );
        assert_eq!(
            resolver
                .resolve(&track_id)
                .expect("first resolution")
                .track_id(),
            track_id
        );
        std::fs::remove_file(file).expect("remove source");
        assert_eq!(
            resolver.resolve(&track_id).err(),
            Some(LocalSourceError::PathDenied)
        );
        storage.close().await;
    }
}
