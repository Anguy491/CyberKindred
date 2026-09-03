use super::{Repository, StorageError, StorageReason};
use crate::contracts::ScheduleRule;
use uuid::Uuid;

const SETTINGS_SCHEMA_VERSION: i64 = 1;
const MAX_JS_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
type ScheduleRow = (
    String,
    String,
    bool,
    String,
    String,
    String,
    bool,
    Option<i64>,
    i64,
    i64,
    i64,
);

#[derive(Clone, Debug)]
pub(crate) struct StoredSchedule {
    pub rule: ScheduleRule,
    pub next_occurrence_at_ms: Option<i64>,
}

#[derive(Clone, Debug)]
pub(crate) struct StoredScheduleCollection {
    pub schedules: Vec<StoredSchedule>,
    pub revision: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct StoredDueOccurrence {
    pub schedule_id: Uuid,
    pub occurrence_id: Uuid,
    pub name: String,
    pub due_at_ms: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StoredNotificationAction {
    Open,
    Dismiss,
    Start,
    Snooze(u8),
}

#[derive(Clone, Debug)]
pub(crate) struct ScheduleActionResult {
    pub status: String,
    pub next_notification_at_ms: Option<i64>,
    pub revision: u64,
    pub newly_starting: bool,
}

impl Repository {
    pub(crate) async fn load_schedules(&self) -> Result<StoredScheduleCollection, StorageError> {
        let rows: Vec<ScheduleRow> = sqlx::query_as(
            "SELECT id, name, enabled, timezone, days_of_week_json, local_time, notification_only, next_occurrence_at_ms, revision, created_at_ms, updated_at_ms FROM schedule_rules ORDER BY created_at_ms, id",
        )
        .fetch_all(&self.writer)
        .await
        .map_err(|_| read_error())?;
        let schedules = rows
            .into_iter()
            .map(stored_schedule_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let revision = load_collection_revision(&self.writer).await?;
        Ok(StoredScheduleCollection {
            schedules,
            revision,
        })
    }

    pub(crate) async fn upsert_schedule(
        &self,
        expected_revision: u64,
        requested: &ScheduleRule,
        next_occurrence_at_ms: Option<i64>,
        now_ms: i64,
    ) -> Result<(StoredSchedule, u64), StorageError> {
        let mut transaction = self.writer.begin().await.map_err(|_| write_error())?;
        let current_revision = load_collection_revision(&mut *transaction).await?;
        if current_revision != expected_revision {
            return Err(StorageError::new(StorageReason::RevisionConflict));
        }
        let requested_id = Uuid::parse_str(&requested.schedule_id).map_err(|_| write_error())?;
        let existing: Option<(i64, i64)> =
            sqlx::query_as("SELECT revision, created_at_ms FROM schedule_rules WHERE id = ?")
                .bind(requested_id.to_string())
                .fetch_optional(&mut *transaction)
                .await
                .map_err(|_| read_error())?;
        let (schedule_id, schedule_revision, created_at_ms) =
            if let Some((stored_revision, created)) = existing {
                let stored_revision =
                    u64::try_from(stored_revision).map_err(|_| integrity_error())?;
                if requested.revision != stored_revision {
                    return Err(StorageError::new(StorageReason::RevisionConflict));
                }
                (
                    requested_id,
                    stored_revision
                        .checked_add(1)
                        .filter(|value| *value <= MAX_JS_SAFE_INTEGER)
                        .ok_or_else(write_error)?,
                    created,
                )
            } else {
                if requested.revision != 0 {
                    return Err(StorageError::new(StorageReason::EntityNotFound));
                }
                (Uuid::now_v7(), 1, now_ms)
            };
        let days = serde_json::to_string(&requested.days_of_week).map_err(|_| write_error())?;
        sqlx::query(
            "INSERT INTO schedule_rules(id, name, enabled, timezone, days_of_week_json, local_time, notification_only, next_occurrence_at_ms, revision, created_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?, 1, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET name = excluded.name, enabled = excluded.enabled, timezone = excluded.timezone, days_of_week_json = excluded.days_of_week_json, local_time = excluded.local_time, notification_only = 1, next_occurrence_at_ms = excluded.next_occurrence_at_ms, revision = excluded.revision, updated_at_ms = excluded.updated_at_ms",
        )
        .bind(schedule_id.to_string())
        .bind(&requested.name)
        .bind(requested.enabled)
        .bind(&requested.timezone)
        .bind(days)
        .bind(&requested.local_time)
        .bind(next_occurrence_at_ms)
        .bind(i64::try_from(schedule_revision).map_err(|_| write_error())?)
        .bind(created_at_ms)
        .bind(now_ms)
        .execute(&mut *transaction)
        .await
        .map_err(|_| write_error())?;
        let next_revision =
            bump_collection_revision(&mut transaction, current_revision, now_ms).await?;
        transaction.commit().await.map_err(|_| write_error())?;
        let rule = ScheduleRule {
            schema_version: requested.schema_version.clone(),
            schedule_id: schedule_id.to_string(),
            name: requested.name.clone(),
            timezone: requested.timezone.clone(),
            days_of_week: requested.days_of_week.clone(),
            local_time: requested.local_time.clone(),
            enabled: requested.enabled,
            notification_only: true,
            created_at: timestamp(created_at_ms)?,
            updated_at: timestamp(now_ms)?,
            revision: schedule_revision,
        };
        Ok((
            StoredSchedule {
                rule,
                next_occurrence_at_ms,
            },
            next_revision,
        ))
    }

    pub(crate) async fn delete_schedule(
        &self,
        schedule_id: Uuid,
        expected_revision: u64,
        now_ms: i64,
    ) -> Result<u64, StorageError> {
        let mut transaction = self.writer.begin().await.map_err(|_| write_error())?;
        let current_revision = load_collection_revision(&mut *transaction).await?;
        if current_revision != expected_revision {
            return Err(StorageError::new(StorageReason::RevisionConflict));
        }
        let deleted = sqlx::query("DELETE FROM schedule_rules WHERE id = ?")
            .bind(schedule_id.to_string())
            .execute(&mut *transaction)
            .await
            .map_err(|_| write_error())?;
        if deleted.rows_affected() != 1 {
            return Err(StorageError::new(StorageReason::EntityNotFound));
        }
        let revision = bump_collection_revision(&mut transaction, current_revision, now_ms).await?;
        transaction.commit().await.map_err(|_| write_error())?;
        Ok(revision)
    }

    pub(crate) async fn claim_due_schedule(
        &self,
        schedule_id: Uuid,
        expected_due_at_ms: i64,
        occurrence_key: &str,
        next_occurrence_at_ms: Option<i64>,
        now_ms: i64,
    ) -> Result<Option<StoredDueOccurrence>, StorageError> {
        let mut transaction = self.writer.begin().await.map_err(|_| write_error())?;
        let name: Option<String> = sqlx::query_scalar(
            "SELECT name FROM schedule_rules WHERE id = ? AND enabled = 1 AND next_occurrence_at_ms = ?",
        )
        .bind(schedule_id.to_string())
        .bind(expected_due_at_ms)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| read_error())?;
        let Some(name) = name else {
            return Ok(None);
        };
        sqlx::query("UPDATE schedule_rules SET next_occurrence_at_ms = ? WHERE id = ? AND next_occurrence_at_ms = ?")
            .bind(next_occurrence_at_ms)
            .bind(schedule_id.to_string())
            .bind(expected_due_at_ms)
            .execute(&mut *transaction)
            .await
            .map_err(|_| write_error())?;
        let occurrence_id = Uuid::now_v7();
        let inserted = sqlx::query(
            "INSERT OR IGNORE INTO schedule_occurrences(id, rule_id, occurrence_key, due_at_ms, status, updated_at_ms) VALUES(?, ?, ?, ?, 'due', ?)",
        )
        .bind(occurrence_id.to_string())
        .bind(schedule_id.to_string())
        .bind(occurrence_key)
        .bind(expected_due_at_ms)
        .bind(now_ms)
        .execute(&mut *transaction)
        .await
        .map_err(|_| write_error())?;
        transaction.commit().await.map_err(|_| write_error())?;
        Ok(
            (inserted.rows_affected() == 1).then_some(StoredDueOccurrence {
                schedule_id,
                occurrence_id,
                name,
                due_at_ms: expected_due_at_ms,
            }),
        )
    }

    pub(crate) async fn load_due_snoozes(
        &self,
        now_ms: i64,
    ) -> Result<Vec<StoredDueOccurrence>, StorageError> {
        type Row = (String, String, String, i64);
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT occurrence.rule_id, occurrence.id, rule.name, occurrence.snoozed_until_ms FROM schedule_occurrences AS occurrence JOIN schedule_rules AS rule ON rule.id = occurrence.rule_id WHERE occurrence.status = 'snoozed' AND occurrence.snoozed_until_ms <= ? ORDER BY occurrence.snoozed_until_ms, occurrence.id",
        )
        .bind(now_ms)
        .fetch_all(&self.writer)
        .await
        .map_err(|_| read_error())?;
        rows.into_iter()
            .map(|(schedule_id, occurrence_id, name, due_at_ms)| {
                Ok(StoredDueOccurrence {
                    schedule_id: Uuid::parse_str(&schedule_id).map_err(|_| integrity_error())?,
                    occurrence_id: Uuid::parse_str(&occurrence_id)
                        .map_err(|_| integrity_error())?,
                    name,
                    due_at_ms,
                })
            })
            .collect()
    }

    pub(crate) async fn mark_occurrence_notification(
        &self,
        occurrence_id: Uuid,
        shown: bool,
        notification_id: Option<&str>,
        now_ms: i64,
    ) -> Result<bool, StorageError> {
        let status = if shown { "awaiting_user" } else { "missed" };
        let updated = sqlx::query(
            "UPDATE schedule_occurrences SET status = ?, notification_id = ?, updated_at_ms = ? WHERE id = ? AND status IN ('due', 'snoozed')",
        )
        .bind(status)
        .bind(notification_id)
        .bind(now_ms)
        .bind(occurrence_id.to_string())
        .execute(&self.writer)
        .await
        .map_err(|_| write_error())?;
        Ok(updated.rows_affected() == 1)
    }

    pub(crate) async fn handle_notification_action(
        &self,
        schedule_id: Uuid,
        occurrence_id: Uuid,
        action: StoredNotificationAction,
        now_ms: i64,
    ) -> Result<ScheduleActionResult, StorageError> {
        let mut transaction = self.writer.begin().await.map_err(|_| write_error())?;
        let status: Option<String> = sqlx::query_scalar(
            "SELECT status FROM schedule_occurrences WHERE id = ? AND rule_id = ?",
        )
        .bind(occurrence_id.to_string())
        .bind(schedule_id.to_string())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| read_error())?;
        let Some(current_status) = status else {
            return Err(StorageError::new(StorageReason::EntityNotFound));
        };
        let revision = load_collection_revision(&mut *transaction).await?;
        let (next_status, snooze_minutes, snoozed_until_ms, newly_starting) = match action {
            StoredNotificationAction::Open if current_status == "awaiting_user" => {
                ("awaiting_user", None, None, false)
            }
            StoredNotificationAction::Dismiss
                if matches!(current_status.as_str(), "awaiting_user" | "snoozed") =>
            {
                ("dismissed", None, None, false)
            }
            StoredNotificationAction::Start if current_status == "awaiting_user" => {
                ("starting", None, None, true)
            }
            StoredNotificationAction::Start if current_status == "starting" => {
                ("starting", None, None, false)
            }
            StoredNotificationAction::Snooze(minutes)
                if matches!(current_status.as_str(), "awaiting_user" | "snoozed") =>
            {
                let next = now_ms
                    .checked_add(i64::from(minutes) * 60 * 1_000)
                    .ok_or_else(write_error)?;
                ("snoozed", Some(minutes), Some(next), false)
            }
            _ => return Err(StorageError::new(StorageReason::RevisionConflict)),
        };
        sqlx::query(
            "UPDATE schedule_occurrences SET status = ?, snooze_minutes = ?, snoozed_until_ms = ?, updated_at_ms = ? WHERE id = ? AND rule_id = ?",
        )
        .bind(next_status)
        .bind(snooze_minutes)
        .bind(snoozed_until_ms)
        .bind(now_ms)
        .bind(occurrence_id.to_string())
        .bind(schedule_id.to_string())
        .execute(&mut *transaction)
        .await
        .map_err(|_| write_error())?;
        transaction.commit().await.map_err(|_| write_error())?;
        Ok(ScheduleActionResult {
            status: next_status.to_owned(),
            next_notification_at_ms: snoozed_until_ms,
            revision,
            newly_starting,
        })
    }

