use chrono::{SecondsFormat, Utc};
use sqlx::{Row, Sqlite, Transaction};

use super::{Repository, StorageError, StorageReason};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct InventoryCounts {
    pub profile_and_preferences: u64,
    pub weather_location_and_cache: u64,
    pub library_roots_and_identity: u64,
    pub embedded_music_tags: u64,
    pub metadata_matches: u64,
    pub artwork_and_tts_cache: u64,
    pub playback_history_and_feedback: u64,
    pub chat_messages: u64,
    pub voice_segment_text: u64,
    pub session_summaries: u64,
    pub memory_proposals: u64,
    pub approved_memories_and_revisions: u64,
    pub schedules_and_notifications: u64,
    pub provider_usage_facts: u64,
    pub operation_outbox: u64,
}

impl Repository {
    pub(crate) async fn load_inventory_counts(&self) -> Result<InventoryCounts, StorageError> {
        Ok(InventoryCounts {
            profile_and_preferences: self.count("SELECT (SELECT count(*) FROM user_profile) + (SELECT count(*) FROM app_settings)").await?,
            weather_location_and_cache: self.count("SELECT (SELECT count(*) FROM user_profile WHERE city IS NOT NULL OR city_lat IS NOT NULL) + (SELECT count(*) FROM weather_cache)").await?,
            library_roots_and_identity: self.count("SELECT (SELECT count(*) FROM library_roots) + (SELECT count(*) FROM tracks)").await?,
            embedded_music_tags: self.count("SELECT count(*) FROM tracks WHERE title IS NOT NULL OR artist IS NOT NULL OR album IS NOT NULL OR genre_json != '[]'").await?,
            metadata_matches: self.count("SELECT (SELECT count(*) FROM track_external_metadata) + (SELECT count(*) FROM cover_art_negative_cache)").await?,
            artwork_and_tts_cache: self.count("SELECT (SELECT count(*) FROM cover_art_cache) + (SELECT count(*) FROM tts_cache_entries)").await?,
            playback_history_and_feedback: self.count("SELECT (SELECT count(*) FROM program_runs) + (SELECT count(*) FROM playback_events) + (SELECT count(*) FROM feedback)").await?,
            chat_messages: self.count("SELECT count(*) FROM messages").await?,
            voice_segment_text: self.count("SELECT count(*) FROM program_segments WHERE kind = 'voice' AND voice_text IS NOT NULL").await?,
            session_summaries: self.count("SELECT count(*) FROM session_summaries").await?,
            memory_proposals: self.count("SELECT count(*) FROM memory_proposals").await?,
            approved_memories_and_revisions: self.count("SELECT (SELECT count(*) FROM memories WHERE status != 'deleted') + (SELECT count(*) FROM memory_revisions WHERE statement_text IS NOT NULL)").await?,
            schedules_and_notifications: self.count("SELECT (SELECT count(*) FROM schedule_rules) + (SELECT count(*) FROM schedule_occurrences)").await?,
            provider_usage_facts: self.count("SELECT count(*) FROM provider_usage").await?,
            operation_outbox: self.count("SELECT count(*) FROM outbox_events").await?,
        })
    }

    pub(crate) async fn category_count(&self, category: &str) -> Result<u64, StorageError> {
        let sql = match category {
            "profile_and_memories" => {
                "SELECT (SELECT count(*) FROM user_profile) + (SELECT count(*) FROM app_settings WHERE key LIKE 'program.%' OR key LIKE 'privacy.%') + (SELECT count(*) FROM memory_proposals) + (SELECT count(*) FROM memories) + (SELECT count(*) FROM memory_revisions)"
            }
            "conversations_and_summaries" => {
                "SELECT (SELECT count(*) FROM chat_sessions) + (SELECT count(*) FROM messages) + (SELECT count(*) FROM session_summaries)"
            }
            "playback_history" => {
                "SELECT (SELECT count(*) FROM program_runs) + (SELECT count(*) FROM program_segments) + (SELECT count(*) FROM playback_events) + (SELECT count(*) FROM feedback)"
            }
            "metadata_cache" => {
                "SELECT (SELECT count(*) FROM track_external_metadata) + (SELECT count(*) FROM cover_art_cache) + (SELECT count(*) FROM cover_art_negative_cache) + (SELECT count(*) FROM tts_cache_entries) + (SELECT count(*) FROM weather_cache)"
            }
            "library_index" => {
                "SELECT (SELECT count(*) FROM library_roots) + (SELECT count(*) FROM scan_jobs) + (SELECT count(*) FROM scan_operations) + (SELECT count(*) FROM tracks)"
            }
            _ => return Err(invalid()),
        };
        self.count(sql).await
    }

