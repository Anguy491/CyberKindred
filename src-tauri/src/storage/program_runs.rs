//! Transactional persistence for validated program plans and runner state.

use crate::contracts::{ContractRegistry, ProgramPlan, ProgramPlanMode, ProgramPlanSegmentsItem};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{Repository, StorageError, StorageReason};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StoredProgramStatus {
    Planning,
    Ready,
    Music,
    VoicePreparing,
    Voice,
    Paused,
    Completing,
    Stopping,
    Completed,
    Failed,
}

impl StoredProgramStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Planning => "planning",
            Self::Ready => "ready",
            Self::Music => "music",
            Self::VoicePreparing => "voice_preparing",
            Self::Voice => "voice",
            Self::Paused => "paused",
            Self::Completing => "completing",
            Self::Stopping => "stopping",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StoredProgramSegmentStatus {
    Planned,
    Preparing,
    Active,
    Completed,
    Skipped,
    Failed,
    Cancelled,
}

impl StoredProgramSegmentStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Preparing => "preparing",
            Self::Active => "active",
            Self::Completed => "completed",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl Repository {
    /// Persists the identity of a confirmed start before provider planning.
    pub(crate) async fn begin_local_program(
        &self,
        program_id: Uuid,
        created_at_ms: i64,
    ) -> Result<(), StorageError> {
        if program_id.get_version_num() != 7 || created_at_ms < 0 {
            return Err(invalid_write());
        }
        sqlx::query("INSERT INTO program_runs(id, source_kind, status, plan_schema_version, created_at_ms) VALUES(?, 'local', 'planning', 1, ?)")
            .bind(program_id.to_string())
            .bind(created_at_ms)
            .execute(&self.writer)
            .await
            .map(|_| ())
            .map_err(|_| invalid_write())
    }

    /// Persists a confirmed Apple companion run without creating a remote queue.
    pub(crate) async fn begin_system_program(
        &self,
        program_id: Uuid,
        created_at_ms: i64,
    ) -> Result<(), StorageError> {
        if program_id.get_version_num() != 7 || created_at_ms < 0 {
            return Err(invalid_write());
        }
        sqlx::query("INSERT INTO program_runs(id, source_kind, status, plan_schema_version, created_at_ms) VALUES(?, 'system_session', 'planning', 1, ?)")
            .bind(program_id.to_string())
            .bind(created_at_ms)
            .execute(&self.writer)
            .await
            .map(|_| ())
            .map_err(|_| invalid_write())
    }

    /// Atomically installs a schema-valid local plan and all of its segments.
    pub(crate) async fn persist_local_program_plan(
        &self,
        plan: &ProgramPlan,
        degraded_features: &[&str],
        provider: Option<&str>,
        model: Option<&str>,
    ) -> Result<(), StorageError> {
        validate_persisted_plan(plan, degraded_features, provider, model)?;
        let degraded_features_json =
            serde_json::to_string(degraded_features).map_err(|_| invalid_write())?;
        let mut transaction = self.writer.begin().await.map_err(|_| invalid_write())?;
        for (ordinal, segment) in plan.segments.iter().enumerate() {
            let ordinal = i64::try_from(ordinal).map_err(|_| invalid_write())?;
            match segment {
                ProgramPlanSegmentsItem::TrackSegment(track) => {
                    sqlx::query("INSERT INTO program_segments(id, program_run_id, ordinal, kind, track_id, status) VALUES(?, ?, ?, 'track', ?, 'planned')")
                        .bind(&track.segment_id)
                        .bind(&plan.program_id)
                        .bind(ordinal)
                        .bind(&track.track_id)
                        .execute(&mut *transaction)
                        .await
                        .map_err(|_| invalid_write())?;
                }
                ProgramPlanSegmentsItem::VoiceSegment(voice) => {
                    let voice_hash = hex::encode(Sha256::digest(voice.text.as_bytes()));
                    sqlx::query("INSERT INTO program_segments(id, program_run_id, ordinal, kind, voice_text, voice_text_hash, status) VALUES(?, ?, ?, 'voice', ?, ?, 'planned')")
                        .bind(&voice.segment_id)
                        .bind(&plan.program_id)
                        .bind(ordinal)
                        .bind(&voice.text)
                        .bind(voice_hash)
                        .execute(&mut *transaction)
                        .await
                        .map_err(|_| invalid_write())?;
                }
            }
        }
        let update = sqlx::query("UPDATE program_runs SET status = 'ready', degraded_features_json = ?, provider = ?, model = ?, revision = revision + 1 WHERE id = ? AND status = 'planning'")
            .bind(degraded_features_json)
            .bind(provider)
            .bind(model)
            .bind(&plan.program_id)
            .execute(&mut *transaction)
            .await
            .map_err(|_| invalid_write())?;
        if update.rows_affected() != 1 {
            let _ = transaction.rollback().await;
            return Err(StorageError::new(StorageReason::RevisionConflict));
        }
        transaction.commit().await.map_err(|_| invalid_write())
    }

    /// Applies an explicit state transition and increments the authoritative revision.
    pub(crate) async fn transition_program(
        &self,
        program_id: Uuid,
        expected: StoredProgramStatus,
        next: StoredProgramStatus,
        occurred_at_ms: i64,
        failure_code: Option<&str>,
    ) -> Result<(), StorageError> {
        if program_id.get_version_num() != 7
            || occurred_at_ms < 0
            || failure_code.is_some_and(|value| !safe_reason(value))
        {
            return Err(invalid_write());
        }
        let terminal = matches!(
            next,
            StoredProgramStatus::Completed | StoredProgramStatus::Failed
        );
        let started = matches!(
            next,
            StoredProgramStatus::Music
                | StoredProgramStatus::VoicePreparing
                | StoredProgramStatus::Voice
        );
        let update = sqlx::query("UPDATE program_runs SET status = ?, started_at_ms = CASE WHEN ? AND started_at_ms IS NULL THEN ? ELSE started_at_ms END, ended_at_ms = CASE WHEN ? THEN ? ELSE ended_at_ms END, failure_code = ?, revision = revision + 1 WHERE id = ? AND status = ?")
            .bind(next.as_str())
            .bind(started)
            .bind(occurred_at_ms)
            .bind(terminal)
            .bind(occurred_at_ms)
            .bind(failure_code)
            .bind(program_id.to_string())
            .bind(expected.as_str())
            .execute(&self.writer)
            .await
            .map_err(|_| invalid_write())?;
        if update.rows_affected() == 1 {
            Ok(())
        } else {
            Err(StorageError::new(StorageReason::RevisionConflict))
        }
    }

    /// Updates one segment only from its expected state.
    pub(crate) async fn transition_program_segment(
        &self,
        program_id: Uuid,
        segment_id: Uuid,
        expected: StoredProgramSegmentStatus,
        next: StoredProgramSegmentStatus,
        occurred_at_ms: i64,
        failure_code: Option<&str>,
    ) -> Result<(), StorageError> {
        if program_id.get_version_num() != 7
            || segment_id.get_version_num() != 7
            || occurred_at_ms < 0
            || failure_code.is_some_and(|value| !safe_reason(value))
        {
            return Err(invalid_write());
        }
        let started = matches!(
            next,
            StoredProgramSegmentStatus::Preparing | StoredProgramSegmentStatus::Active
        );
        let terminal = matches!(
            next,
            StoredProgramSegmentStatus::Completed
                | StoredProgramSegmentStatus::Skipped
                | StoredProgramSegmentStatus::Failed
                | StoredProgramSegmentStatus::Cancelled
        );
        let update = sqlx::query("UPDATE program_segments SET status = ?, started_at_ms = CASE WHEN ? AND started_at_ms IS NULL THEN ? ELSE started_at_ms END, ended_at_ms = CASE WHEN ? THEN ? ELSE ended_at_ms END, failure_code = ? WHERE id = ? AND program_run_id = ? AND status = ?")
            .bind(next.as_str())
            .bind(started)
            .bind(occurred_at_ms)
            .bind(terminal)
            .bind(occurred_at_ms)
            .bind(failure_code)
            .bind(segment_id.to_string())
            .bind(program_id.to_string())
            .bind(expected.as_str())
            .execute(&self.writer)
            .await
            .map_err(|_| invalid_write())?;
        if update.rows_affected() == 1 {
            Ok(())
        } else {
            Err(StorageError::new(StorageReason::RevisionConflict))
        }
    }

    /// Marks non-terminal runs interrupted and cancels unfinished segments after restart.
    pub(crate) async fn recover_interrupted_programs(
        &self,
        occurred_at_ms: i64,
    ) -> Result<u64, StorageError> {
        if occurred_at_ms < 0 {
            return Err(invalid_write());
        }
        let mut transaction = self.writer.begin().await.map_err(|_| invalid_write())?;
        sqlx::query("UPDATE program_segments SET status = 'cancelled', ended_at_ms = ?, failure_code = 'interrupted' WHERE status IN ('planned', 'preparing', 'active') AND program_run_id IN (SELECT id FROM program_runs WHERE status NOT IN ('completed', 'interrupted', 'failed'))")
            .bind(occurred_at_ms)
            .execute(&mut *transaction)
            .await
            .map_err(|_| invalid_write())?;
        let update = sqlx::query("UPDATE program_runs SET status = 'interrupted', ended_at_ms = ?, failure_code = 'interrupted', revision = revision + 1 WHERE status NOT IN ('completed', 'interrupted', 'failed')")
            .bind(occurred_at_ms)
            .execute(&mut *transaction)
            .await
            .map_err(|_| invalid_write())?;
        transaction.commit().await.map_err(|_| invalid_write())?;
        Ok(update.rows_affected())
    }
}

