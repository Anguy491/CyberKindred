//! Transactional M4 storage boundaries for chat, feedback, memory, profile and summaries.

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use super::{Repository, StorageError, StorageReason, repository::THIRTY_DAYS_MS};

const MAX_CONTEXT_TURNS: i64 = 12;
const MAX_CONTEXT_MEMORIES: i64 = 20;
const MAX_CONTEXT_MEMORY_CHARS: usize = 240;
const MAX_HISTORICAL_TURN_CHARS: usize = 500;
const MAX_JS_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct AcceptedChat {
    pub session_id: Uuid,
    pub message_id: Uuid,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct ContextTurn {
    pub role: String,
    pub text: String,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct ContextMemory {
    pub memory_id: Uuid,
    pub content: String,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct ChatContextSnapshot {
    pub display_name: String,
    pub initial_preferences: Vec<String>,
    pub approved_memories: Vec<ContextMemory>,
    pub turns: Vec<ContextTurn>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StoredMemoryKind {
    Preference,
    Routine,
    Boundary,
    Biographical,
}

impl StoredMemoryKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Preference => "preference",
            Self::Routine => "routine",
            Self::Boundary => "boundary",
            Self::Biographical => "biographical",
        }
    }

    fn parse(value: &str) -> Result<Self, StorageError> {
        match value {
            "preference" => Ok(Self::Preference),
            "routine" => Ok(Self::Routine),
            "boundary" => Ok(Self::Boundary),
            "biographical" => Ok(Self::Biographical),
            _ => Err(integrity_error()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StoredMemoryStatus {
    Proposed,
    Approved,
    Disabled,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StoredMemory {
    pub memory_id: Uuid,
    pub status: StoredMemoryStatus,
    pub kind: StoredMemoryKind,
    pub content: String,
    pub confidence: f64,
    pub source_session_id: Option<Uuid>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub approved_at_ms: Option<i64>,
    pub last_used_at_ms: Option<i64>,
    pub enabled: bool,
    pub revision: u64,
}

#[derive(Clone, PartialEq)]
pub(crate) struct NewMemoryProposal {
    pub kind: StoredMemoryKind,
    pub content: String,
    pub confidence: f64,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct StoredProfileView {
    pub display_name: String,
    pub companion_style: String,
    pub initial_preferences: Vec<String>,
    pub narration_density: String,
    pub city: Option<String>,
    pub region: Option<String>,
    pub country: Option<String>,
    pub country_code: Option<String>,
    pub latitude: Option<String>,
    pub longitude: Option<String>,
    pub timezone: Option<String>,
    pub revision: u64,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct StoredPreferenceTrend {
    pub kind: String,
    pub label: String,
    pub direction: String,
    pub sample_count: u32,
    pub window_days: u32,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct StoredSessionSummary {
    pub summary_id: Uuid,
    pub covered_from_ms: i64,
    pub covered_to_ms: i64,
    pub summary: String,
    pub generation_kind: String,
    pub revision: u64,
}

impl Repository {
    /// Accepts one user-authored chat turn before any provider call begins.
    pub(crate) async fn accept_chat_message(
        &self,
        program_id: Uuid,
        text: &str,
        apply_less_talk: bool,
        created_at_ms: i64,
    ) -> Result<AcceptedChat, StorageError> {
        if text.trim().is_empty() || text.chars().count() > 4_000 || created_at_ms < 0 {
            return Err(write_error());
        }
        let mut tx = self.writer.begin().await.map_err(|_| write_error())?;
        let program_exists: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM program_runs WHERE id = ? AND status NOT IN ('completed', 'failed', 'interrupted'))",
        )
        .bind(program_id.to_string())
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| read_error())?;
        if program_exists != 1 {
            return Err(StorageError::new(StorageReason::EntityNotFound));
        }
        let session_id: Option<String> =
            sqlx::query_scalar("SELECT id FROM chat_sessions WHERE program_run_id = ?")
                .bind(program_id.to_string())
                .fetch_optional(&mut *tx)
                .await
                .map_err(|_| read_error())?;
        let session_id = if let Some(id) = session_id {
            parse_v7(&id)?
        } else {
            let id = Uuid::now_v7();
            sqlx::query(
                "INSERT INTO chat_sessions(id, program_run_id, started_at_ms, status) VALUES(?, ?, ?, 'active')",
            )
            .bind(id.to_string())
            .bind(program_id.to_string())
            .bind(created_at_ms)
            .execute(&mut *tx)
            .await
            .map_err(|_| write_error())?;
            id
        };
        let message_id = Uuid::now_v7();
        insert_message(
            &mut tx,
            message_id,
            session_id,
            "user",
            text,
            None,
            None,
            None,
            created_at_ms,
        )
        .await?;
        if apply_less_talk {
            let baseline_completed_tracks: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM program_segments WHERE program_run_id = ? AND kind = 'track' AND status = 'completed'",
            )
            .bind(program_id.to_string())
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| read_error())?;
            sqlx::query(
                "INSERT INTO feedback(id, program_run_id, target_kind, feedback_type, value_json, created_at_ms) VALUES(?, ?, 'program', 'less_talk', ?, ?)",
            )
            .bind(Uuid::now_v7().to_string())
            .bind(program_id.to_string())
            .bind(json!({"baselineCompletedTracks": baseline_completed_tracks}).to_string())
            .bind(created_at_ms)
            .execute(&mut *tx)
            .await
            .map_err(|_| write_error())?;
        }
        tx.commit().await.map_err(|_| write_error())?;
        Ok(AcceptedChat {
            session_id,
            message_id,
        })
    }

    /// Loads bounded, path-free chat context. Proposals and disabled/deleted memories are excluded.
    pub(crate) async fn load_chat_context(
        &self,
        session_id: Uuid,
    ) -> Result<ChatContextSnapshot, StorageError> {
        let mut tx = self.writer.begin().await.map_err(|_| read_error())?;
        let profile: Option<(Option<String>, String)> = sqlx::query_as(
            "SELECT display_name, program_preferences_json FROM user_profile WHERE id = 'current'",
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| read_error())?;
        let (display_name, initial_preferences) = match profile {
            Some((name, encoded)) => (name.unwrap_or_default(), decode_preferences(&encoded)?),
            None => (String::new(), Vec::new()),
        };
        let memory_rows = sqlx::query(
            "SELECT memory.id, revision.statement_text \
             FROM memories memory \
             JOIN memory_revisions revision ON revision.memory_id = memory.id AND revision.revision = memory.current_revision \
             WHERE memory.status = 'approved' AND memory.deleted_at_ms IS NULL AND revision.statement_text IS NOT NULL \
             ORDER BY memory.pinned DESC, memory.updated_at_ms DESC, memory.id ASC LIMIT ?",
        )
        .bind(MAX_CONTEXT_MEMORIES)
        .fetch_all(&mut *tx)
        .await
        .map_err(|_| read_error())?;
        let approved_memories = memory_rows
            .into_iter()
            .map(|row| {
                let content: String = row
                    .try_get("statement_text")
                    .map_err(|_| integrity_error())?;
                if content.is_empty()
                    || content.chars().count() > 500
                    || content.chars().any(char::is_control)
                {
                    return Err(integrity_error());
                }
                Ok(ContextMemory {
                    memory_id: parse_v7(
                        row.try_get::<String, _>("id")
                            .map_err(|_| integrity_error())?
                            .as_str(),
                    )?,
                    content: truncate_chars(content, MAX_CONTEXT_MEMORY_CHARS),
                })
            })
            .collect::<Result<Vec<_>, StorageError>>()?;
        let turn_rows = sqlx::query(
            "SELECT role, content_text FROM messages WHERE chat_session_id = ? ORDER BY created_at_ms DESC, id DESC LIMIT ?",
        )
        .bind(session_id.to_string())
        .bind(MAX_CONTEXT_TURNS)
        .fetch_all(&mut *tx)
        .await
        .map_err(|_| read_error())?;
        let mut turns = turn_rows
            .into_iter()
            .map(|row| {
                let role: String = row.try_get("role").map_err(|_| integrity_error())?;
                if !matches!(role.as_str(), "user" | "assistant") {
                    return Err(integrity_error());
                }
                Ok(ContextTurn {
                    role,
                    text: row.try_get("content_text").map_err(|_| integrity_error())?,
                })
            })
            .collect::<Result<Vec<_>, StorageError>>()?;
        turns.reverse();
        // A provider-bound load follows `accept_chat_message`, so its final row
        // is the current user turn and must retain the API's full input bound.
        // Read-only callers may inspect a completed transcript whose final row
        // is an assistant reply; in that case every row is historical.
        let current_turn = turns
            .last()
            .is_some_and(|turn| turn.role == "user")
            .then(|| turns.len().saturating_sub(1));
        for (index, turn) in turns.iter_mut().enumerate() {
            if Some(index) != current_turn {
                turn.text =
                    truncate_chars(std::mem::take(&mut turn.text), MAX_HISTORICAL_TURN_CHARS);
            }
        }
        tx.commit().await.map_err(|_| read_error())?;
        Ok(ChatContextSnapshot {
            display_name,
            initial_preferences,
            approved_memories,
            turns,
        })
    }

    /// Persists the assistant reply and non-duplicate memory proposals atomically.
    #[allow(clippy::too_many_arguments)] // The transaction records explicit provider provenance.
    #[allow(clippy::too_many_lines)] // Reply, proposal provenance, and last-used stamps share one atomic write.
    pub(crate) async fn persist_chat_result(
        &self,
        session_id: Uuid,
        source_message_id: Uuid,
        used_memory_ids: &[Uuid],
        assistant_text: &str,
        proposals: &[NewMemoryProposal],
        provider: Option<&str>,
        model: Option<&str>,
        prompt_version: &str,
        created_at_ms: i64,
    ) -> Result<(Uuid, Vec<StoredMemory>), StorageError> {
        if assistant_text.is_empty()
            || assistant_text.chars().count() > 2_000
            || proposals.len() > 3
            || prompt_version.is_empty()
        {
            return Err(write_error());
        }
        let mut tx = self.writer.begin().await.map_err(|_| write_error())?;
        let message_id = Uuid::now_v7();
        insert_message(
            &mut tx,
            message_id,
            session_id,
            "assistant",
            assistant_text,
            provider,
            model,
            Some(prompt_version),
            created_at_ms,
        )
        .await?;
        let mut stored = Vec::new();
        for proposal in proposals {
            if proposal.content.is_empty()
                || proposal.content.chars().count() > 500
                || proposal.content.chars().any(char::is_control)
                || !(0.0..=1.0).contains(&proposal.confidence)
            {
                return Err(write_error());
            }
            let hash = content_hash(&proposal.content);
            let duplicate: i64 = sqlx::query_scalar(
                "SELECT (EXISTS(SELECT 1 FROM memory_proposals WHERE statement_hash = ?)) \
                        OR (EXISTS(SELECT 1 FROM memory_revisions WHERE statement_hash = ?))",
            )
            .bind(&hash)
            .bind(&hash)
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| read_error())?;
            if duplicate == 1 {
                continue;
            }
            let memory_id = Uuid::now_v7();
            sqlx::query(
                "INSERT INTO memory_proposals(id, category, statement_text, statement_hash, confidence, status, created_at_ms, prompt_version, model, revision) VALUES(?, ?, ?, ?, ?, 'proposed', ?, ?, ?, 0)",
            )
            .bind(memory_id.to_string())
            .bind(proposal.kind.as_str())
            .bind(&proposal.content)
            .bind(&hash)
            .bind(proposal.confidence)
            .bind(created_at_ms)
            .bind(prompt_version)
            .bind(model)
            .execute(&mut *tx)
            .await
            .map_err(|_| write_error())?;
            let source = sqlx::query(
                "INSERT INTO memory_proposal_sources(proposal_id, message_id, source_content_hash) \
                 SELECT ?, id, content_hash FROM messages \
                 WHERE id = ? AND chat_session_id = ? AND role = 'user'",
            )
            .bind(memory_id.to_string())
            .bind(source_message_id.to_string())
            .bind(session_id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(|_| write_error())?;
            if source.rows_affected() != 1 {
                return Err(integrity_error());
            }
            stored.push(StoredMemory {
                memory_id,
                status: StoredMemoryStatus::Proposed,
                kind: proposal.kind,
                content: proposal.content.clone(),
                confidence: proposal.confidence,
                source_session_id: Some(session_id),
                created_at_ms,
                updated_at_ms: created_at_ms,
                approved_at_ms: None,
                last_used_at_ms: None,
                enabled: false,
                revision: 0,
            });
        }
        for memory_id in used_memory_ids {
            sqlx::query(
                "UPDATE memories SET last_used_at_ms = ? WHERE id = ? AND status = 'approved' AND deleted_at_ms IS NULL",
            )
            .bind(created_at_ms)
            .bind(memory_id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(|_| write_error())?;
        }
        tx.commit().await.map_err(|_| write_error())?;
        Ok((message_id, stored))
    }

    /// Persists explicit feedback synchronously and returns the next program revision.
    pub(crate) async fn record_feedback(
        &self,
        program_id: Uuid,
        track_id: Option<Uuid>,
        kind: &str,
        created_at_ms: i64,
    ) -> Result<u64, StorageError> {
        if !matches!(kind, "like" | "skip" | "less_talk") {
            return Err(StorageError::new(StorageReason::InvalidSetting));
        }
        if kind == "less_talk" && track_id.is_some() || kind != "less_talk" && track_id.is_none() {
            return Err(StorageError::new(StorageReason::InvalidSetting));
        }
        let mut tx = self.writer.begin().await.map_err(|_| write_error())?;
        let current_revision: Option<i64> =
            sqlx::query_scalar("SELECT revision FROM program_runs WHERE id = ?")
                .bind(program_id.to_string())
                .fetch_optional(&mut *tx)
                .await
                .map_err(|_| read_error())?;
        let current_revision =
            current_revision.ok_or_else(|| StorageError::new(StorageReason::EntityNotFound))?;
        let baseline_completed_tracks: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM program_segments WHERE program_run_id = ? AND kind = 'track' AND status = 'completed'",
        )
        .bind(program_id.to_string())
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| read_error())?;
        if let Some(track_id) = track_id {
            let belongs: i64 = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM program_segments WHERE program_run_id = ? AND track_id = ?)",
            )
            .bind(program_id.to_string())
            .bind(track_id.to_string())
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| read_error())?;
            if belongs != 1 {
                return Err(StorageError::new(StorageReason::EntityNotFound));
            }
        }
        let id = Uuid::now_v7();
        let value_json = if kind == "less_talk" {
            json!({"baselineCompletedTracks": baseline_completed_tracks}).to_string()
        } else {
            "{}".to_owned()
        };
        sqlx::query(
            "INSERT INTO feedback(id, program_run_id, target_kind, track_id, feedback_type, value_json, created_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(program_id.to_string())
        .bind(if kind == "less_talk" { "program" } else { "track" })
        .bind(track_id.map(|id| id.to_string()))
        .bind(kind)
        .bind(value_json)
        .bind(created_at_ms)
        .execute(&mut *tx)
        .await
        .map_err(|_| write_error())?;
        tx.commit().await.map_err(|_| write_error())?;
        u64::try_from(current_revision).map_err(|_| integrity_error())
    }

    /// Enforces the active less-talk request for the currently running program.
    pub(crate) async fn voice_allowed_after_feedback(
        &self,
        program_id: Uuid,
    ) -> Result<bool, StorageError> {
        let row: Option<(String, i64)> = sqlx::query_as(
            "SELECT value_json, created_at_ms FROM feedback WHERE program_run_id = ? AND feedback_type = 'less_talk' AND revoked_at_ms IS NULL ORDER BY created_at_ms DESC, id DESC LIMIT 1",
        )
        .bind(program_id.to_string())
        .fetch_optional(&self.writer)
        .await
        .map_err(|_| read_error())?;
        let Some((encoded, feedback_at)) = row else {
            return Ok(true);
        };
        let baseline = serde_json::from_str::<Value>(&encoded)
            .ok()
            .and_then(|value| value.get("baselineCompletedTracks").and_then(Value::as_i64))
            .ok_or_else(integrity_error)?;
        let completed_tracks: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM program_segments WHERE program_run_id = ? AND kind = 'track' AND status = 'completed'",
        )
        .bind(program_id.to_string())
        .fetch_one(&self.writer)
        .await
        .map_err(|_| read_error())?;
        let latest_voice_end: Option<i64> = sqlx::query_scalar(
            "SELECT max(ended_at_ms) FROM program_segments WHERE program_run_id = ? AND kind = 'voice' AND status = 'completed' AND ended_at_ms >= ?",
        )
        .bind(program_id.to_string())
        .bind(feedback_at)
        .fetch_one(&self.writer)
        .await
        .map_err(|_| read_error())?;
        let since = match latest_voice_end {
            Some(voice_end) => sqlx::query_scalar(
                "SELECT count(*) FROM program_segments WHERE program_run_id = ? AND kind = 'track' AND status = 'completed' AND ended_at_ms > ?",
            )
            .bind(program_id.to_string())
            .bind(voice_end)
            .fetch_one(&self.writer)
            .await
            .map_err(|_| read_error())?,
            None => completed_tracks.saturating_sub(baseline),
        };
        Ok(since >= 4)
    }

    pub(crate) async fn list_memories(
        &self,
        status: Option<&str>,
        offset: u64,
        limit: u32,
    ) -> Result<(Vec<StoredMemory>, Option<u64>), StorageError> {
        if !matches!(status, None | Some("proposed" | "approved" | "disabled"))
            || limit == 0
            || limit > 100
        {
            return Err(StorageError::new(StorageReason::InvalidSetting));
        }
        let rows = sqlx::query(
            "SELECT * FROM ( \
               SELECT proposal.id, 'proposed' AS exposed_status, proposal.category, proposal.statement_text AS content, proposal.confidence, \
                 (SELECT messages.chat_session_id FROM memory_proposal_sources source JOIN messages ON messages.id = source.message_id WHERE source.proposal_id = proposal.id LIMIT 1) AS source_session_id, \
                 proposal.created_at_ms, proposal.created_at_ms AS updated_at_ms, NULL AS approved_at_ms, NULL AS last_used_at_ms, 0 AS enabled, proposal.revision \
               FROM memory_proposals proposal WHERE proposal.status = 'proposed' \
               UNION ALL \
               SELECT memory.id, memory.status, memory.category, revision.statement_text, COALESCE(proposal.confidence, 1.0), \
                 (SELECT messages.chat_session_id FROM memory_proposal_sources source JOIN messages ON messages.id = source.message_id WHERE source.proposal_id = memory.created_from_proposal_id LIMIT 1), \
                 memory.created_at_ms, memory.updated_at_ms, proposal.decided_at_ms, memory.last_used_at_ms, CASE WHEN memory.status = 'approved' THEN 1 ELSE 0 END, memory.current_revision \
               FROM memories memory JOIN memory_revisions revision ON revision.memory_id = memory.id AND revision.revision = memory.current_revision \
               LEFT JOIN memory_proposals proposal ON proposal.id = memory.created_from_proposal_id \
               WHERE memory.status IN ('approved', 'disabled') AND memory.deleted_at_ms IS NULL \
             ) WHERE (? IS NULL OR exposed_status = ?) ORDER BY updated_at_ms DESC, id ASC LIMIT ? OFFSET ?",
        )
        .bind(status)
        .bind(status)
        .bind(i64::from(limit) + 1)
        .bind(i64::try_from(offset).map_err(|_| StorageError::new(StorageReason::InvalidSetting))?)
        .fetch_all(&self.writer)
        .await
        .map_err(|_| read_error())?;
        let has_more = rows.len() > limit as usize;
        let items = rows
            .into_iter()
            .take(limit as usize)
            .map(|row| decode_memory(&row))
            .collect::<Result<Vec<_>, _>>()?;
        let next = has_more.then(|| offset + u64::from(limit));
        Ok((items, next))
    }

    pub(crate) async fn approve_memory(
        &self,
        memory_id: Uuid,
        expected_revision: u64,
        decided_at_ms: i64,
    ) -> Result<StoredMemory, StorageError> {
        let mut tx = self.writer.begin().await.map_err(|_| write_error())?;
        let row = sqlx::query(
            "SELECT category, statement_text, confidence, created_at_ms, revision, \
             (SELECT messages.chat_session_id FROM memory_proposal_sources source JOIN messages ON messages.id = source.message_id WHERE source.proposal_id = memory_proposals.id LIMIT 1) AS source_session_id \
             FROM memory_proposals WHERE id = ? AND status = 'proposed'",
        )
        .bind(memory_id.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| read_error())?
        .ok_or_else(|| StorageError::new(StorageReason::EntityNotFound))?;
        let revision: i64 = row.try_get("revision").map_err(|_| integrity_error())?;
        if u64::try_from(revision).map_err(|_| integrity_error())? != expected_revision {
            return Err(StorageError::new(StorageReason::RevisionConflict));
        }
        let content: String = row
            .try_get("statement_text")
            .map_err(|_| integrity_error())?;
        let kind = StoredMemoryKind::parse(
            row.try_get::<String, _>("category")
                .map_err(|_| integrity_error())?
                .as_str(),
        )?;
        sqlx::query("UPDATE memory_proposals SET status = 'approved', decided_at_ms = ?, revision = revision + 1 WHERE id = ? AND revision = ?")
            .bind(decided_at_ms).bind(memory_id.to_string()).bind(revision)
            .execute(&mut *tx).await.map_err(|_| write_error())?;
        sqlx::query("INSERT INTO memories(id, category, current_revision, status, created_from_proposal_id, created_at_ms, updated_at_ms) VALUES(?, ?, 1, 'approved', ?, ?, ?)")
            .bind(memory_id.to_string()).bind(kind.as_str()).bind(memory_id.to_string())
            .bind(row.try_get::<i64, _>("created_at_ms").map_err(|_| integrity_error())?).bind(decided_at_ms)
            .execute(&mut *tx).await.map_err(|_| write_error())?;
        sqlx::query("INSERT INTO memory_revisions(memory_id, revision, statement_text, statement_hash, change_kind, changed_at_ms) VALUES(?, 1, ?, ?, 'approved', ?)")
            .bind(memory_id.to_string()).bind(&content).bind(content_hash(&content)).bind(decided_at_ms)
            .execute(&mut *tx).await.map_err(|_| write_error())?;
        tx.commit().await.map_err(|_| write_error())?;
        Ok(StoredMemory {
            memory_id,
            status: StoredMemoryStatus::Approved,
            kind,
            content,
            confidence: row.try_get("confidence").map_err(|_| integrity_error())?,
            source_session_id: row
                .try_get::<Option<String>, _>("source_session_id")
                .map_err(|_| integrity_error())?
                .map(|id| parse_v7(&id))
                .transpose()?,
            created_at_ms: row
                .try_get("created_at_ms")
                .map_err(|_| integrity_error())?,
            updated_at_ms: decided_at_ms,
            approved_at_ms: Some(decided_at_ms),
            last_used_at_ms: None,
            enabled: true,
            revision: 1,
        })
    }

    #[allow(clippy::too_many_lines)] // Proposal and approved-memory revision paths share one gate.
    pub(crate) async fn update_memory(
        &self,
        memory_id: Uuid,
        expected_revision: u64,
        content: &str,
        enabled: bool,
        updated_at_ms: i64,
    ) -> Result<StoredMemory, StorageError> {
        if content.is_empty() || content.chars().count() > 500 {
            return Err(StorageError::new(StorageReason::InvalidSetting));
        }
        let mut tx = self.writer.begin().await.map_err(|_| write_error())?;
        let proposal = sqlx::query("SELECT category, confidence, created_at_ms, revision FROM memory_proposals WHERE id = ? AND status = 'proposed'")
            .bind(memory_id.to_string()).fetch_optional(&mut *tx).await.map_err(|_| read_error())?;
        if let Some(row) = proposal {
            let revision: i64 = row.try_get("revision").map_err(|_| integrity_error())?;
            if u64::try_from(revision).map_err(|_| integrity_error())? != expected_revision
                || enabled
            {
                return Err(StorageError::new(StorageReason::RevisionConflict));
            }
            let hash = content_hash(content);
            let duplicate: i64 = sqlx::query_scalar(
                "SELECT (EXISTS(SELECT 1 FROM memory_proposals WHERE statement_hash = ? AND id <> ?)) \
                        OR (EXISTS(SELECT 1 FROM memory_revisions WHERE statement_hash = ?))",
            )
            .bind(&hash)
            .bind(memory_id.to_string())
            .bind(&hash)
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| read_error())?;
            if duplicate == 1 {
                return Err(StorageError::new(StorageReason::RevisionConflict));
            }
            sqlx::query("UPDATE memory_proposals SET statement_text = ?, statement_hash = ?, revision = revision + 1 WHERE id = ? AND revision = ?")
                .bind(content).bind(hash).bind(memory_id.to_string()).bind(revision)
                .execute(&mut *tx).await.map_err(|_| write_error())?;
            tx.commit().await.map_err(|_| write_error())?;
            return Ok(StoredMemory {
                memory_id,
                status: StoredMemoryStatus::Proposed,
                kind: StoredMemoryKind::parse(
                    row.try_get::<String, _>("category")
                        .map_err(|_| integrity_error())?
                        .as_str(),
                )?,
                content: content.to_owned(),
                confidence: row.try_get("confidence").map_err(|_| integrity_error())?,
                source_session_id: None,
                created_at_ms: row
                    .try_get("created_at_ms")
                    .map_err(|_| integrity_error())?,
                updated_at_ms,
                approved_at_ms: None,
                last_used_at_ms: None,
                enabled: false,
                revision: expected_revision.checked_add(1).ok_or_else(write_error)?,
            });
        }
        let row = sqlx::query(
            "SELECT memory.category, memory.status, memory.current_revision, memory.created_at_ms, memory.last_used_at_ms, \
             revision.statement_text, COALESCE(proposal.confidence, 1.0) AS confidence, proposal.decided_at_ms, \
             (SELECT messages.chat_session_id FROM memory_proposal_sources source JOIN messages ON messages.id = source.message_id WHERE source.proposal_id = memory.created_from_proposal_id LIMIT 1) AS source_session_id \
             FROM memories memory JOIN memory_revisions revision ON revision.memory_id = memory.id AND revision.revision = memory.current_revision \
             LEFT JOIN memory_proposals proposal ON proposal.id = memory.created_from_proposal_id \
             WHERE memory.id = ? AND memory.status IN ('approved', 'disabled')",
        )
        .bind(memory_id.to_string()).fetch_optional(&mut *tx).await.map_err(|_| read_error())?
        .ok_or_else(|| StorageError::new(StorageReason::EntityNotFound))?;
        let current: i64 = row
            .try_get("current_revision")
            .map_err(|_| integrity_error())?;
        if u64::try_from(current).map_err(|_| integrity_error())? != expected_revision {
            return Err(StorageError::new(StorageReason::RevisionConflict));
        }
        let old_content: String = row
            .try_get("statement_text")
            .map_err(|_| integrity_error())?;
        let old_status: String = row.try_get("status").map_err(|_| integrity_error())?;
        let next = current.checked_add(1).ok_or_else(write_error)?;
        let next_status = if enabled { "approved" } else { "disabled" };
        let change_kind = if old_content != content {
            "edited"
        } else if enabled && old_status == "disabled" {
            "reenabled"
        } else if !enabled && old_status == "approved" {
            "disabled"
        } else {
            "edited"
        };
        sqlx::query("INSERT INTO memory_revisions(memory_id, revision, statement_text, statement_hash, change_kind, changed_at_ms) VALUES(?, ?, ?, ?, ?, ?)")
            .bind(memory_id.to_string()).bind(next).bind(content).bind(content_hash(content)).bind(change_kind).bind(updated_at_ms)
            .execute(&mut *tx).await.map_err(|_| write_error())?;
        sqlx::query("UPDATE memories SET current_revision = ?, status = ?, updated_at_ms = ? WHERE id = ? AND current_revision = ?")
            .bind(next).bind(next_status).bind(updated_at_ms).bind(memory_id.to_string()).bind(current)
            .execute(&mut *tx).await.map_err(|_| write_error())?;
        tx.commit().await.map_err(|_| write_error())?;
        Ok(StoredMemory {
            memory_id,
            status: if enabled {
                StoredMemoryStatus::Approved
            } else {
                StoredMemoryStatus::Disabled
            },
            kind: StoredMemoryKind::parse(
                row.try_get::<String, _>("category")
                    .map_err(|_| integrity_error())?
                    .as_str(),
            )?,
            content: content.to_owned(),
            confidence: row.try_get("confidence").map_err(|_| integrity_error())?,
            source_session_id: row
                .try_get::<Option<String>, _>("source_session_id")
                .map_err(|_| integrity_error())?
                .map(|id| parse_v7(&id))
                .transpose()?,
            created_at_ms: row
                .try_get("created_at_ms")
                .map_err(|_| integrity_error())?,
            updated_at_ms,
            approved_at_ms: row
                .try_get("decided_at_ms")
                .map_err(|_| integrity_error())?,
            last_used_at_ms: row
                .try_get("last_used_at_ms")
                .map_err(|_| integrity_error())?,
            enabled,
            revision: u64::try_from(next).map_err(|_| integrity_error())?,
        })
    }

    pub(crate) async fn reject_memory_proposal(
        &self,
        memory_id: Uuid,
        expected_revision: u64,
        rejected_at_ms: i64,
    ) -> Result<u64, StorageError> {
        let next = expected_revision.checked_add(1).ok_or_else(write_error)?;
        let result = sqlx::query("UPDATE memory_proposals SET status = 'rejected', decided_at_ms = ?, revision = ? WHERE id = ? AND status = 'proposed' AND revision = ?")
            .bind(rejected_at_ms).bind(i64::try_from(next).map_err(|_| write_error())?).bind(memory_id.to_string())
            .bind(i64::try_from(expected_revision).map_err(|_| write_error())?)
            .execute(&self.writer).await.map_err(|_| write_error())?;
        if result.rows_affected() == 1 {
            Ok(next)
        } else {
            Err(resolve_memory_write_miss(self, memory_id).await)
        }
    }

    pub(crate) async fn delete_memory(
        &self,
        memory_id: Uuid,
        expected_revision: u64,
        deleted_at_ms: i64,
    ) -> Result<u64, StorageError> {
        let mut tx = self.writer.begin().await.map_err(|_| write_error())?;
        let row: Option<(i64, String)> = sqlx::query_as(
            "SELECT current_revision, statement_hash FROM memories JOIN memory_revisions ON memory_revisions.memory_id = memories.id AND memory_revisions.revision = memories.current_revision WHERE memories.id = ? AND memories.status IN ('approved', 'disabled')",
        )
        .bind(memory_id.to_string()).fetch_optional(&mut *tx).await.map_err(|_| read_error())?;
        let Some((current, hash)) = row else {
            return Err(StorageError::new(StorageReason::EntityNotFound));
        };
        if u64::try_from(current).map_err(|_| integrity_error())? != expected_revision {
            return Err(StorageError::new(StorageReason::RevisionConflict));
        }
        let next = current.checked_add(1).ok_or_else(write_error)?;
        sqlx::query("UPDATE memory_revisions SET statement_text = NULL WHERE memory_id = ?")
            .bind(memory_id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(|_| write_error())?;
        sqlx::query("INSERT INTO memory_revisions(memory_id, revision, statement_text, statement_hash, change_kind, changed_at_ms) VALUES(?, ?, NULL, ?, 'deleted', ?)")
            .bind(memory_id.to_string()).bind(next).bind(hash).bind(deleted_at_ms).execute(&mut *tx).await.map_err(|_| write_error())?;
        sqlx::query("UPDATE memories SET current_revision = ?, status = 'deleted', updated_at_ms = ?, deleted_at_ms = ? WHERE id = ? AND current_revision = ?")
            .bind(next).bind(deleted_at_ms).bind(deleted_at_ms).bind(memory_id.to_string()).bind(current)
            .execute(&mut *tx).await.map_err(|_| write_error())?;
        tx.commit().await.map_err(|_| write_error())?;
        u64::try_from(next).map_err(|_| integrity_error())
    }

    pub(crate) async fn load_profile_view(&self) -> Result<StoredProfileView, StorageError> {
        let row = sqlx::query("SELECT display_name, city, region, country, country_code, city_lat, city_lon, timezone, routine_json, program_preferences_json, profile_revision FROM user_profile WHERE id = 'current'")
            .fetch_optional(&self.writer).await.map_err(|_| read_error())?;
        let Some(row) = row else {
            return Ok(StoredProfileView {
                display_name: String::new(),
                companion_style: "quiet_warm".to_owned(),
                initial_preferences: Vec::new(),
                narration_density: "balanced".to_owned(),
                city: None,
                region: None,
                country: None,
                country_code: None,
                latitude: None,
                longitude: None,
                timezone: None,
                revision: 0,
            });
        };
        let routine: Value = serde_json::from_str(
            row.try_get::<String, _>("routine_json")
                .map_err(|_| integrity_error())?
                .as_str(),
        )
        .map_err(|_| integrity_error())?;
        let prefs: Value = serde_json::from_str(
            row.try_get::<String, _>("program_preferences_json")
                .map_err(|_| integrity_error())?
                .as_str(),
        )
        .map_err(|_| integrity_error())?;
        let revision: i64 = row
            .try_get("profile_revision")
            .map_err(|_| integrity_error())?;
        Ok(StoredProfileView {
            display_name: row
                .try_get::<Option<String>, _>("display_name")
                .map_err(|_| integrity_error())?
                .unwrap_or_default(),
            companion_style: routine
                .get("companionStyle")
                .and_then(Value::as_str)
                .unwrap_or("quiet_warm")
                .to_owned(),
            initial_preferences: decode_preferences_value(&prefs)?,
            narration_density: prefs
                .get("narrationDensity")
                .and_then(Value::as_str)
                .unwrap_or("balanced")
                .to_owned(),
            city: row.try_get("city").map_err(|_| integrity_error())?,
            region: row.try_get("region").map_err(|_| integrity_error())?,
            country: row.try_get("country").map_err(|_| integrity_error())?,
            country_code: row.try_get("country_code").map_err(|_| integrity_error())?,
            latitude: row
                .try_get::<Option<f64>, _>("city_lat")
                .map_err(|_| integrity_error())?
                .map(|value| value.to_string()),
            longitude: row
                .try_get::<Option<f64>, _>("city_lon")
                .map_err(|_| integrity_error())?
                .map(|value| value.to_string()),
            timezone: row.try_get("timezone").map_err(|_| integrity_error())?,
            revision: u64::try_from(revision).map_err(|_| integrity_error())?,
        })
    }

    /// Returns bounded, content-free feedback aggregates for the YOU view.
    pub(crate) async fn load_preference_trends(
        &self,
        now_ms: i64,
    ) -> Result<Vec<StoredPreferenceTrend>, StorageError> {
        const WINDOW_DAYS: u32 = 30;
        const HALF_WINDOW_MS: i64 = 15 * 24 * 60 * 60 * 1_000;
        let window_start = now_ms.checked_sub(THIRTY_DAYS_MS).ok_or_else(read_error)?;
        let midpoint = now_ms.checked_sub(HALF_WINDOW_MS).ok_or_else(read_error)?;
        let profile_created_at_ms: Option<i64> =
            sqlx::query_scalar("SELECT created_at_ms FROM user_profile WHERE id = 'current'")
                .fetch_optional(&self.writer)
                .await
                .map_err(|_| read_error())?;
        let Some(profile_created_at_ms) = profile_created_at_ms else {
            return Ok(Vec::new());
        };
        let rows = sqlx::query(
            "SELECT feedback_type, \
                    SUM(CASE WHEN created_at_ms >= ? THEN 1 ELSE 0 END) AS recent_count, \
                    SUM(CASE WHEN created_at_ms < ? THEN 1 ELSE 0 END) AS prior_count, \
                    COUNT(*) AS sample_count \
             FROM feedback \
             WHERE revoked_at_ms IS NULL AND created_at_ms >= ? AND created_at_ms >= ? AND created_at_ms <= ? \
             GROUP BY feedback_type ORDER BY feedback_type ASC LIMIT 4",
        )
        .bind(midpoint)
        .bind(midpoint)
        .bind(window_start)
        .bind(profile_created_at_ms)
        .bind(now_ms)
        .fetch_all(&self.writer)
        .await
        .map_err(|_| read_error())?;
        rows.into_iter()
            .map(|row| {
                let kind: String = row
                    .try_get("feedback_type")
                    .map_err(|_| integrity_error())?;
                let label = match kind.as_str() {
                    "like" => "喜欢",
                    "skip" => "跳过",
                    "less_talk" => "少说一点",
                    "more_like_this" => "更多类似内容",
                    _ => return Err(integrity_error()),
                };
                let recent: i64 = row.try_get("recent_count").map_err(|_| integrity_error())?;
                let prior: i64 = row.try_get("prior_count").map_err(|_| integrity_error())?;
                let sample_count: i64 =
                    row.try_get("sample_count").map_err(|_| integrity_error())?;
                Ok(StoredPreferenceTrend {
                    kind,
                    label: label.to_owned(),
                    direction: match recent.cmp(&prior) {
                        std::cmp::Ordering::Greater => "up",
                        std::cmp::Ordering::Less => "down",
                        std::cmp::Ordering::Equal => "stable",
                    }
                    .to_owned(),
                    sample_count: u32::try_from(sample_count).map_err(|_| integrity_error())?,
                    window_days: WINDOW_DAYS,
                })
            })
            .collect()
    }

    pub(crate) async fn update_profile_fields(
        &self,
        expected_revision: u64,
        display_name: Option<&str>,
        companion_style: Option<&str>,
        initial_preferences: Option<&[String]>,
        narration_density: Option<&str>,
        updated_at_ms: i64,
    ) -> Result<u64, StorageError> {
        let mut tx = self.writer.begin().await.map_err(|_| write_error())?;
        let row: Option<(Option<String>, String, String, i64)> = sqlx::query_as("SELECT display_name, routine_json, program_preferences_json, profile_revision FROM user_profile WHERE id = 'current'")
            .fetch_optional(&mut *tx).await.map_err(|_| read_error())?;
        let current = row.as_ref().map_or(0, |value| value.3);
        if u64::try_from(current).map_err(|_| integrity_error())? != expected_revision {
            return Err(StorageError::new(StorageReason::RevisionConflict));
        }
        let next = current
            .checked_add(1)
            .filter(|value| *value <= MAX_JS_SAFE_INTEGER)
            .ok_or_else(write_error)?;
        let mut routine = row
            .as_ref()
            .map(|value| parse_object(&value.1))
            .transpose()?
            .unwrap_or_default();
        let mut preferences = row
            .as_ref()
            .map(|value| parse_object(&value.2))
            .transpose()?
            .unwrap_or_default();
        if let Some(value) = companion_style {
            routine.insert("companionStyle".to_owned(), Value::String(value.to_owned()));
        }
        if let Some(value) = initial_preferences {
            preferences.insert(
                "initialPreferences".to_owned(),
                serde_json::to_value(value).map_err(|_| write_error())?,
            );
        }
        if let Some(value) = narration_density {
            preferences.insert(
                "narrationDensity".to_owned(),
                Value::String(value.to_owned()),
            );
        }
        let name = display_name
            .map(ToOwned::to_owned)
            .or_else(|| row.as_ref().and_then(|value| value.0.clone()))
            .filter(|value| !value.is_empty());
        sqlx::query("INSERT INTO user_profile(id, display_name, locale, timezone, routine_json, program_preferences_json, profile_revision, created_at_ms, updated_at_ms) VALUES('current', ?, 'zh-CN', 'UTC', ?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET display_name = excluded.display_name, routine_json = excluded.routine_json, program_preferences_json = excluded.program_preferences_json, profile_revision = excluded.profile_revision, updated_at_ms = excluded.updated_at_ms")
            .bind(name).bind(serde_json::to_string(&routine).map_err(|_| write_error())?).bind(serde_json::to_string(&preferences).map_err(|_| write_error())?)
            .bind(next).bind(updated_at_ms).bind(updated_at_ms).execute(&mut *tx).await.map_err(|_| write_error())?;
        if let Some(value) = narration_density {
            sqlx::query("INSERT INTO app_settings(key, value_json, schema_version, updated_at_ms) VALUES('program.narration_density', ?, 1, ?) ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at_ms = excluded.updated_at_ms")
                .bind(serde_json::to_string(value).map_err(|_| write_error())?).bind(updated_at_ms).execute(&mut *tx).await.map_err(|_| write_error())?;
        }
        tx.commit().await.map_err(|_| write_error())?;
        u64::try_from(next).map_err(|_| integrity_error())
    }

    pub(crate) async fn list_session_summaries(
        &self,
        offset: u64,
        limit: u32,
    ) -> Result<(Vec<StoredSessionSummary>, Option<u64>), StorageError> {
        if limit == 0 || limit > 100 {
            return Err(StorageError::new(StorageReason::InvalidSetting));
        }
        let rows = sqlx::query("SELECT id, source_from_ms, source_to_ms, summary_text, generation_kind, revision FROM session_summaries WHERE status = 'active' ORDER BY updated_at_ms DESC, id ASC LIMIT ? OFFSET ?")
            .bind(i64::from(limit) + 1).bind(i64::try_from(offset).map_err(|_| StorageError::new(StorageReason::InvalidSetting))?)
            .fetch_all(&self.writer).await.map_err(|_| read_error())?;
        let has_more = rows.len() > limit as usize;
        let items = rows
            .into_iter()
            .take(limit as usize)
            .map(|row| {
                let revision: i64 = row.try_get("revision").map_err(|_| integrity_error())?;
                Ok(StoredSessionSummary {
                    summary_id: parse_v7(
                        row.try_get::<String, _>("id")
                            .map_err(|_| integrity_error())?
                            .as_str(),
                    )?,
                    covered_from_ms: row
                        .try_get("source_from_ms")
                        .map_err(|_| integrity_error())?,
                    covered_to_ms: row.try_get("source_to_ms").map_err(|_| integrity_error())?,
                    summary: row.try_get("summary_text").map_err(|_| integrity_error())?,
                    generation_kind: row
                        .try_get("generation_kind")
                        .map_err(|_| integrity_error())?,
                    revision: u64::try_from(revision).map_err(|_| integrity_error())?,
                })
            })
            .collect::<Result<Vec<_>, StorageError>>()?;
        Ok((items, has_more.then(|| offset + u64::from(limit))))
    }

    pub(crate) async fn delete_session_summary(
        &self,
        summary_id: Uuid,
        expected_revision: u64,
        deleted_at_ms: i64,
    ) -> Result<u64, StorageError> {
        let next = expected_revision.checked_add(1).ok_or_else(write_error)?;
        let result = sqlx::query("UPDATE session_summaries SET status = 'deleted', summary_text = NULL, source_hash = NULL, preference_signals_json = '[]', provider = NULL, model = NULL, prompt_version = NULL, revision = ?, updated_at_ms = ?, deleted_at_ms = ? WHERE id = ? AND status = 'active' AND revision = ?")
            .bind(i64::try_from(next).map_err(|_| write_error())?).bind(deleted_at_ms).bind(deleted_at_ms).bind(summary_id.to_string()).bind(i64::try_from(expected_revision).map_err(|_| write_error())?)
            .execute(&self.writer).await.map_err(|_| write_error())?;
        if result.rows_affected() == 1 {
            Ok(next)
        } else {
            let exists: i64 =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM session_summaries WHERE id = ?)")
                    .bind(summary_id.to_string())
                    .fetch_one(&self.writer)
                    .await
                    .map_err(|_| read_error())?;
            Err(StorageError::new(if exists == 1 {
                StorageReason::RevisionConflict
            } else {
                StorageReason::EntityNotFound
            }))
        }
    }
}