    pub(crate) async fn delete_category(&self, category: &str) -> Result<u64, StorageError> {
        let mut transaction = self.writer.begin().await.map_err(|_| write_failed())?;
        let deleted = match category {
            "profile_and_memories" => delete_all(
                &mut transaction,
                &[
                    "DELETE FROM memory_proposal_sources",
                    "DELETE FROM memory_revisions",
                    "DELETE FROM memories",
                    "DELETE FROM memory_proposals",
                    "DELETE FROM user_profile",
                    "DELETE FROM app_settings WHERE key LIKE 'program.%' OR key LIKE 'privacy.%'",
                    "DELETE FROM outbox_events",
                ],
            )
            .await?,
            "conversations_and_summaries" => {
                delete_all(
                    &mut transaction,
                    &[
                        "DELETE FROM session_summaries",
                        "DELETE FROM memory_proposal_sources WHERE message_id IS NOT NULL",
                        "DELETE FROM messages",
                        "DELETE FROM chat_sessions",
                        "DELETE FROM outbox_events",
                    ],
                )
                .await?
            }
            "playback_history" => {
                delete_all(
                    &mut transaction,
                    &[
                        "DELETE FROM tts_cache_references WHERE owner_kind = 'program_segment'",
                        "DELETE FROM feedback",
                        "DELETE FROM playback_events",
                        "DELETE FROM program_segments",
                        "DELETE FROM program_runs",
                        "UPDATE tracks SET last_played_at_ms = NULL",
                        "DELETE FROM outbox_events",
                    ],
                )
                .await?
            }
            "metadata_cache" => {
                delete_all(
                    &mut transaction,
                    &[
                        "DELETE FROM tts_cache_references",
                        "DELETE FROM tts_cache_leases",
                        "DELETE FROM tts_cache_entries",
                        "DELETE FROM cover_art_cache",
                        "DELETE FROM cover_art_negative_cache",
                        "DELETE FROM track_external_metadata",
                        "DELETE FROM weather_cache",
                        "DELETE FROM outbox_events",
                    ],
                )
                .await?
            }
            "library_index" => {
                delete_all(
                    &mut transaction,
                    &[
                        "DELETE FROM scan_operation_roots",
                        "DELETE FROM scan_jobs",
                        "DELETE FROM tracks",
                        "DELETE FROM library_roots",
                        "DELETE FROM scan_operations",
                        "DELETE FROM outbox_events",
                    ],
                )
                .await?
            }
            _ => return Err(invalid()),
        };
        transaction.commit().await.map_err(|_| write_failed())?;
        Ok(deleted)
    }

    pub(crate) async fn export_user_data_jsonl(&self) -> Result<Vec<u8>, StorageError> {
        let generated_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        let mut lines = vec![serde_json::json!({
            "recordType": "manifest",
            "schemaVersion": crate::ipc::IPC_SCHEMA_VERSION,
            "exportVersion": 1,
            "generatedAt": generated_at,
        })];
        for query in EXPORT_QUERIES {
            let rows = sqlx::query(*query)
                .fetch_all(&self.writer)
                .await
                .map_err(|_| read_failed())?;
            for row in rows {
                let encoded: String = row.try_get(0).map_err(|_| read_failed())?;
                lines.push(serde_json::from_str(&encoded).map_err(|_| read_failed())?);
            }
        }
        let mut output = Vec::new();
        for line in lines {
            serde_json::to_writer(&mut output, &line).map_err(|_| read_failed())?;
            output.push(b'\n');
        }
        Ok(output)
    }

    async fn count(&self, sql: &'static str) -> Result<u64, StorageError> {
        let value: i64 = sqlx::query_scalar(sql)
            .fetch_one(&self.writer)
            .await
            .map_err(|_| read_failed())?;
        u64::try_from(value).map_err(|_| read_failed())
    }
}

