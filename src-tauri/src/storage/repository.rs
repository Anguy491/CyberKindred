use super::{StorageError, StorageReason};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

pub const THIRTY_DAYS_MS: i64 = 30 * 24 * 60 * 60 * 1_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChatRole {
    User,
    Assistant,
}

impl ChatRole {
    const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

/// Raw chat input for a single transactional insert.
///
/// This type intentionally does not implement `Debug`, `Clone`, or `Serialize`.
pub struct NewChatMessage {
    id: String,
    chat_session_id: String,
    role: ChatRole,
    content_text: String,
    provider: Option<String>,
    model: Option<String>,
    prompt_version: Option<String>,
    created_at_ms: i64,
}

impl NewChatMessage {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        id: String,
        chat_session_id: String,
        role: ChatRole,
        content_text: String,
        provider: Option<String>,
        model: Option<String>,
        prompt_version: Option<String>,
        created_at_ms: i64,
    ) -> Self {
        Self {
            id,
            chat_session_id,
            role,
            content_text,
            provider,
            model,
            prompt_version,
            created_at_ms,
        }
    }
}

/// Counts from one bounded retention transaction.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RetentionResult {
    pub messages_deleted: u64,
    pub summaries_created: u64,
    pub voice_text_cleared: u64,
    pub rejected_proposal_content_cleared: u64,
    pub delivered_outbox_deleted: u64,
    pub expired_undelivered_outbox_deleted: u64,
    pub weather_cache_deleted: u64,
}

/// Parameterized, transactional application repository.
#[derive(Clone)]
pub struct Repository {
    pub(super) writer: SqlitePool,
}

impl Repository {
    pub(super) const fn new(writer: SqlitePool) -> Self {
        Self { writer }
    }

    /// Creates an empty chat aggregate in a transaction.
    ///
    /// # Errors
    ///
    /// Returns `storage_write_failed` when validation or the transaction fails.
    pub async fn create_chat_session(
        &self,
        id: &str,
        started_at_ms: i64,
    ) -> Result<(), StorageError> {
        let mut transaction = self
            .writer
            .begin()
            .await
            .map_err(|_| StorageError::new(StorageReason::StorageWriteFailed))?;
        sqlx::query("INSERT INTO chat_sessions(id, started_at_ms, status) VALUES(?, ?, 'active')")
            .bind(id)
            .bind(started_at_ms)
            .execute(&mut *transaction)
            .await
            .map_err(|_| StorageError::new(StorageReason::StorageWriteFailed))?;
        transaction
            .commit()
            .await
            .map_err(|_| StorageError::new(StorageReason::StorageWriteFailed))
    }