fn validate_persisted_plan(
    plan: &ProgramPlan,
    degraded_features: &[&str],
    provider: Option<&str>,
    model: Option<&str>,
) -> Result<(), StorageError> {
    let document = serde_json::to_value(plan).map_err(|_| invalid_write())?;
    ContractRegistry::new()
        .and_then(|registry| registry.validate("program-plan", &document))
        .map_err(|_| invalid_write())?;
    if plan.mode != ProgramPlanMode::Local
        || plan.source_id != "local"
        || !Uuid::parse_str(&plan.program_id)
            .is_ok_and(|id| id.get_version_num() == 7 && id.to_string() == plan.program_id)
        || plan.segments.iter().any(|segment| match segment {
            ProgramPlanSegmentsItem::TrackSegment(track) => {
                !valid_uuid_v7(&track.segment_id) || !valid_uuid_v7(&track.track_id)
            }
            ProgramPlanSegmentsItem::VoiceSegment(voice) => !valid_uuid_v7(&voice.segment_id),
        })
        || degraded_features.len() > 16
        || degraded_features.iter().any(|value| !safe_reason(value))
        || provider.is_some() != model.is_some()
        || provider.is_some_and(|value| !safe_identifier(value))
        || model.is_some_and(|value| !safe_identifier(value))
    {
        return Err(invalid_write());
    }
    Ok(())
}