    pub(crate) async fn consume_notification_start(
        &self,
        occurrence_id: Uuid,
        now_ms: i64,
    ) -> Result<bool, StorageError> {
        let updated = sqlx::query(
            "UPDATE schedule_occurrences SET status = 'consumed', updated_at_ms = ? WHERE id = ? AND status = 'starting'",
        )
        .bind(now_ms)
        .bind(occurrence_id.to_string())
        .execute(&self.writer)
        .await
        .map_err(|_| write_error())?;
        Ok(updated.rows_affected() == 1)
    }
}

fn stored_schedule_from_row(row: ScheduleRow) -> Result<StoredSchedule, StorageError> {
    let revision = u64::try_from(row.8).map_err(|_| integrity_error())?;
    let days_of_week = serde_json::from_str(&row.4).map_err(|_| integrity_error())?;
    Ok(StoredSchedule {
        rule: ScheduleRule {
            schema_version: crate::ipc::IPC_SCHEMA_VERSION.to_owned(),
            schedule_id: Uuid::parse_str(&row.0)
                .map_err(|_| integrity_error())?
                .to_string(),
            name: row.1,
            timezone: row.3,
            days_of_week,
            local_time: row.5,
            enabled: row.2,
            notification_only: row.6,
            created_at: timestamp(row.9)?,
            updated_at: timestamp(row.10)?,
            revision,
        },
        next_occurrence_at_ms: row.7,
    })
}

