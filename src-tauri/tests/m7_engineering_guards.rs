//! M7 release-only engineering guards.
//!
//! These tests are ignored by default because they create large temporary
//! fixtures or exercise long virtual timelines. They never open an audio
//! device, access the network, or use production application data.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};

use chrono::{DateTime, Utc};
use cyberkindred_lib::{
    contracts::{
        ProgramPlan, ProgramPlanMode, ProgramPlanSegmentsItem, ProgramPlanTrackSegment,
        ProgramPlanVoiceSegment, ProgramPlanVoiceSegmentTrigger,
    },
    ipc::{ApiError, InternalReason, ProcessSequence},
    library::{ListTracksRequest, TrackCatalogService, TrackFilters, TrackSort},
    program::{
        CandidateLoadRequest, CandidateSelection, CandidateSelectionRequest, PlanOrigin,
        PlannedProgram, ProgramCandidate, ProgramRepository, select_candidates,
    },
    radio::{
        ConfirmedProgramStart, ManualProgramStartAuthorizer, ProgramEventState, ProgramPlayback,
        ProgramPlaybackSignal, ProgramRadioPlanner, ProgramRunPhase, ProgramSegmentPhase,
        RadioClock, RadioEvent, RadioEventSink, RadioFuture, RadioIdFactory, RadioProgramStore,
        RadioService, RadioServiceDependencies, StartProgramRequest, StartProgramTrigger,
        TextOnlyProgramSpeech, UnavailableAppleCompanion,
    },
    storage::{AppPaths, LATEST_SCHEMA_VERSION, Storage},
};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{
    ConnectOptions, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use tokio::sync::{mpsc, watch};
use uuid::Uuid;

const MIGRATIONS: [(i64, &str, &str); 4] = [
    (
        1,
        "initial_schema",
        include_str!("../migrations/V0001__initial_schema.sql"),
    ),
    (
        2,
        "scan_operations",
        include_str!("../migrations/V0002__scan_operations.sql"),
    ),
    (
        3,
        "memory_last_used",
        include_str!("../migrations/V0003__memory_last_used.sql"),
    ),
    (
        4,
        "detached_program_segments",
        include_str!("../migrations/V0004__detached_program_segments.sql"),
    ),
];

fn database_path(data_root: &std::path::Path) -> std::path::PathBuf {
    data_root.join("cyberkindred.sqlite3")
}

async fn fixture_pool(path: &std::path::Path, create: bool) -> SqlitePool {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(create)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Delete)
        .disable_statement_logging();
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("isolated SQLite fixture")
}

async fn create_schema_fixture(path: &std::path::Path, version: i64) {
    let pool = fixture_pool(path, true).await;
    for (migration_version, name, sql) in MIGRATIONS
        .iter()
        .copied()
        .take(usize::try_from(version).expect("supported fixture version"))
    {
        sqlx::raw_sql(sql)
            .execute(&pool)
            .await
            .expect("apply fixture migration");
        let checksum = hex::encode(Sha256::digest(sql.as_bytes()));
        sqlx::query(
            "INSERT INTO schema_migrations(version, name, checksum_sha256, applied_at_ms, app_version) VALUES(?, ?, ?, ?, ?)",
        )
        .bind(migration_version)
        .bind(name)
        .bind(checksum)
        .bind(migration_version)
        .bind(format!("fixture-v{migration_version}"))
        .execute(&pool)
        .await
        .expect("record fixture migration");
        let user_version_statement = match migration_version {
            1 => "PRAGMA user_version = 1",
            2 => "PRAGMA user_version = 2",
            3 => "PRAGMA user_version = 3",
            4 => "PRAGMA user_version = 4",
            _ => panic!("unsupported fixture version"),
        };
        sqlx::query(user_version_statement)
            .execute(&pool)
            .await
            .expect("set fixture user version");
    }
    sqlx::query("PRAGMA application_id = 1129008708")
        .execute(&pool)
        .await
        .expect("set fixture application id");
    let root_id = Uuid::now_v7();
    let track_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO library_roots(id, canonical_path, path_key, display_name, enabled, created_at_ms) VALUES(?, 'migration-fixture-path', 'migration-fixture-key', 'Migration fixture', 1, 1)",
    )
    .bind(root_id.to_string())
    .execute(&pool)
    .await
    .expect("insert migration root canary");
    sqlx::query(
        "INSERT INTO tracks(id, root_id, relative_path, relative_path_key, availability, format, file_size_bytes, modified_at_ms, duration_ms, title, genre_json, metadata_confidence, created_at_ms, updated_at_ms) VALUES(?, ?, 'canary.mp3', 'canary.mp3', 'available', 'mp3', 1, 1, 1000, 'migration-canary', '[]', 1.0, 1, 1)",
    )
    .bind(track_id.to_string())
    .bind(root_id.to_string())
    .execute(&pool)
    .await
    .expect("insert migration track canary");
    pool.close().await;
}

