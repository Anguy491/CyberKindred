use super::{Repository, RetentionResult, StorageError, StorageReason};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const MAX_RETENTION_BATCH: u32 = 500;
const DAY_MS: i64 = 24 * 60 * 60 * 1_000;
const DELIVERED_OUTBOX_RETENTION_MS: i64 = DAY_MS;
const UNDELIVERED_OUTBOX_RETENTION_MS: i64 = 7 * DAY_MS;

impl Repository {
    /// Runs one absolute-time retention batch.
    ///
    /// Callers run this at startup and at least daily. It is intentionally
    /// stateless: a surfaced failure can be retried on the next startup without
    /// advancing or extending any stored expiry timestamp.
    ///
    /// # Errors
    ///
    /// Returns a stable storage error and rolls back the whole batch if summary
    /// generation, content deletion, outbox expiry, or commit fails.
    pub async fn run_retention_batch(
        &self,
        now_ms: i64,
        requested_limit: u32,
    ) -> Result<RetentionResult, StorageError> {
        let limit = requested_limit.clamp(1, MAX_RETENTION_BATCH);
        let mut transaction = self
            .writer
            .begin()
            .await
            .map_err(|_| StorageError::new(StorageReason::StorageWriteFailed))?;

        let sessions: Vec<(String, i64, i64, i64, i64)> = sqlx::query_as(
            "SELECT session.id, session.covered_from_ms, session.covered_to_ms, session.user_message_count, session.assistant_message_count FROM chat_sessions AS session WHERE session.covered_from_ms IS NOT NULL AND session.covered_to_ms IS NOT NULL AND EXISTS (SELECT 1 FROM messages WHERE messages.chat_session_id = session.id AND messages.expires_at_ms <= ?) AND NOT EXISTS (SELECT 1 FROM session_summaries WHERE session_summaries.chat_session_id = session.id) ORDER BY session.covered_from_ms, session.id LIMIT ?",
        )
        .bind(now_ms)
        .bind(limit)
        .fetch_all(&mut *transaction)
        .await
        .map_err(|_| StorageError::new(StorageReason::StorageReadFailed))?;

        let mut summaries_created = 0_u64;
        for (session_id, covered_from, covered_to, user_count, assistant_count) in sessions {
            let summary =
                format!("会话包含 {user_count} 条用户消息和 {assistant_count} 条助手消息。");
            let source_hash = retention_source_hash(
                &session_id,
                covered_from,
                covered_to,
                user_count,
                assistant_count,
            );
            sqlx::query(
                "INSERT INTO session_summaries(id, chat_session_id, status, summary_text, source_from_ms, source_to_ms, source_hash, generation_kind, preference_signals_json, prompt_version, revision, created_at_ms, updated_at_ms) VALUES(?, ?, 'active', ?, ?, ?, ?, 'deterministic', '[]', 'retention-v1', 1, ?, ?)",
            )
            .bind(Uuid::now_v7().to_string())
            .bind(session_id)
            .bind(summary)
            .bind(covered_from)
            .bind(covered_to)
            .bind(source_hash)
            .bind(now_ms)
            .bind(now_ms)
            .execute(&mut *transaction)
            .await
            .map_err(|_| StorageError::new(StorageReason::StorageWriteFailed))?;
            summaries_created = summaries_created.saturating_add(1);
        }

        let messages_deleted = sqlx::query(
            "DELETE FROM messages WHERE id IN (SELECT id FROM messages WHERE expires_at_ms <= ? ORDER BY expires_at_ms, id LIMIT ?)",
        )
        .bind(now_ms)
        .bind(limit)
        .execute(&mut *transaction)
        .await
        .map_err(|_| StorageError::new(StorageReason::StorageWriteFailed))?
        .rows_affected();

        let remaining = u64::from(limit)
            .saturating_sub(messages_deleted)
            .min(u64::from(u32::MAX));
        let remaining = i64::try_from(remaining)
            .map_err(|_| StorageError::new(StorageReason::StorageWriteFailed))?;
        let voice_text_cleared = if remaining == 0 {
            0
        } else {
            sqlx::query(
                "UPDATE program_segments SET voice_text = NULL WHERE id IN (SELECT id FROM program_segments WHERE kind = 'voice' AND voice_text IS NOT NULL AND ended_at_ms IS NOT NULL AND ended_at_ms + 2592000000 <= ? ORDER BY ended_at_ms, id LIMIT ?)",
            )
            .bind(now_ms)
            .bind(remaining)
            .execute(&mut *transaction)
            .await
            .map_err(|_| StorageError::new(StorageReason::StorageWriteFailed))?
            .rows_affected()
        };

        let processed = messages_deleted.saturating_add(voice_text_cleared);
        let remaining = u64::from(limit)
            .saturating_sub(processed)
            .min(u64::from(u32::MAX));
        let remaining = i64::try_from(remaining)
            .map_err(|_| StorageError::new(StorageReason::StorageWriteFailed))?;
        let (delivered_outbox_deleted, expired_undelivered_outbox_deleted) =
            delete_expired_outbox(&mut transaction, now_ms, remaining).await?;

        transaction
            .commit()
            .await
            .map_err(|_| StorageError::new(StorageReason::StorageWriteFailed))?;
        Ok(RetentionResult {
            messages_deleted,
            summaries_created,
            voice_text_cleared,
            delivered_outbox_deleted,
            expired_undelivered_outbox_deleted,
        })
    }
}

