use super::{Repository, StorageError, StorageReason};

impl Repository {
    /// Persists the observed pre-suspend edge without changing unrelated OS
    /// integration state.
    pub(crate) async fn record_power_suspend(
        &self,
        observed_at_ms: i64,
    ) -> Result<(), StorageError> {
        if observed_at_ms < 0 {
            return Err(StorageError::new(StorageReason::StorageWriteFailed));
        }
        sqlx::query(
            "INSERT INTO os_integration_state(id, notifications_permission, autostart_enabled, tray_enabled, last_suspend_at_ms, last_resume_at_ms, scheduler_checked_at_ms, updated_at_ms) VALUES('current', 'unknown', 0, 0, ?, NULL, NULL, ?) ON CONFLICT(id) DO UPDATE SET last_suspend_at_ms = excluded.last_suspend_at_ms, updated_at_ms = excluded.updated_at_ms",
        )
        .bind(observed_at_ms)
        .bind(observed_at_ms)
        .execute(&self.writer)
        .await
        .map_err(|_| StorageError::new(StorageReason::StorageWriteFailed))?;
        Ok(())
    }

    /// Persists a reconciled resume and scheduler check, returning the latest
    /// observed suspend edge for EVT-010.
    pub(crate) async fn record_power_resume(
        &self,
        observed_at_ms: i64,
    ) -> Result<Option<i64>, StorageError> {
        if observed_at_ms < 0 {
            return Err(StorageError::new(StorageReason::StorageWriteFailed));
        }
        let mut transaction = self
            .writer
            .begin()
            .await
            .map_err(|_| StorageError::new(StorageReason::StorageWriteFailed))?;
        sqlx::query(
            "INSERT INTO os_integration_state(id, notifications_permission, autostart_enabled, tray_enabled, last_suspend_at_ms, last_resume_at_ms, scheduler_checked_at_ms, updated_at_ms) VALUES('current', 'unknown', 0, 0, NULL, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET last_resume_at_ms = excluded.last_resume_at_ms, scheduler_checked_at_ms = excluded.scheduler_checked_at_ms, updated_at_ms = excluded.updated_at_ms",
        )
        .bind(observed_at_ms)
        .bind(observed_at_ms)
        .bind(observed_at_ms)
        .execute(&mut *transaction)
        .await
        .map_err(|_| StorageError::new(StorageReason::StorageWriteFailed))?;
        let slept_at_ms = sqlx::query_scalar::<_, Option<i64>>(
            "SELECT last_suspend_at_ms FROM os_integration_state WHERE id = 'current'",
        )
        .fetch_one(&mut *transaction)
        .await
        .map_err(|_| StorageError::new(StorageReason::StorageReadFailed))?;
        transaction
            .commit()
            .await
            .map_err(|_| StorageError::new(StorageReason::StorageWriteFailed))?;
        Ok(slept_at_ms)
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::{AppPaths, Storage};

    #[tokio::test]
    async fn power_lifecycle_persists_suspend_resume_and_scheduler_check() {
        let temp = tempfile::tempdir().expect("temporary root");
        let paths = AppPaths::create(
            temp.path().join("data"),
            temp.path().join("cache"),
            temp.path().join("logs"),
        )
        .expect("paths");
        let storage = Storage::open(&paths, "0.1.0").await.expect("storage");
        let repository = storage.repository();
        repository
            .record_power_suspend(1_000)
            .await
            .expect("suspend");
        assert_eq!(
            repository.record_power_resume(2_000).await.expect("resume"),
            Some(1_000)
        );
        let row = sqlx::query_as::<_, (Option<i64>, Option<i64>, Option<i64>, i64)>(
            "SELECT last_suspend_at_ms, last_resume_at_ms, scheduler_checked_at_ms, updated_at_ms FROM os_integration_state WHERE id = 'current'",
        )
        .fetch_one(&repository.writer)
        .await
        .expect("lifecycle row");
        assert_eq!(row, (Some(1_000), Some(2_000), Some(2_000), 2_000));
        storage.close().await;
    }
}