fn valid_uuid_v7(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|id| id.get_version_num() == 7 && id.to_string() == value)
}

fn safe_reason(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
}

fn safe_identifier(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

const fn invalid_write() -> StorageError {
    StorageError::new(StorageReason::StorageWriteFailed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        contracts::{
            ProgramPlanTrackSegment, ProgramPlanVoiceSegment, ProgramPlanVoiceSegmentTrigger,
        },
        storage::{AppPaths, Storage},
    };

    async fn fixture() -> (tempfile::TempDir, Storage, Repository, Uuid) {
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
        let track_id = Uuid::now_v7();
        sqlx::query("INSERT INTO tracks(id, root_id, relative_path, relative_path_key, availability, format, file_size_bytes, modified_at_ms, duration_ms, genre_json, metadata_confidence, created_at_ms, updated_at_ms) VALUES(?, ?, 'fixture.wav', 'fixture.wav', 'available', 'wav', 1, 1, 60000, '[]', 1.0, 1, 1)")
            .bind(track_id.to_string())
            .bind(root_id.to_string())
            .execute(&repository.writer)
            .await
            .expect("track");
        (temp, storage, repository, track_id)
    }

    fn plan(program_id: Uuid, track_id: Uuid) -> ProgramPlan {
        ProgramPlan {
            schema_version: "1.0.0".to_owned(),
            program_id: program_id.to_string(),
            source_id: "local".to_owned(),
            mode: ProgramPlanMode::Local,
            created_at: "2026-09-03T01:02:03.000Z".to_owned(),
            segments: vec![
                ProgramPlanSegmentsItem::VoiceSegment(ProgramPlanVoiceSegment {
                    r#type: "voice".to_owned(),
                    segment_id: Uuid::now_v7().to_string(),
                    text: "欢迎回来。".to_owned(),
                    trigger: ProgramPlanVoiceSegmentTrigger::Opening,
                }),
                ProgramPlanSegmentsItem::TrackSegment(ProgramPlanTrackSegment {
                    r#type: "track".to_owned(),
                    segment_id: Uuid::now_v7().to_string(),
                    track_id: track_id.to_string(),
                    segue_text: None,
                }),
            ],
        }
    }

    #[tokio::test]
    async fn program_run_plan_persistence_is_atomic_and_state_checked() {
        let (_temp, storage, repository, track_id) = fixture().await;
        let program_id = Uuid::now_v7();
        repository
            .begin_local_program(program_id, 10)
            .await
            .expect("planning run");
        let missing = plan(program_id, Uuid::now_v7());
        assert!(
            repository
                .persist_local_program_plan(&missing, &["llm"], None, None)
                .await
                .is_err()
        );
        let after_rollback: (String, i64) = sqlx::query_as(
            "SELECT status, (SELECT count(*) FROM program_segments WHERE program_run_id = ?) FROM program_runs WHERE id = ?",
        )
        .bind(program_id.to_string())
        .bind(program_id.to_string())
        .fetch_one(&repository.writer)
        .await
        .expect("rollback facts");
        assert_eq!(after_rollback, ("planning".to_owned(), 0));

        let valid = plan(program_id, track_id);
        repository
            .persist_local_program_plan(&valid, &["llm", "tts"], None, None)
            .await
            .expect("valid plan");
        let persisted: (String, i64, String) = sqlx::query_as(
            "SELECT status, revision, degraded_features_json FROM program_runs WHERE id = ?",
        )
        .bind(program_id.to_string())
        .fetch_one(&repository.writer)
        .await
        .expect("run facts");
        assert_eq!(
            persisted,
            ("ready".to_owned(), 1, r#"["llm","tts"]"#.to_owned())
        );
        let segment_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM program_segments WHERE program_run_id = ?")
                .bind(program_id.to_string())
                .fetch_one(&repository.writer)
                .await
                .expect("segments");
        assert_eq!(segment_count, 2);
        storage.close().await;
    }

    #[tokio::test]
    async fn apple_program_run_persists_system_mode_without_queue_segments() {
        let (_temp, storage, repository, _track_id) = fixture().await;
        let program_id = Uuid::now_v7();
        repository
            .begin_system_program(program_id, 10)
            .await
            .expect("system companion run");
        let persisted: (String, String, i64) = sqlx::query_as(
            "SELECT source_kind, status, (SELECT count(*) FROM program_segments WHERE program_run_id = ?) FROM program_runs WHERE id = ?",
        )
        .bind(program_id.to_string())
        .bind(program_id.to_string())
        .fetch_one(&repository.writer)
        .await
        .expect("system run facts");
        assert_eq!(persisted, ("system_session".to_owned(), "planning".to_owned(), 0));
        storage.close().await;
    }

    #[tokio::test]
    async fn program_run_recovery_cancels_unfinished_segments_and_is_idempotent() {
        let (_temp, storage, repository, track_id) = fixture().await;
        let program_id = Uuid::now_v7();
        repository
            .begin_local_program(program_id, 10)
            .await
            .expect("planning run");
        let valid = plan(program_id, track_id);
        repository
            .persist_local_program_plan(&valid, &[], None, None)
            .await
            .expect("valid plan");
        repository
            .transition_program(
                program_id,
                StoredProgramStatus::Ready,
                StoredProgramStatus::Music,
                20,
                None,
            )
            .await
            .expect("music transition");
        assert_eq!(
            repository
                .recover_interrupted_programs(30)
                .await
                .expect("first recovery"),
            1
        );
        assert_eq!(
            repository
                .recover_interrupted_programs(40)
                .await
                .expect("idempotent recovery"),
            0
        );
        let run: (String, Option<i64>, Option<String>) = sqlx::query_as(
            "SELECT status, ended_at_ms, failure_code FROM program_runs WHERE id = ?",
        )
        .bind(program_id.to_string())
        .fetch_one(&repository.writer)
        .await
        .expect("recovered run");
        assert_eq!(
            run,
            (
                "interrupted".to_owned(),
                Some(30),
                Some("interrupted".to_owned())
            )
        );
        let unfinished: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM program_segments WHERE program_run_id = ? AND status != 'cancelled'",
        )
        .bind(program_id.to_string())
        .fetch_one(&repository.writer)
        .await
        .expect("recovered segments");
        assert_eq!(unfinished, 0);
        storage.close().await;
    }
}