fn backup_count(data_root: &std::path::Path) -> usize {
    std::fs::read_dir(data_root.join("backups"))
        .expect("migration backup directory")
        .count()
}

// NFR-MAINT-001; TEST-MAINT-001; TASK-032.
#[tokio::test]
async fn repository_migrates_every_supported_schema_version_to_current() {
    assert_eq!(
        usize::try_from(LATEST_SCHEMA_VERSION).expect("non-negative latest schema version"),
        MIGRATIONS.len(),
        "update the M7 migration fixture matrix when a schema migration is added"
    );
    for starting_version in 0..=LATEST_SCHEMA_VERSION {
        let temporary = tempfile::tempdir().expect("temporary migration root");
        let data = temporary.path().join("data");
        let paths = AppPaths::create(
            &data,
            temporary.path().join("cache"),
            temporary.path().join("logs"),
        )
        .expect("scoped migration paths");
        let path = database_path(&data);
        if starting_version > 0 {
            create_schema_fixture(&path, starting_version).await;
        }

        let storage = Storage::open(&paths, "0.1.0")
            .await
            .unwrap_or_else(|error| panic!("upgrade from V{starting_version}: {error}"));
        storage
            .verify_integrity()
            .await
            .expect("upgraded database integrity");
        storage.close().await;

        let pool = fixture_pool(&path, false).await;
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .expect("current user version");
        assert_eq!(version, LATEST_SCHEMA_VERSION);
        let history: Vec<i64> =
            sqlx::query_scalar("SELECT version FROM schema_migrations ORDER BY version")
                .fetch_all(&pool)
                .await
                .expect("migration history");
        assert_eq!(history, vec![1, 2, 3, 4]);
        if starting_version > 0 {
            let canary: String =
                sqlx::query_scalar("SELECT title FROM tracks WHERE relative_path = 'canary.mp3'")
                    .fetch_one(&pool)
                    .await
                    .expect("preserved migration canary");
            assert_eq!(canary, "migration-canary");
        }
        pool.close().await;

        let expected_backups = usize::try_from(LATEST_SCHEMA_VERSION - starting_version)
            .expect("non-negative backup count");
        assert_eq!(backup_count(&data), expected_backups);
        let reopened = Storage::open(&paths, "0.1.0")
            .await
            .expect("current schema reopens idempotently");
        reopened.close().await;
        assert_eq!(backup_count(&data), expected_backups);
    }
}

async fn seed_scale_fixture(pool: &SqlitePool, library_root: &std::path::Path) {
    let root_id = Uuid::now_v7();
    let mut transaction = pool.begin().await.expect("scale fixture transaction");
    sqlx::query(
        "INSERT INTO library_roots(id, canonical_path, path_key, display_name, enabled, created_at_ms) VALUES(?, ?, 'm7-scale-root', 'M7 scale fixture', 1, 1)",
    )
    .bind(root_id.to_string())
    .bind(library_root.to_string_lossy().as_ref())
    .execute(&mut *transaction)
    .await
    .expect("scale fixture root");
    for index in 0..10_000_u32 {
        let name = format!("synthetic-{index:05}.mp3");
        std::fs::write(library_root.join(&name), []).expect("synthetic directory entry");
        sqlx::query(
            "INSERT INTO tracks(id, root_id, relative_path, relative_path_key, availability, format, file_size_bytes, modified_at_ms, duration_ms, title, genre_json, metadata_confidence, created_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, 'available', 'mp3', 0, 1, 180000, ?, '[\"synthetic\"]', 1.0, 1, 1)",
        )
        .bind(Uuid::now_v7().to_string())
        .bind(root_id.to_string())
        .bind(&name)
        .bind(&name)
        .bind(format!("Synthetic {index:05}"))
        .execute(&mut *transaction)
        .await
        .expect("scale fixture track");
    }
    transaction.commit().await.expect("commit scale fixture");
}