const EXPORT_QUERIES: &[&str] = &[
    "SELECT json_object('recordType','profile','displayName',display_name,'locale',locale,'city',city,'region',region,'country',country,'countryCode',country_code,'latitude',city_lat,'longitude',city_lon,'timezone',timezone,'routine',json(routine_json),'programPreferences',json(program_preferences_json),'revision',profile_revision,'createdAtMs',created_at_ms,'updatedAtMs',updated_at_ms) FROM user_profile ORDER BY id",
    "SELECT json_object('recordType','setting','key',key,'value',json(value_json),'schemaVersion',schema_version,'updatedAtMs',updated_at_ms) FROM app_settings ORDER BY key",
    "SELECT json_object('recordType','memory','memoryId',m.id,'category',m.category,'status',m.status,'content',r.statement_text,'revision',m.current_revision,'createdAtMs',m.created_at_ms,'updatedAtMs',m.updated_at_ms) FROM memories m JOIN memory_revisions r ON r.memory_id=m.id AND r.revision=m.current_revision WHERE m.status IN ('approved','disabled') AND r.statement_text IS NOT NULL ORDER BY m.created_at_ms,m.id",
    "SELECT json_object('recordType','memoryProposalStatus','proposalId',id,'category',category,'status',status,'confidence',confidence,'createdAtMs',created_at_ms,'decidedAtMs',decided_at_ms) FROM memory_proposals ORDER BY created_at_ms,id",
    "SELECT json_object('recordType','sessionSummary','summaryId',id,'coveredFromMs',source_from_ms,'coveredToMs',source_to_ms,'summary',summary_text,'generationKind',generation_kind,'revision',revision,'createdAtMs',created_at_ms,'updatedAtMs',updated_at_ms) FROM session_summaries WHERE status='active' ORDER BY created_at_ms,id",
    "SELECT json_object('recordType','conversationAggregate','coveredFromMs',covered_from_ms,'coveredToMs',covered_to_ms,'userMessageCount',user_message_count,'assistantMessageCount',assistant_message_count,'startedAtMs',started_at_ms,'endedAtMs',ended_at_ms,'status',status) FROM chat_sessions ORDER BY started_at_ms,id",
    "SELECT json_object('recordType','schedule','scheduleId',id,'name',name,'enabled',json(CASE enabled WHEN 1 THEN 'true' ELSE 'false' END),'timezone',timezone,'daysOfWeek',json(days_of_week_json),'localTime',local_time,'notificationOnly',json('true'),'revision',revision,'createdAtMs',created_at_ms,'updatedAtMs',updated_at_ms) FROM schedule_rules ORDER BY created_at_ms,id",
    "SELECT json_object('recordType','program','programId',id,'sourceKind',source_kind,'sourceInstanceId',source_instance_id,'status',status,'startedAtMs',started_at_ms,'endedAtMs',ended_at_ms,'createdAtMs',created_at_ms) FROM program_runs ORDER BY created_at_ms,id",
    "SELECT json_object('recordType','playbackEvent','sourceKind',source_kind,'sourceInstanceId',source_instance_id,'trackId',track_id,'systemMediaIdentity',system_media_identity,'displayTitle',display_title,'displayArtist',display_artist,'displayAlbum',display_album,'eventType',event_type,'positionMs',position_ms,'reasonCode',reason_code,'occurredAtMs',occurred_at_ms,'playbackRevision',playback_revision) FROM playback_events ORDER BY occurred_at_ms,id",
    "SELECT json_object('recordType','feedback','targetKind',target_kind,'trackId',track_id,'systemMediaIdentity',system_media_identity,'feedbackType',feedback_type,'value',json(value_json),'createdAtMs',created_at_ms,'revokedAtMs',revoked_at_ms) FROM feedback ORDER BY created_at_ms,id",
];