async fn delete_expired_outbox(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    now_ms: i64,
    limit: i64,
) -> Result<(u64, u64), StorageError> {
    if limit == 0 {
        return Ok((0, 0));
    }
    let delivered_cutoff = now_ms.saturating_sub(DELIVERED_OUTBOX_RETENTION_MS);
    let undelivered_cutoff = now_ms.saturating_sub(UNDELIVERED_OUTBOX_RETENTION_MS);
    let candidates: Vec<(String, bool)> = sqlx::query_as(
        "SELECT id, delivered_at_ms IS NULL FROM outbox_events WHERE (delivered_at_ms IS NOT NULL AND delivered_at_ms <= ?) OR (delivered_at_ms IS NULL AND created_at_ms <= ?) ORDER BY CASE WHEN delivered_at_ms IS NULL THEN created_at_ms ELSE delivered_at_ms END, id LIMIT ?",
    )
    .bind(delivered_cutoff)
    .bind(undelivered_cutoff)
    .bind(limit)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|_| StorageError::new(StorageReason::StorageReadFailed))?;
    let delivered = candidates.iter().filter(|(_, pending)| !pending).count();
    let undelivered = candidates.len().saturating_sub(delivered);
    for (id, _) in candidates {
        sqlx::query("DELETE FROM outbox_events WHERE id = ?")
            .bind(id)
            .execute(&mut **transaction)
            .await
            .map_err(|_| StorageError::new(StorageReason::StorageWriteFailed))?;
    }
    Ok((
        u64::try_from(delivered)
            .map_err(|_| StorageError::new(StorageReason::StorageWriteFailed))?,
        u64::try_from(undelivered)
            .map_err(|_| StorageError::new(StorageReason::StorageWriteFailed))?,
    ))
}