fn report_scale_result(
    insert_ms: u128,
    directory_ms: u128,
    pagination_ms: u128,
    candidate_ms: u128,
    directory_count: usize,
    pages: usize,
    selected_candidates: usize,
) {
    println!(
        "{}",
        json!({
            "schemaVersion": 1,
            "guardId": "M7-GUARD-10000",
            "blocking": false,
            "fixture": "program-generated-empty-files-and-synthetic-db-rows",
            "rows": 10_000,
            "directoryEntries": directory_count,
            "pages": pages,
            "selectedCandidates": selected_candidates,
            "timingMs": {
                "fixtureInsert": insert_ms,
                "directoryEnumerate": directory_ms,
                "catalogPaginate": pagination_ms,
                "candidateLoadAndSelect": candidate_ms
            },
            "result": "completed"
        })
    );
}

// NFR-PERF-002; NFR-MAINT-002; RISK-010; TASK-032.
#[tokio::test]
#[ignore = "M7 non-blocking 10,000-row directory/database engineering guard"]
async fn synthetic_ten_thousand_catalog_guard() {
    let temporary = tempfile::tempdir().expect("temporary scale root");
    let data = temporary.path().join("data");
    let library_root = temporary.path().join("library");
    std::fs::create_dir(&library_root).expect("synthetic library directory");
    let paths = AppPaths::create(
        &data,
        temporary.path().join("cache"),
        temporary.path().join("logs"),
    )
    .expect("scoped scale paths");
    let storage = Storage::open(&paths, "0.1.0")
        .await
        .expect("initialize scale database");
    storage.close().await;

    let insert_started = Instant::now();
    let pool = fixture_pool(&database_path(&data), false).await;
    seed_scale_fixture(&pool, &library_root).await;
    pool.close().await;
    let insert_ms = insert_started.elapsed().as_millis();

    let directory_started = Instant::now();
    let directory_count = std::fs::read_dir(&library_root)
        .expect("read synthetic directory")
        .count();
    let directory_ms = directory_started.elapsed().as_millis();
    assert_eq!(directory_count, 10_000);

    let storage = Storage::open(&paths, "0.1.0")
        .await
        .expect("reopen scale database");
    let repository = storage.repository();
    let catalog = TrackCatalogService::new(Arc::new(repository.clone()));
    let pagination_started = Instant::now();
    let mut cursor = None;
    let mut catalog_rows = 0_usize;
    let mut pages = 0_usize;
    loop {
        let page = catalog
            .list_tracks(ListTracksRequest {
                cursor,
                limit: 200,
                query: None,
                sort: TrackSort::Title,
                filters: TrackFilters {
                    availability: None,
                    match_status: None,
                },
            })
            .await
            .expect("bounded catalog page");
        catalog_rows += page.items.len();
        pages += 1;
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    let pagination_ms = pagination_started.elapsed().as_millis();
    assert_eq!(catalog_rows, 10_000);
    assert_eq!(pages, 50);

    let candidate_started = Instant::now();
    let candidates = repository
        .load_candidate_tracks(CandidateLoadRequest {
            maximum_rows: 10_001,
        })
        .await
        .expect("bounded 10,000-row candidate projection");
    assert_eq!(candidates.len(), 10_000);
    let selected = select_candidates(
        candidates,
        &CandidateSelectionRequest {
            now_ms: 1_800_000_000_000,
            local_hour: 12,
            profile_tags: vec!["synthetic".to_owned()],
            approved_memory_tags: Vec::new(),
            recently_played_track_ids: Vec::new(),
            cooldown_ms: 0,
            desired_count: 6,
            limit: 200,
            allow_cooldown_relaxation: false,
            selection_seed: 7,
        },
    )
    .expect("select bounded program candidates");
    let candidate_ms = candidate_started.elapsed().as_millis();
    assert_eq!(selected.candidates.len(), 200);
    storage.close().await;

    report_scale_result(
        insert_ms,
        directory_ms,
        pagination_ms,
        candidate_ms,
        directory_count,
        pages,
        selected.candidates.len(),
    );
}

#[derive(Default)]
struct GuardStoreState {
    phase: Option<ProgramRunPhase>,
    revision: u64,
    segments: HashMap<Uuid, ProgramSegmentPhase>,
}

struct GuardStore(Mutex<GuardStoreState>);

impl GuardStore {
    fn new() -> Self {
        Self(Mutex::new(GuardStoreState::default()))
    }
}

impl RadioProgramStore for GuardStore {
    fn begin_program(
        &self,
        _program_id: Uuid,
        _created_at_ms: i64,
    ) -> RadioFuture<'_, Result<u64, ApiError>> {
        Box::pin(async move {
            let mut state = self.0.lock().map_err(|_| ApiError::unexpected())?;
            state.phase = Some(ProgramRunPhase::Planning);
            Ok(0)
        })
    }

    fn begin_system_program(
        &self,
        program_id: Uuid,
        created_at_ms: i64,
    ) -> RadioFuture<'_, Result<u64, ApiError>> {
        self.begin_program(program_id, created_at_ms)
    }

    fn persist_plan<'a>(
        &'a self,
        planned: &'a PlannedProgram,
    ) -> RadioFuture<'a, Result<u64, ApiError>> {
        Box::pin(async move {
            let mut state = self.0.lock().map_err(|_| ApiError::unexpected())?;
            if state.phase != Some(ProgramRunPhase::Planning) {
                return Err(ApiError::from_reason(InternalReason::RevisionConflict));
            }
            state.phase = Some(ProgramRunPhase::Ready);
            state.revision = 1;
            state.segments = planned
                .plan
                .segments
                .iter()
                .map(|segment| {
                    let id = match segment {
                        ProgramPlanSegmentsItem::TrackSegment(value) => &value.segment_id,
                        ProgramPlanSegmentsItem::VoiceSegment(value) => &value.segment_id,
                    };
                    Uuid::parse_str(id)
                        .map(|id| (id, ProgramSegmentPhase::Planned))
                        .map_err(|_| ApiError::unexpected())
                })
                .collect::<Result<HashMap<_, _>, _>>()?;
            Ok(1)
        })
    }

    fn transition_program(
        &self,
        _program_id: Uuid,
        expected: ProgramRunPhase,
        next: ProgramRunPhase,
        current_revision: u64,
        _occurred_at_ms: i64,
        _failure_code: Option<&'static str>,
    ) -> RadioFuture<'_, Result<u64, ApiError>> {
        Box::pin(async move {
            let mut state = self.0.lock().map_err(|_| ApiError::unexpected())?;
            if state.phase != Some(expected) || state.revision != current_revision {
                return Err(ApiError::from_reason(InternalReason::RevisionConflict));
            }
            state.revision = state
                .revision
                .checked_add(1)
                .ok_or_else(ApiError::unexpected)?;
            state.phase = Some(next);
            Ok(state.revision)
        })
    }

    fn transition_segment(
        &self,
        _program_id: Uuid,
        segment_id: Uuid,
        expected: ProgramSegmentPhase,
        next: ProgramSegmentPhase,
        _occurred_at_ms: i64,
        _failure_code: Option<&'static str>,
    ) -> RadioFuture<'_, Result<(), ApiError>> {
        Box::pin(async move {
            let mut state = self.0.lock().map_err(|_| ApiError::unexpected())?;
            if state.segments.get(&segment_id) != Some(&expected) {
                return Err(ApiError::from_reason(InternalReason::RevisionConflict));
            }
            state.segments.insert(segment_id, next);
            Ok(())
        })
    }

    fn voice_allowed_after_feedback(
        &self,
        _program_id: Uuid,
    ) -> RadioFuture<'_, Result<bool, ApiError>> {
        Box::pin(async { Ok(true) })
    }
}