async fn load_collection_revision<'e, E>(executor: E) -> Result<u64, StorageError>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let encoded: Option<String> = sqlx::query_scalar(
        "SELECT value_json FROM app_settings WHERE key = 'ui.schedules_revision'",
    )
    .fetch_optional(executor)
    .await
    .map_err(|_| read_error())?;
    encoded
        .as_deref()
        .map(serde_json::from_str::<u64>)
        .transpose()
        .map_err(|_| integrity_error())
        .map(|value| value.unwrap_or(0))
}

async fn bump_collection_revision(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    current: u64,
    now_ms: i64,
) -> Result<u64, StorageError> {
    let next = current
        .checked_add(1)
        .filter(|value| *value <= MAX_JS_SAFE_INTEGER)
        .ok_or_else(write_error)?;
    let encoded = serde_json::to_string(&next).map_err(|_| write_error())?;
    sqlx::query(
        "INSERT INTO app_settings(key, value_json, schema_version, updated_at_ms) VALUES('ui.schedules_revision', ?, ?, ?) ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, schema_version = excluded.schema_version, updated_at_ms = excluded.updated_at_ms",
    )
    .bind(encoded)
    .bind(SETTINGS_SCHEMA_VERSION)
    .bind(now_ms)
    .execute(&mut **transaction)
    .await
    .map_err(|_| write_error())?;
    Ok(next)
}

fn timestamp(timestamp_ms: i64) -> Result<String, StorageError> {
    use chrono::{SecondsFormat, TimeZone, Utc};
    Utc.timestamp_millis_opt(timestamp_ms)
        .single()
        .map(|value| value.to_rfc3339_opts(SecondsFormat::Millis, true))
        .ok_or_else(integrity_error)
}

const fn read_error() -> StorageError {
    StorageError::new(StorageReason::StorageReadFailed)
}

const fn write_error() -> StorageError {
    StorageError::new(StorageReason::StorageWriteFailed)
}

const fn integrity_error() -> StorageError {
    StorageError::new(StorageReason::StorageIntegrityFailed)
}
