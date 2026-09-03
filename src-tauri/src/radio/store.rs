use uuid::Uuid;

use crate::{
    ipc::{ApiError, InternalReason},
    program::PlannedProgram,
    storage::{
        Repository, StorageError, StorageReason, StoredProgramSegmentStatus, StoredProgramStatus,
    },
};

use super::traits::RadioFuture;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProgramRunPhase {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProgramSegmentPhase {
    Planned,
    Preparing,
    Active,
    Completed,
    Skipped,
    Failed,
    Cancelled,
}

/// Authoritative program persistence boundary. Every successful transition
/// completes before its corresponding public event or audio side effect.
pub trait RadioProgramStore: Send + Sync {
    fn begin_program(
        &self,
        program_id: Uuid,
        created_at_ms: i64,
    ) -> RadioFuture<'_, Result<u64, ApiError>>;

    fn persist_plan<'a>(
        &'a self,
        planned: &'a PlannedProgram,
    ) -> RadioFuture<'a, Result<u64, ApiError>>;

    fn transition_program(
        &self,
        program_id: Uuid,
        expected: ProgramRunPhase,
        next: ProgramRunPhase,
        current_revision: u64,
        occurred_at_ms: i64,
        failure_code: Option<&'static str>,
    ) -> RadioFuture<'_, Result<u64, ApiError>>;

    fn transition_segment(
        &self,
        program_id: Uuid,
        segment_id: Uuid,
        expected: ProgramSegmentPhase,
        next: ProgramSegmentPhase,
        occurred_at_ms: i64,
        failure_code: Option<&'static str>,
    ) -> RadioFuture<'_, Result<(), ApiError>>;

    fn voice_allowed_after_feedback(
        &self,
        _program_id: Uuid,
    ) -> RadioFuture<'_, Result<bool, ApiError>> {
        Box::pin(async { Ok(true) })
    }
}

impl RadioProgramStore for Repository {
    fn begin_program(
        &self,
        program_id: Uuid,
        created_at_ms: i64,
    ) -> RadioFuture<'_, Result<u64, ApiError>> {
        Box::pin(async move {
            self.begin_local_program(program_id, created_at_ms)
                .await
                .map_err(|error| map_storage_error(&error))?;
            Ok(0)
        })
    }

    fn persist_plan<'a>(
        &'a self,
        planned: &'a PlannedProgram,
    ) -> RadioFuture<'a, Result<u64, ApiError>> {
        Box::pin(async move {
            let mut degraded = Vec::with_capacity(2);
            if planned.degradation.is_some() {
                degraded.push("llm");
            }
            if planned.candidates.cooldown_relaxed {
                degraded.push("candidate_cooldown");
            }
            self.persist_local_program_plan(&planned.plan, &degraded, None, None)
                .await
                .map_err(|error| map_storage_error(&error))?;
            Ok(1)
        })
    }

    fn transition_program(
        &self,
        program_id: Uuid,
        expected: ProgramRunPhase,
        next: ProgramRunPhase,
        current_revision: u64,
        occurred_at_ms: i64,
        failure_code: Option<&'static str>,
    ) -> RadioFuture<'_, Result<u64, ApiError>> {
        Box::pin(async move {
            Repository::transition_program(
                self,
                program_id,
                stored_program_status(expected),
                stored_program_status(next),
                occurred_at_ms,
                failure_code,
            )
            .await
            .map_err(|error| map_storage_error(&error))?;
            current_revision
                .checked_add(1)
                .ok_or_else(ApiError::unexpected)
        })
    }

    fn transition_segment(
        &self,
        program_id: Uuid,
        segment_id: Uuid,
        expected: ProgramSegmentPhase,
        next: ProgramSegmentPhase,
        occurred_at_ms: i64,
        failure_code: Option<&'static str>,
    ) -> RadioFuture<'_, Result<(), ApiError>> {
        Box::pin(async move {
            self.transition_program_segment(
                program_id,
                segment_id,
                stored_segment_status(expected),
                stored_segment_status(next),
                occurred_at_ms,
                failure_code,
            )
            .await
            .map_err(|error| map_storage_error(&error))
        })
    }

    fn voice_allowed_after_feedback(
        &self,
        program_id: Uuid,
    ) -> RadioFuture<'_, Result<bool, ApiError>> {
        Box::pin(async move {
            Repository::voice_allowed_after_feedback(self, program_id)
                .await
                .map_err(|error| map_storage_error(&error))
        })
    }
}

const fn stored_program_status(value: ProgramRunPhase) -> StoredProgramStatus {
    match value {
        ProgramRunPhase::Planning => StoredProgramStatus::Planning,
        ProgramRunPhase::Ready => StoredProgramStatus::Ready,
        ProgramRunPhase::Music => StoredProgramStatus::Music,
        ProgramRunPhase::VoicePreparing => StoredProgramStatus::VoicePreparing,
        ProgramRunPhase::Voice => StoredProgramStatus::Voice,
        ProgramRunPhase::Paused => StoredProgramStatus::Paused,
        ProgramRunPhase::Completing => StoredProgramStatus::Completing,
        ProgramRunPhase::Stopping => StoredProgramStatus::Stopping,
        ProgramRunPhase::Completed => StoredProgramStatus::Completed,
        ProgramRunPhase::Failed => StoredProgramStatus::Failed,
    }
}

const fn stored_segment_status(value: ProgramSegmentPhase) -> StoredProgramSegmentStatus {
    match value {
        ProgramSegmentPhase::Planned => StoredProgramSegmentStatus::Planned,
        ProgramSegmentPhase::Preparing => StoredProgramSegmentStatus::Preparing,
        ProgramSegmentPhase::Active => StoredProgramSegmentStatus::Active,
        ProgramSegmentPhase::Completed => StoredProgramSegmentStatus::Completed,
        ProgramSegmentPhase::Skipped => StoredProgramSegmentStatus::Skipped,
        ProgramSegmentPhase::Failed => StoredProgramSegmentStatus::Failed,
        ProgramSegmentPhase::Cancelled => StoredProgramSegmentStatus::Cancelled,
    }
}

fn map_storage_error(error: &StorageError) -> ApiError {
    let reason = match error.reason() {
        StorageReason::StorageReadFailed => InternalReason::StorageReadFailed,
        StorageReason::StorageIntegrityFailed | StorageReason::ForeignDatabase => {
            InternalReason::StorageIntegrityFailed
        }
        StorageReason::MigrationFailed => InternalReason::MigrationFailed,
        StorageReason::DatabaseVersionUnsupported => InternalReason::DatabaseVersionUnsupported,
        StorageReason::EntityNotFound => InternalReason::EntityNotFound,
        StorageReason::RevisionConflict => InternalReason::RevisionConflict,
        StorageReason::ResourceBusy => InternalReason::ResourceBusy,
        StorageReason::PathDenied => InternalReason::PathDenied,
        StorageReason::PathOutsideScope => InternalReason::PathOutsideRoot,
        StorageReason::UnsafeReparsePoint => InternalReason::UnsafeReparsePoint,
        StorageReason::StorageWriteFailed | StorageReason::InvalidSetting => {
            InternalReason::StorageWriteFailed
        }
    };
    ApiError::from_reason(reason)
}