struct GuardPlanner {
    track_count: usize,
    duration_ms: u64,
}

impl ProgramRadioPlanner for GuardPlanner {
    fn plan_local(&self, program_id: Uuid) -> RadioFuture<'_, Result<PlannedProgram, ApiError>> {
        let planned = virtual_program(program_id, self.track_count, self.duration_ms);
        Box::pin(async move { Ok(planned) })
    }
}

#[derive(Default)]
struct GuardPlayback {
    started: AtomicUsize,
    completed: AtomicUsize,
}

impl ProgramPlayback for GuardPlayback {
    fn play_batch<'a>(
        &'a self,
        _program_id: Uuid,
        tracks: &'a [cyberkindred_lib::radio::ProgramTrack],
        _authorization: ConfirmedProgramStart,
        cancellation: watch::Receiver<bool>,
        signals: mpsc::Sender<ProgramPlaybackSignal>,
    ) -> RadioFuture<'a, Result<(), ApiError>> {
        Box::pin(async move {
            if *cancellation.borrow() {
                return Err(ApiError::from_reason(InternalReason::OperationCancelled));
            }
            for track in tracks {
                self.started.fetch_add(1, Ordering::SeqCst);
                signals
                    .send(ProgramPlaybackSignal::Started(track.segment_id))
                    .await
                    .map_err(|_| ApiError::unexpected())?;
                self.completed.fetch_add(1, Ordering::SeqCst);
                signals
                    .send(ProgramPlaybackSignal::Completed(track.segment_id))
                    .await
                    .map_err(|_| ApiError::unexpected())?;
            }
            Ok(())
        })
    }

    fn stop(&self) -> RadioFuture<'_, Result<(), ApiError>> {
        Box::pin(async { Ok(()) })
    }
}