#[allow(clippy::too_many_arguments)] // Mirrors the fixed messages-table privacy/provenance columns.
async fn insert_message(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: Uuid,
    session_id: Uuid,
    role: &str,
    text: &str,
    provider: Option<&str>,
    model: Option<&str>,
    prompt_version: Option<&str>,
    created_at_ms: i64,
) -> Result<(), StorageError> {
    let expires = created_at_ms
        .checked_add(THIRTY_DAYS_MS)
        .ok_or_else(write_error)?;
    sqlx::query("INSERT INTO messages(id, chat_session_id, role, content_text, content_hash, provider, model, prompt_version, created_at_ms, expires_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(id.to_string()).bind(session_id.to_string()).bind(role).bind(text).bind(content_hash(text)).bind(provider).bind(model).bind(prompt_version).bind(created_at_ms).bind(expires)
        .execute(&mut **tx).await.map_err(|_| write_error())?;
    let (user, assistant) = if role == "user" { (1, 0) } else { (0, 1) };
    let result = sqlx::query("UPDATE chat_sessions SET covered_from_ms = CASE WHEN covered_from_ms IS NULL OR ? < covered_from_ms THEN ? ELSE covered_from_ms END, covered_to_ms = CASE WHEN covered_to_ms IS NULL OR ? > covered_to_ms THEN ? ELSE covered_to_ms END, user_message_count = user_message_count + ?, assistant_message_count = assistant_message_count + ? WHERE id = ?")
        .bind(created_at_ms).bind(created_at_ms).bind(created_at_ms).bind(created_at_ms).bind(user).bind(assistant).bind(session_id.to_string())
        .execute(&mut **tx).await.map_err(|_| write_error())?;
    if result.rows_affected() == 1 {
        Ok(())
    } else {
        Err(StorageError::new(StorageReason::EntityNotFound))
    }
}

fn decode_memory(row: &sqlx::sqlite::SqliteRow) -> Result<StoredMemory, StorageError> {
    let status: String = row
        .try_get("exposed_status")
        .map_err(|_| integrity_error())?;
    let revision: i64 = row.try_get("revision").map_err(|_| integrity_error())?;
    Ok(StoredMemory {
        memory_id: parse_v7(
            row.try_get::<String, _>("id")
                .map_err(|_| integrity_error())?
                .as_str(),
        )?,
        status: match status.as_str() {
            "proposed" => StoredMemoryStatus::Proposed,
            "approved" => StoredMemoryStatus::Approved,
            "disabled" => StoredMemoryStatus::Disabled,
            _ => return Err(integrity_error()),
        },
        kind: StoredMemoryKind::parse(
            row.try_get::<String, _>("category")
                .map_err(|_| integrity_error())?
                .as_str(),
        )?,
        content: row.try_get("content").map_err(|_| integrity_error())?,
        confidence: row.try_get("confidence").map_err(|_| integrity_error())?,
        source_session_id: row
            .try_get::<Option<String>, _>("source_session_id")
            .map_err(|_| integrity_error())?
            .map(|id| parse_v7(&id))
            .transpose()?,
        created_at_ms: row
            .try_get("created_at_ms")
            .map_err(|_| integrity_error())?,
        updated_at_ms: row
            .try_get("updated_at_ms")
            .map_err(|_| integrity_error())?,
        approved_at_ms: row
            .try_get("approved_at_ms")
            .map_err(|_| integrity_error())?,
        last_used_at_ms: row
            .try_get("last_used_at_ms")
            .map_err(|_| integrity_error())?,
        enabled: row
            .try_get::<i64, _>("enabled")
            .map_err(|_| integrity_error())?
            == 1,
        revision: u64::try_from(revision).map_err(|_| integrity_error())?,
    })
}

async fn resolve_memory_write_miss(repository: &Repository, id: Uuid) -> StorageError {
    match sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM memory_proposals WHERE id = ? UNION ALL SELECT 1 FROM memories WHERE id = ?)")
        .bind(id.to_string()).bind(id.to_string()).fetch_one(&repository.writer).await {
        Ok(1) => StorageError::new(StorageReason::RevisionConflict),
        Ok(_) => StorageError::new(StorageReason::EntityNotFound),
        Err(_) => read_error(),
    }
}