fn retention_source_hash(
    session_id: &str,
    covered_from: i64,
    covered_to: i64,
    user_count: i64,
    assistant_count: i64,
) -> String {
    let source = format!(
        "{session_id}\u{1f}{covered_from}\u{1f}{covered_to}\u{1f}{user_count}\u{1f}{assistant_count}"
    );
    hex::encode(Sha256::digest(source.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{AppPaths, ChatRole, NewChatMessage, Storage};

    async fn retention_fixture() -> (tempfile::TempDir, Storage, Repository) {
        let temp = tempfile::tempdir().expect("temporary root");
        let paths = AppPaths::create(
            temp.path().join("data"),
            temp.path().join("cache"),
            temp.path().join("logs"),
        )
        .expect("scoped paths");
        let storage = Storage::open(&paths, "0.1.0").await.expect("storage");
        let repository = storage.repository();
        (temp, storage, repository)
    }

    async fn insert_retention_message(repository: &Repository, session: &str, created_at: i64) {
        repository
            .create_chat_session(session, created_at)
            .await
            .expect("session");
        repository
            .insert_message(NewChatMessage::new(
                format!("message-{session}"),
                session.to_owned(),
                ChatRole::User,
                "retention boundary fixture".to_owned(),
                None,
                None,
                None,
                created_at,
            ))
            .await
            .expect("message");
    }

    #[tokio::test]
    async fn retention_keeps_29_days_23_59_and_deletes_at_exactly_30_days() {
        let (_temp, storage, repository) = retention_fixture().await;
        let created_at = 10_000;
        insert_retention_message(&repository, "boundary-session", created_at).await;

        let before = repository
            .run_retention_batch(created_at + 30 * DAY_MS - 1, 500)
            .await
            .expect("before-boundary cleanup");
        assert_eq!(before.messages_deleted, 0);
        assert_eq!(repository.message_count().await.expect("count"), 1);

        let exact = repository
            .run_retention_batch(created_at + 30 * DAY_MS, 500)
            .await
            .expect("exact-boundary cleanup");
        assert_eq!(exact.messages_deleted, 1);
        assert_eq!(exact.summaries_created, 1);
        assert_eq!(repository.message_count().await.expect("count"), 0);
        let summary: (String, String, i64, i64) = sqlx::query_as(
            "SELECT status, generation_kind, source_from_ms, source_to_ms FROM session_summaries WHERE chat_session_id = 'boundary-session'",
        )
        .fetch_one(&repository.writer)
        .await
        .expect("deterministic summary");
        assert_eq!(
            summary,
            (
                "active".to_owned(),
                "deterministic".to_owned(),
                created_at,
                created_at
            )
        );
        storage.close().await;
    }

    #[tokio::test]
    async fn retention_deleted_summary_tombstone_prevents_regeneration() {
        let (_temp, storage, repository) = retention_fixture().await;
        let created_at = 20_000;
        insert_retention_message(&repository, "deleted-summary-session", created_at).await;
        sqlx::query(
            "INSERT INTO session_summaries(id, chat_session_id, status, summary_text, source_from_ms, source_to_ms, source_hash, generation_kind, preference_signals_json, provider, model, prompt_version, revision, created_at_ms, updated_at_ms, deleted_at_ms) VALUES('summary-tombstone', 'deleted-summary-session', 'deleted', NULL, ?, ?, NULL, NULL, '[]', NULL, NULL, NULL, 2, ?, ?, ?)",
        )
        .bind(created_at)
        .bind(created_at)
        .bind(created_at)
        .bind(created_at)
        .bind(created_at)
        .execute(&repository.writer)
        .await
        .expect("tombstone fixture");

        let result = repository
            .run_retention_batch(created_at + 30 * DAY_MS, 500)
            .await
            .expect("cleanup");
        assert_eq!(result.messages_deleted, 1);
        assert_eq!(result.summaries_created, 0);
        let status: String = sqlx::query_scalar(
            "SELECT status FROM session_summaries WHERE chat_session_id = 'deleted-summary-session'",
        )
        .fetch_one(&repository.writer)
        .await
        .expect("tombstone remains");
        assert_eq!(status, "deleted");
        storage.close().await;
    }

    #[tokio::test]
    async fn retention_voice_text_uses_segment_end_absolute_time() {
        let (_temp, storage, repository) = retention_fixture().await;
        let ended_at = 50_000;
        sqlx::query(
            "INSERT INTO program_runs(id, source_kind, status, plan_schema_version, created_at_ms) VALUES('run-1', 'local', 'completed', 1, ?)",
        )
        .bind(ended_at)
        .execute(&repository.writer)
        .await
        .expect("program fixture");
        sqlx::query(
            "INSERT INTO program_segments(id, program_run_id, ordinal, kind, voice_text, voice_text_hash, status, ended_at_ms) VALUES('voice-1', 'run-1', 0, 'voice', 'voice canary', ?, 'completed', ?)",
        )
        .bind("a".repeat(64))
        .bind(ended_at)
        .execute(&repository.writer)
        .await
        .expect("voice fixture");

        let before = repository
            .run_retention_batch(ended_at + 30 * DAY_MS - 1, 500)
            .await
            .expect("before boundary");
        assert_eq!(before.voice_text_cleared, 0);
        let exact = repository
            .run_retention_batch(ended_at + 30 * DAY_MS, 500)
            .await
            .expect("exact boundary");
        assert_eq!(exact.voice_text_cleared, 1);
        let text: Option<String> =
            sqlx::query_scalar("SELECT voice_text FROM program_segments WHERE id = 'voice-1'")
                .fetch_one(&repository.writer)
                .await
                .expect("voice text");
        assert!(text.is_none());
        storage.close().await;
    }

    #[tokio::test]
    async fn retention_batch_is_capped_at_500_rows() {
        let (_temp, storage, repository) = retention_fixture().await;
        repository
            .create_chat_session("batch-session", 0)
            .await
            .expect("session");
        let mut transaction = repository
            .writer
            .begin()
            .await
            .expect("fixture transaction");
        for index in 0..501 {
            sqlx::query(
                "INSERT INTO messages(id, chat_session_id, role, content_text, content_hash, created_at_ms, expires_at_ms) VALUES(?, 'batch-session', 'user', 'fixture', ?, 0, 2592000000)",
            )
            .bind(format!("message-{index:03}"))
            .bind(format!("{index:064x}"))
            .execute(&mut *transaction)
            .await
            .expect("fixture message");
        }
        sqlx::query("UPDATE chat_sessions SET covered_from_ms = 0, covered_to_ms = 0, user_message_count = 501 WHERE id = 'batch-session'")
            .execute(&mut *transaction)
            .await
            .expect("fixture aggregate");
        transaction.commit().await.expect("fixture commit");

        let result = repository
            .run_retention_batch(30 * DAY_MS, 5_000)
            .await
            .expect("bounded cleanup");
        assert_eq!(result.messages_deleted, 500);
        assert_eq!(
            repository.message_count().await.expect("remaining count"),
            1
        );
        storage.close().await;
    }

    #[tokio::test]
    async fn retention_applies_exact_outbox_boundaries_and_reports_expired_pending() {
        let (_temp, storage, repository) = retention_fixture().await;
        let now_ms = 8 * DAY_MS;
        for (id, created_at_ms, delivered_at_ms) in [
            ("delivered-before", 0, Some(now_ms - DAY_MS - 1)),
            ("delivered-exact", 0, Some(now_ms - DAY_MS)),
            ("delivered-young", 0, Some(now_ms - DAY_MS + 1)),
            ("pending-before", now_ms - 7 * DAY_MS - 1, None),
            ("pending-exact", now_ms - 7 * DAY_MS, None),
            ("pending-young", now_ms - 7 * DAY_MS + 1, None),
        ] {
            sqlx::query(
                "INSERT INTO outbox_events(id, aggregate_type, aggregate_id, aggregate_revision, event_type, payload_json, created_at_ms, delivered_at_ms) VALUES(?, 'retention-fixture', ?, 0, 'fixture', '{}', ?, ?)",
            )
            .bind(id)
            .bind(format!("aggregate-{id}"))
            .bind(created_at_ms)
            .bind(delivered_at_ms)
            .execute(&repository.writer)
            .await
            .expect("outbox fixture");
        }

        let result = repository
            .run_retention_batch(now_ms, 500)
            .await
            .expect("outbox retention");
        assert_eq!(result.delivered_outbox_deleted, 2);
        assert_eq!(result.expired_undelivered_outbox_deleted, 2);
        let remaining: Vec<String> = sqlx::query_scalar("SELECT id FROM outbox_events ORDER BY id")
            .fetch_all(&repository.writer)
            .await
            .expect("remaining outbox");
        assert_eq!(
            remaining,
            vec!["delivered-young".to_owned(), "pending-young".to_owned()]
        );
        storage.close().await;
    }
}