#[derive(Default)]
struct GuardEvents(Mutex<Vec<RadioEvent>>);

impl RadioEventSink for GuardEvents {
    fn publish(&self, event: RadioEvent) -> Result<(), ApiError> {
        self.0
            .lock()
            .map_err(|_| ApiError::unexpected())?
            .push(event);
        Ok(())
    }
}

struct GuardClock;

impl RadioClock for GuardClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::from_timestamp_millis(1_800_000_000_000).expect("fixed guard timestamp")
    }
}

struct GuardId(Uuid);

impl RadioIdFactory for GuardId {
    fn next_id(&self) -> Uuid {
        self.0
    }
}

fn voice(text: &str, trigger: ProgramPlanVoiceSegmentTrigger) -> ProgramPlanSegmentsItem {
    ProgramPlanSegmentsItem::VoiceSegment(ProgramPlanVoiceSegment {
        r#type: "voice".to_owned(),
        segment_id: Uuid::now_v7().to_string(),
        text: text.to_owned(),
        trigger,
    })
}

fn virtual_program(program_id: Uuid, track_count: usize, duration_ms: u64) -> PlannedProgram {
    let track_ids = (0..track_count).map(|_| Uuid::now_v7()).collect::<Vec<_>>();
    let mut segments = vec![voice("opening", ProgramPlanVoiceSegmentTrigger::Opening)];
    for (index, track_id) in track_ids.iter().enumerate() {
        if index > 0 && index % 3 == 0 {
            segments.push(voice(
                "bridge",
                ProgramPlanVoiceSegmentTrigger::BetweenTracks,
            ));
        }
        segments.push(ProgramPlanSegmentsItem::TrackSegment(
            ProgramPlanTrackSegment {
                r#type: "track".to_owned(),
                segment_id: Uuid::now_v7().to_string(),
                track_id: track_id.to_string(),
                segue_text: None,
            },
        ));
    }
    PlannedProgram {
        plan: ProgramPlan {
            schema_version: "1.0.0".to_owned(),
            program_id: program_id.to_string(),
            source_id: "local".to_owned(),
            mode: ProgramPlanMode::Local,
            created_at: "2027-01-15T08:00:00.000Z".to_owned(),
            segments,
        },
        candidates: CandidateSelection {
            candidates: track_ids
                .iter()
                .map(|track_id| ProgramCandidate {
                    track_id: track_id.to_string(),
                    title: None,
                    artist: None,
                    album: None,
                    duration_ms,
                    normalized_tags: Vec::new(),
                    recent_play_penalty: 0,
                })
                .collect(),
            cooldown_relaxed: false,
        },
        origin: PlanOrigin::Deterministic,
        degradation: None,
    }
}