    /// Inserts a message and updates its session aggregate in one transaction.
    ///
    /// # Errors
    ///
    /// Returns a stable storage error when the message is invalid, the session
    /// does not exist, or either transactional write fails.
    pub async fn insert_message(&self, message: NewChatMessage) -> Result<(), StorageError> {
        if message.id.is_empty()
            || message.chat_session_id.is_empty()
            || message.content_text.is_empty()
            || message.content_text.len() > 100_000
        {
            return Err(StorageError::new(StorageReason::StorageWriteFailed));
        }
        let expires_at_ms = message
            .created_at_ms
            .checked_add(THIRTY_DAYS_MS)
            .ok_or_else(|| StorageError::new(StorageReason::StorageWriteFailed))?;
        let content_hash = hex::encode(Sha256::digest(message.content_text.as_bytes()));
        let mut transaction = self
            .writer
            .begin()
            .await
            .map_err(|_| StorageError::new(StorageReason::StorageWriteFailed))?;

        sqlx::query(
            "INSERT INTO messages(id, chat_session_id, role, content_text, content_hash, provider, model, prompt_version, created_at_ms, expires_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(message.id)
        .bind(&message.chat_session_id)
        .bind(message.role.as_str())
        .bind(message.content_text)
        .bind(content_hash)
        .bind(message.provider)
        .bind(message.model)
        .bind(message.prompt_version)
        .bind(message.created_at_ms)
        .bind(expires_at_ms)
        .execute(&mut *transaction)
        .await
        .map_err(|_| StorageError::new(StorageReason::StorageWriteFailed))?;

        let (user_increment, assistant_increment) = match message.role {
            ChatRole::User => (1, 0),
            ChatRole::Assistant => (0, 1),
        };
        let update = sqlx::query(
            "UPDATE chat_sessions SET covered_from_ms = CASE WHEN covered_from_ms IS NULL OR ? < covered_from_ms THEN ? ELSE covered_from_ms END, covered_to_ms = CASE WHEN covered_to_ms IS NULL OR ? > covered_to_ms THEN ? ELSE covered_to_ms END, user_message_count = user_message_count + ?, assistant_message_count = assistant_message_count + ? WHERE id = ?",
        )
        .bind(message.created_at_ms)
        .bind(message.created_at_ms)
        .bind(message.created_at_ms)
        .bind(message.created_at_ms)
        .bind(user_increment)
        .bind(assistant_increment)
        .bind(message.chat_session_id)
        .execute(&mut *transaction)
        .await
        .map_err(|_| StorageError::new(StorageReason::StorageWriteFailed))?;
        if update.rows_affected() != 1 {
            let _ = transaction.rollback().await;
            return Err(StorageError::new(StorageReason::EntityNotFound));
        }
        transaction
            .commit()
            .await
            .map_err(|_| StorageError::new(StorageReason::StorageWriteFailed))
    }

    /// Returns only a count for inventory; message content never leaves storage.
    ///
    /// # Errors
    ///
    /// Returns `storage_read_failed` when SQLite cannot return the count.
    pub async fn message_count(&self) -> Result<i64, StorageError> {
        sqlx::query_scalar("SELECT count(*) FROM messages")
            .fetch_one(&self.writer)
            .await
            .map_err(|_| StorageError::new(StorageReason::StorageReadFailed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{AppPaths, Storage};

    async fn repository_fixture() -> (tempfile::TempDir, Storage, Repository) {
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

    #[tokio::test]
    async fn repository_message_write_is_atomic_and_uses_absolute_expiry() {
        let (_temp, storage, repository) = repository_fixture().await;
        repository
            .create_chat_session("session-1", 1_000)
            .await
            .expect("session");
        repository
            .insert_message(NewChatMessage::new(
                "message-1".to_owned(),
                "session-1".to_owned(),
                ChatRole::User,
                "fixture chat body".to_owned(),
                None,
                None,
                None,
                1_000,
            ))
            .await
            .expect("message");
        let row: (i64, i64, i64) = sqlx::query_as(
            "SELECT created_at_ms, expires_at_ms, (SELECT user_message_count FROM chat_sessions WHERE id = 'session-1') FROM messages WHERE id = 'message-1'",
        )
        .fetch_one(&repository.writer)
        .await
        .expect("message facts");
        assert_eq!(row, (1_000, 1_000 + THIRTY_DAYS_MS, 1));
        storage.close().await;
    }

    #[tokio::test]
    async fn repository_missing_session_rolls_back_message() {
        let (_temp, storage, repository) = repository_fixture().await;
        let result = repository
            .insert_message(NewChatMessage::new(
                "message-orphan".to_owned(),
                "missing-session".to_owned(),
                ChatRole::Assistant,
                "must roll back".to_owned(),
                None,
                None,
                None,
                1_000,
            ))
            .await;
        assert!(result.is_err());
        assert_eq!(repository.message_count().await.expect("message count"), 0);
        storage.close().await;
    }

    #[tokio::test]
    async fn repository_interrupted_transaction_is_rolled_back_on_reopen() {
        let (temp, storage, repository) = repository_fixture().await;
        let mut transaction = repository.writer.begin().await.expect("transaction");
        sqlx::query("INSERT INTO app_settings(key, value_json, schema_version, updated_at_ms) VALUES('ui.crash-fixture', 'true', 1, 1)")
            .execute(&mut *transaction)
            .await
            .expect("uncommitted write");
        drop(transaction);
        storage.close().await;

        let paths = AppPaths::create(
            temp.path().join("data"),
            temp.path().join("cache"),
            temp.path().join("logs"),
        )
        .expect("scoped paths");
        let reopened = Storage::open(&paths, "0.1.0").await.expect("recovery open");
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM app_settings WHERE key = 'ui.crash-fixture'")
                .fetch_one(&reopened.repository().writer)
                .await
                .expect("setting count");
        assert_eq!(count, 0);
        reopened.close().await;
    }

    #[tokio::test]
    async fn repository_rejects_secret_settings_and_unsafe_origins() {
        let (temp, storage, repository) = repository_fixture().await;
        let canary = "ck-task007-secret-canary-never-in-sqlite";
        let secret_insert = sqlx::query(
            "INSERT INTO app_settings(key, value_json, schema_version, updated_at_ms) VALUES(?, ?, 1, 1)",
        )
        .bind("provider.openai.api_key")
        .bind(serde_json::to_string(canary).expect("encode canary"))
        .execute(&repository.writer)
        .await;
        assert!(secret_insert.is_err());

        let unsafe_settings = crate::storage::StoredProviderSettings {
            provider_origin: "http://127.0.0.1".to_owned(),
            ..crate::storage::StoredProviderSettings::default()
        };
        assert!(
            repository
                .save_provider_settings(0, &unsafe_settings, 1, false, None)
                .await
                .is_err()
        );
        storage.passive_checkpoint().await.expect("checkpoint");
        let data_root = temp.path().join("data");
        let backup_root = data_root.join("backups");
        for directory in [&data_root, &backup_root] {
            for entry in std::fs::read_dir(directory).expect("app-data entries") {
                let entry = entry.expect("app-data entry");
                if entry.path().is_file() {
                    let bytes = std::fs::read(entry.path()).expect("read app-data artifact");
                    assert!(
                        !bytes
                            .windows(canary.len())
                            .any(|window| window == canary.as_bytes()),
                        "secret canary must not enter an app-data artifact"
                    );
                }
            }
        }
        storage.close().await;
    }
}
