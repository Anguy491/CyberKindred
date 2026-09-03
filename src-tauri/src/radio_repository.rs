//! Path-free repository context adapter for local radio planning.

use std::{future::Future, pin::Pin, sync::Arc};

use chrono::{Local, Timelike};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    ipc::{ApiError, InternalReason},
    radio::{ProgramPlannerContextSource, RadioPlanningContext},
    storage::{Repository, StorageError, StorageReason},
};

const PROGRAM_COOLDOWN_MS: i64 = 30 * 24 * 60 * 60 * 1_000;
const SEED_DOMAIN: &[u8] = b"cyberkindred:local-program-selection:v1";

trait LocalHourSource: Send + Sync {
    fn local_hour(&self) -> u8;
}

#[derive(Clone, Copy, Debug, Default)]
struct SystemLocalHourSource;

impl LocalHourSource for SystemLocalHourSource {
    fn local_hour(&self) -> u8 {
        let Ok(hour) = u8::try_from(Local::now().hour()) else {
            return 0;
        };
        hour
    }
}

/// Production adapter from path-free repository facts to the radio planner.
pub(crate) struct RepositoryProgramContextSource {
    repository: Repository,
    local_hour: Arc<dyn LocalHourSource>,
}

impl RepositoryProgramContextSource {
    #[must_use]
    pub(crate) fn new(repository: Repository) -> Self {
        Self {
            repository,
            local_hour: Arc::new(SystemLocalHourSource),
        }
    }

    #[cfg(test)]
    fn with_local_hour(repository: Repository, local_hour: Arc<dyn LocalHourSource>) -> Self {
        Self {
            repository,
            local_hour,
        }
    }
}

impl ProgramPlannerContextSource for RepositoryProgramContextSource {
    fn load_context(
        &self,
        program_id: Uuid,
    ) -> Pin<Box<dyn Future<Output = Result<RadioPlanningContext, ApiError>> + Send + '_>> {
        Box::pin(async move {
            let (profile_tags, approved_memory_tags, recently_played_track_ids) = self
                .repository
                .load_program_planning_facts()
                .await
                .map_err(|error| map_storage_error(&error))?;
            let local_hour = self.local_hour.local_hour();
            if local_hour > 23 {
                return Err(ApiError::from_reason(InternalReason::UnexpectedInternal));
            }
            Ok(RadioPlanningContext {
                local_hour,
                profile_tags,
                // The repository projection admits approved current revisions only.
                // Proposed, disabled and deleted memory prose cannot cross this seam.
                approved_memory_tags,
                recently_played_track_ids,
                cooldown_ms: PROGRAM_COOLDOWN_MS,
                allow_cooldown_relaxation: true,
                selection_seed: selection_seed(program_id),
            })
        })
    }
}

fn selection_seed(program_id: Uuid) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(SEED_DOMAIN);
    hasher.update(program_id.as_bytes());
    let digest = hasher.finalize();
    let mut seed = [0_u8; 8];
    seed.copy_from_slice(&digest[..8]);
    u64::from_be_bytes(seed)
}

fn map_storage_error(error: &StorageError) -> ApiError {
    let reason = match error.reason() {
        StorageReason::PathDenied => InternalReason::PathDenied,
        StorageReason::PathOutsideScope => InternalReason::PathOutsideRoot,
        StorageReason::UnsafeReparsePoint => InternalReason::UnsafeReparsePoint,
        StorageReason::StorageReadFailed => InternalReason::StorageReadFailed,
        StorageReason::StorageWriteFailed | StorageReason::InvalidSetting => {
            InternalReason::StorageWriteFailed
        }
        StorageReason::StorageIntegrityFailed | StorageReason::ForeignDatabase => {
            InternalReason::StorageIntegrityFailed
        }
        StorageReason::MigrationFailed => InternalReason::MigrationFailed,
        StorageReason::DatabaseVersionUnsupported => InternalReason::DatabaseVersionUnsupported,
        StorageReason::EntityNotFound => InternalReason::EntityNotFound,
        StorageReason::RevisionConflict => InternalReason::RevisionConflict,
        StorageReason::ResourceBusy => InternalReason::ResourceBusy,
    };
    ApiError::from_reason(reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{AppPaths, Storage};

    #[derive(Clone, Copy, Debug)]
    struct FixedLocalHour(u8);

    impl LocalHourSource for FixedLocalHour {
        fn local_hour(&self) -> u8 {
            self.0
        }
    }

    #[tokio::test]
    async fn radio_context_adapter_is_stable_bounded_and_path_free() {
        let temp = tempfile::tempdir().expect("temporary root");
        let paths = AppPaths::create(
            temp.path().join("private-data"),
            temp.path().join("private-cache"),
            temp.path().join("private-logs"),
        )
        .expect("app paths");
        let storage = Storage::open(&paths, "0.3.0").await.expect("storage");
        let source = RepositoryProgramContextSource::with_local_hour(
            storage.repository(),
            Arc::new(FixedLocalHour(21)),
        );
        let program_id = Uuid::now_v7();
        let first = source
            .load_context(program_id)
            .await
            .expect("first context");
        let second = source
            .load_context(program_id)
            .await
            .expect("second context");
        let other = source
            .load_context(Uuid::now_v7())
            .await
            .expect("other context");

        assert_eq!(first, second);
        assert_eq!(first.local_hour, 21);
        assert!(first.profile_tags.is_empty());
        assert!(first.approved_memory_tags.is_empty());
        assert!(first.recently_played_track_ids.is_empty());
        assert_eq!(first.cooldown_ms, PROGRAM_COOLDOWN_MS);
        assert!(first.allow_cooldown_relaxation);
        assert_eq!(first.selection_seed, selection_seed(program_id));
        assert_ne!(first.selection_seed, other.selection_seed);
        assert!(!format!("{first:?}").contains(temp.path().to_string_lossy().as_ref()));
        storage.close().await;
    }

    #[test]
    fn radio_context_seed_depends_only_on_the_program_uuid() {
        let program_id = Uuid::now_v7();
        let first = selection_seed(program_id);
        let second = selection_seed(program_id);
        assert_eq!(first, second);
        assert_ne!(first, selection_seed(Uuid::now_v7()));
    }
}