fn decode_preferences(encoded: &str) -> Result<Vec<String>, StorageError> {
    decode_preferences_value(&serde_json::from_str(encoded).map_err(|_| integrity_error())?)
}

fn decode_preferences_value(value: &Value) -> Result<Vec<String>, StorageError> {
    value
        .get("initialPreferences")
        .map_or(Ok(Vec::new()), |value| {
            value
                .as_array()
                .ok_or_else(integrity_error)?
                .iter()
                .map(|item| {
                    item.as_str()
                        .map(ToOwned::to_owned)
                        .ok_or_else(integrity_error)
                })
                .collect()
        })
}

fn parse_object(encoded: &str) -> Result<Map<String, Value>, StorageError> {
    serde_json::from_str::<Value>(encoded)
        .map_err(|_| integrity_error())?
        .as_object()
        .cloned()
        .ok_or_else(integrity_error)
}

fn content_hash(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}
fn truncate_chars(value: String, maximum: usize) -> String {
    if value.chars().count() <= maximum {
        value
    } else {
        value.chars().take(maximum).collect()
    }
}
fn parse_v7(value: &str) -> Result<Uuid, StorageError> {
    Uuid::parse_str(value)
        .ok()
        .filter(|id| id.get_version_num() == 7 && id.to_string() == value)
        .ok_or_else(integrity_error)
}
fn read_error() -> StorageError {
    StorageError::new(StorageReason::StorageReadFailed)
}
fn write_error() -> StorageError {
    StorageError::new(StorageReason::StorageWriteFailed)
}
fn integrity_error() -> StorageError {
    StorageError::new(StorageReason::StorageIntegrityFailed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{AppPaths, Storage};

    async fn fixture() -> (tempfile::TempDir, Storage, Repository, Uuid, Uuid) {
        let temp = tempfile::tempdir().expect("temp");
        let paths = AppPaths::create(
            temp.path().join("data"),
            temp.path().join("cache"),
            temp.path().join("logs"),
        )
        .expect("paths");
        let storage = Storage::open(&paths, "0.4.0").await.expect("storage");
        let repository = storage.repository();
        let program = Uuid::now_v7();
        let track = Uuid::now_v7();
        sqlx::query("INSERT INTO library_roots(id, canonical_path, path_key, display_name, enabled, created_at_ms) VALUES('root', 'fixture', 'fixture', 'Fixture', 1, 0)").execute(&repository.writer).await.expect("root");
        sqlx::query("INSERT INTO tracks(id, root_id, relative_path, relative_path_key, availability, format, file_size_bytes, modified_at_ms, duration_ms, metadata_confidence, created_at_ms, updated_at_ms) VALUES(?, 'root', 'song.mp3', 'song.mp3', 'available', 'mp3', 1, 1, 1000, 1.0, 1, 1)").bind(track.to_string()).execute(&repository.writer).await.expect("track");
        sqlx::query("INSERT INTO program_runs(id, source_kind, status, plan_schema_version, degraded_features_json, revision, created_at_ms) VALUES(?, 'local', 'music', 1, '[]', 0, 1)").bind(program.to_string()).execute(&repository.writer).await.expect("program");
        sqlx::query("INSERT INTO program_segments(id, program_run_id, ordinal, kind, track_id, status) VALUES(?, ?, 0, 'track', ?, 'completed')").bind(Uuid::now_v7().to_string()).bind(program.to_string()).bind(track.to_string()).execute(&repository.writer).await.expect("segment");
        (temp, storage, repository, program, track)
    }

    #[tokio::test]
    async fn cancellation_boundary_can_leave_user_turn_without_ai_output() {
        let (_temp, storage, repository, program, _) = fixture().await;
        let accepted = repository
            .accept_chat_message(program, "请安静一点", false, 10)
            .await
            .expect("accept");
        assert_eq!(
            repository
                .load_chat_context(accepted.session_id)
                .await
                .expect("context")
                .turns
                .len(),
            1
        );
        let counts: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM messages), (SELECT count(*) FROM memory_proposals)",
        )
        .fetch_one(&repository.writer)
        .await
        .expect("counts");
        assert_eq!(counts, (1, 0));
        storage.close().await;
    }

    #[tokio::test]
    async fn context_projection_bounds_history_and_approved_memory_without_truncating_current_turn()
    {
        let (_temp, storage, repository, program, _) = fixture().await;
        let first = repository
            .accept_chat_message(program, "第一条", false, 10)
            .await
            .expect("first turn");
        let memory_text = "记".repeat(500);
        let (_, proposals) = repository
            .persist_chat_result(
                first.session_id,
                first.message_id,
                &[],
                &"答".repeat(2_000),
                &[NewMemoryProposal {
                    kind: StoredMemoryKind::Preference,
                    content: memory_text,
                    confidence: 0.8,
                }],
                Some("fixture"),
                Some("fixture"),
                "chat-v1",
                11,
            )
            .await
            .expect("assistant and proposal");
        repository
            .approve_memory(proposals[0].memory_id, 0, 12)
            .await
            .expect("approve");
        let current_text = "今".repeat(4_000);
        repository
            .accept_chat_message(program, &current_text, false, 13)
            .await
            .expect("current turn");

        let context = repository
            .load_chat_context(first.session_id)
            .await
            .expect("bounded chat context");
        assert_eq!(context.approved_memories[0].content.chars().count(), 240);
        assert_eq!(context.turns[1].text.chars().count(), 500);
        assert_eq!(context.turns[2].text.chars().count(), 4_000);
        let (_, program_memories, _) = repository
            .load_program_planning_facts()
            .await
            .expect("bounded program context");
        assert_eq!(program_memories[0].chars().count(), 240);
        storage.close().await;
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)] // One scenario proves the full proposal-to-deletion isolation lifecycle.
    async fn context_memory_approved_disabled_deleted_and_proposed_are_strictly_isolated() {
        let (_temp, storage, repository, program, _) = fixture().await;
        let accepted = repository
            .accept_chat_message(program, "我喜欢环境音乐", false, 10)
            .await
            .expect("accept");
        let (_, proposals) = repository
            .persist_chat_result(
                accepted.session_id,
                accepted.message_id,
                &[],
                "记住了。",
                &[NewMemoryProposal {
                    kind: StoredMemoryKind::Preference,
                    content: "喜欢环境音乐".to_owned(),
                    confidence: 0.9,
                }],
                Some("fixture"),
                Some("fixture"),
                "chat-v1",
                11,
            )
            .await
            .expect("result");
        assert!(
            repository
                .load_chat_context(accepted.session_id)
                .await
                .expect("pre-approve")
                .approved_memories
                .is_empty()
        );
        let id = proposals[0].memory_id;
        let proposal_source: (String, String) = sqlx::query_as(
            "SELECT message_id, source_content_hash FROM memory_proposal_sources WHERE proposal_id = ?",
        )
        .bind(id.to_string())
        .fetch_one(&repository.writer)
        .await
        .expect("proposal source");
        assert_eq!(proposal_source.0, accepted.message_id.to_string());
        assert_eq!(proposal_source.1, content_hash("我喜欢环境音乐"));
        let approved = repository.approve_memory(id, 0, 12).await.expect("approve");
        assert_eq!(
            repository
                .load_chat_context(accepted.session_id)
                .await
                .expect("approved")
                .approved_memories
                .len(),
            1
        );
        let disabled = repository
            .update_memory(id, approved.revision, "喜欢环境音乐", false, 13)
            .await
            .expect("disable");
        assert!(
            repository
                .load_chat_context(accepted.session_id)
                .await
                .expect("disabled")
                .approved_memories
                .is_empty()
        );
        repository
            .delete_memory(id, disabled.revision, 14)
            .await
            .expect("delete");
        let old_text_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM memory_revisions WHERE memory_id = ? AND statement_text IS NOT NULL",
        )
        .bind(id.to_string())
        .fetch_one(&repository.writer)
        .await
        .expect("deleted revision bodies");
        assert_eq!(old_text_count, 0);
        let (_, repeated) = repository
            .persist_chat_result(
                accepted.session_id,
                accepted.message_id,
                &[],
                "不会重建。",
                &[NewMemoryProposal {
                    kind: StoredMemoryKind::Preference,
                    content: "喜欢环境音乐".to_owned(),
                    confidence: 0.9,
                }],
                None,
                None,
                "fallback-v1",
                15,
            )
            .await
            .expect("repeat");
        assert!(repeated.is_empty());

        let (_, different) = repository
            .persist_chat_result(
                accepted.session_id,
                accepted.message_id,
                &[],
                "可以记住新内容。",
                &[NewMemoryProposal {
                    kind: StoredMemoryKind::Preference,
                    content: "喜欢爵士乐".to_owned(),
                    confidence: 0.8,
                }],
                Some("fixture"),
                Some("fixture"),
                "chat-v1",
                16,
            )
            .await
            .expect("different proposal");
        let error = repository
            .update_memory(different[0].memory_id, 0, "喜欢环境音乐", false, 17)
            .await
            .expect_err("deleted hash cannot be restored by editing another proposal");
        assert_eq!(error.reason(), StorageReason::RevisionConflict);
        storage.close().await;
    }

    #[tokio::test]
    async fn feedback_is_immediate_and_less_talk_requires_four_completed_tracks() {
        let (_temp, storage, repository, program, track) = fixture().await;
        assert_eq!(
            repository
                .record_feedback(program, Some(track), "like", 10)
                .await
                .expect("like"),
            0
        );
        assert_eq!(
            repository
                .record_feedback(program, None, "less_talk", 11)
                .await
                .expect("less talk"),
            0
        );
        assert!(
            !repository
                .voice_allowed_after_feedback(program)
                .await
                .expect("blocked")
        );
        for ordinal in 1..=4 {
            sqlx::query("INSERT INTO program_segments(id, program_run_id, ordinal, kind, track_id, status, ended_at_ms) VALUES(?, ?, ?, 'track', ?, 'completed', ?)")
                .bind(Uuid::now_v7().to_string()).bind(program.to_string()).bind(ordinal).bind(track.to_string()).bind(20 + ordinal).execute(&repository.writer).await.expect("track segment");
        }
        assert!(
            repository
                .voice_allowed_after_feedback(program)
                .await
                .expect("allowed")
        );
        storage.close().await;
    }
}