async fn run_virtual_program(track_count: usize, duration_ms: u64) -> serde_json::Value {
    let program_id = Uuid::now_v7();
    let playback = Arc::new(GuardPlayback::default());
    let events = Arc::new(GuardEvents::default());
    let service = RadioService::new(RadioServiceDependencies {
        planner: Arc::new(GuardPlanner {
            track_count,
            duration_ms,
        }),
        authorizer: Arc::new(ManualProgramStartAuthorizer),
        store: Arc::new(GuardStore::new()),
        playback: playback.clone(),
        speech: Arc::new(TextOnlyProgramSpeech),
        apple: Arc::new(UnavailableAppleCompanion),
        event_sink: events.clone(),
        clock: Arc::new(GuardClock),
        sequence: Arc::new(ProcessSequence::default()),
        id_factory: Arc::new(GuardId(program_id)),
    });
    service
        .start_local_program(StartProgramRequest {
            client_request_id: Uuid::now_v7(),
            source_id: "local".to_owned(),
            trigger: StartProgramTrigger::Manual,
        })
        .await
        .expect("start hermetic virtual program");
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while service.active_program_id().await.is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("virtual runner terminates");
    let completed_event = events
        .0
        .lock()
        .expect("guard events")
        .iter()
        .any(|event| {
            matches!(event, RadioEvent::ProgramState(state) if state.state == ProgramEventState::Completed)
        });
    let started = playback.started.load(Ordering::SeqCst);
    let completed = playback.completed.load(Ordering::SeqCst);
    assert!(completed_event);
    assert_eq!(started, track_count);
    assert_eq!(completed, track_count);
    json!({
        "logicalDurationMs": u64::try_from(track_count).expect("track count") * duration_ms,
        "tracks": track_count,
        "started": started,
        "completed": completed,
        "adjacentTransitions": track_count.saturating_sub(1),
        "successfulTransitions": completed.saturating_sub(1),
        "audioMeasured": false,
        "networkUsed": false,
        "result": "completed"
    })
}

// NFR-REL-001; TEST-RAD-005; TASK-032.
#[tokio::test]
#[ignore = "M7 hermetic accelerated ten-minute program guard"]
async fn accelerated_ten_minute_programs() {
    let started = Instant::now();
    let mut runs = Vec::new();
    for _ in 0..10 {
        runs.push(run_virtual_program(4, 150_000).await);
    }
    println!(
        "{}",
        json!({
            "schemaVersion": 1,
            "guardId": "M7-GUARD-10X10MIN",
            "blocking": true,
            "kind": "hermetic-accelerated-state-machine",
            "runs": runs,
            "wallClockMs": started.elapsed().as_millis(),
            "realAudioClaimed": false,
            "result": "completed"
        })
    );
}

// NFR-REL-001; TEST-RAD-005; TASK-032.
#[tokio::test]
#[ignore = "M7 non-blocking thirty-minute virtual-timeline engineering guard"]
async fn virtual_thirty_minute_program_guard() {
    let started = Instant::now();
    let run = run_virtual_program(12, 150_000).await;
    println!(
        "{}",
        json!({
            "schemaVersion": 1,
            "guardId": "M7-GUARD-30MIN-VIRTUAL",
            "blocking": false,
            "kind": "hermetic-virtual-state-machine",
            "run": run,
            "wallClockMs": started.elapsed().as_millis(),
            "realAudioClaimed": false,
            "result": "completed"
        })
    );
}