async fn delete_all(
    transaction: &mut Transaction<'_, Sqlite>,
    statements: &[&'static str],
) -> Result<u64, StorageError> {
    let mut count = 0_u64;
    for statement in statements {
        let affected = sqlx::query(*statement)
            .execute(&mut **transaction)
            .await
            .map_err(|_| write_failed())?
            .rows_affected();
        count = count.checked_add(affected).ok_or_else(write_failed)?;
    }
    Ok(count)
}

const fn invalid() -> StorageError {
    StorageError::new(StorageReason::InvalidSetting)
}

const fn read_failed() -> StorageError {
    StorageError::new(StorageReason::StorageReadFailed)
}

const fn write_failed() -> StorageError {
    StorageError::new(StorageReason::StorageWriteFailed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{AppPaths, ChatRole, NewChatMessage, Storage};

    async fn fixture() -> (tempfile::TempDir, Storage, Repository) {
        let temp = tempfile::tempdir().expect("temporary root");
        let paths = AppPaths::create(
            temp.path().join("data"),
            temp.path().join("cache"),
            temp.path().join("logs"),
        )
        .expect("paths");
        let storage = Storage::open(&paths, "0.6.0").await.expect("storage");
        let repository = storage.repository();
        (temp, storage, repository)
    }

    #[tokio::test]
    async fn data_inventory_counts_all_database_classes_without_returning_content() {
        let (_temp, storage, repository) = fixture().await;
        repository
            .create_chat_session("session", 1)
            .await
            .expect("session");
        repository
            .insert_message(NewChatMessage::new(
                "message".to_owned(),
                "session".to_owned(),
                ChatRole::User,
                "private inventory canary".to_owned(),
                None,
                None,
                None,
                1,
            ))
            .await
            .expect("message");

        let counts = repository.load_inventory_counts().await.expect("inventory");
        assert_eq!(counts.chat_messages, 1);
        assert_eq!(counts.library_roots_and_identity, 0);
        storage.close().await;
    }

    #[tokio::test]
    async fn data_export_excludes_raw_chat_paths_voice_provider_usage_and_outbox() {
        const CHAT_CANARY: &str = "PRIVATE_CHAT_CANARY_9a331";
        const PATH_CANARY: &str = r"C:\Users\Canary\Secret Music";
        const VOICE_CANARY: &str = "PRIVATE_VOICE_CANARY_8d271";
        const PROPOSAL_CANARY: &str = "PRIVATE_PROPOSAL_CANARY_3b112";
        const USAGE_CANARY: &str = "PRIVATE_CORRELATION_CANARY_1f991";
        const OUTBOX_CANARY: &str = "PRIVATE_OUTBOX_CANARY_4c551";

        let (_temp, storage, repository) = fixture().await;
        repository
            .create_chat_session("session", 1)
            .await
            .expect("session");
        repository
            .insert_message(NewChatMessage::new(
                "message".to_owned(),
                "session".to_owned(),
                ChatRole::User,
                CHAT_CANARY.to_owned(),
                None,
                None,
                None,
                1,
            ))
            .await
            .expect("message");
        sqlx::query("INSERT INTO library_roots(id,canonical_path,path_key,display_name,enabled,created_at_ms) VALUES('root',?1,'root','Canary',1,1)")
            .bind(PATH_CANARY)
            .execute(&repository.writer)
            .await
            .expect("root");
        sqlx::query("INSERT INTO program_runs(id,source_kind,status,plan_schema_version,degraded_features_json,revision,created_at_ms) VALUES('program','local','completed',1,'[]',0,1)")
            .execute(&repository.writer)
            .await
            .expect("program");
        sqlx::query("INSERT INTO program_segments(id,program_run_id,ordinal,kind,voice_text,voice_text_hash,status,ended_at_ms) VALUES('segment','program',0,'voice',?1,?2,'completed',2)")
            .bind(VOICE_CANARY)
            .bind("a".repeat(64))
            .execute(&repository.writer)
            .await
            .expect("voice segment");
        sqlx::query("INSERT INTO memory_proposals(id,category,statement_text,statement_hash,reason_text,confidence,status,created_at_ms,prompt_version) VALUES('proposal','preference',?1,?2,'reason',0.8,'proposed',1,'v1')")
            .bind(PROPOSAL_CANARY)
            .bind("b".repeat(64))
            .execute(&repository.writer)
            .await
            .expect("proposal");
        sqlx::query("INSERT INTO provider_usage(id,provider,request_kind,latency_ms,status_class,correlation_id,created_at_ms) VALUES('usage','openai','program',1,'ok',?1,1)")
            .bind(USAGE_CANARY)
            .execute(&repository.writer)
            .await
            .expect("usage");
        sqlx::query("INSERT INTO outbox_events(id,aggregate_type,aggregate_id,aggregate_revision,event_type,payload_json,created_at_ms) VALUES('outbox','fixture','fixture',1,'fixture.event',?1,1)")
            .bind(format!(r#"{{"canary":"{OUTBOX_CANARY}"}}"#))
            .execute(&repository.writer)
            .await
            .expect("outbox");

        let export = String::from_utf8(repository.export_user_data_jsonl().await.expect("export"))
            .expect("UTF-8");
        assert!(export.contains(r#""recordType":"manifest""#));
        assert!(export.contains(r#""recordType":"conversationAggregate""#));
        for forbidden in [
            CHAT_CANARY,
            PATH_CANARY,
            VOICE_CANARY,
            PROPOSAL_CANARY,
            USAGE_CANARY,
            OUTBOX_CANARY,
        ] {
            assert!(
                !export.contains(forbidden),
                "export leaked a forbidden canary"
            );
        }
        storage.close().await;
    }

    #[tokio::test]
    async fn data_deletion_removes_library_index_without_deleting_source_music() {
        let (temp, storage, repository) = fixture().await;
        let source = temp.path().join("source-track.mp3");
        std::fs::write(&source, b"source music canary").expect("source fixture");
        sqlx::query("INSERT INTO library_roots(id,canonical_path,path_key,display_name,enabled,created_at_ms) VALUES('root',?1,'root','Canary',1,1)")
            .bind(source.parent().expect("source parent").to_string_lossy().as_ref())
            .execute(&repository.writer)
            .await
            .expect("root");

        assert_eq!(
            repository
                .category_count("library_index")
                .await
                .expect("count"),
            1
        );
        assert_eq!(
            repository
                .delete_category("library_index")
                .await
                .expect("delete"),
            1
        );
        assert_eq!(
            repository
                .category_count("library_index")
                .await
                .expect("empty"),
            0
        );
        assert!(source.exists(), "source music must remain untouched");
        storage.close().await;
    }
}
