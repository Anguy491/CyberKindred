use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

use chrono::{DateTime, TimeZone, Utc};
use cyberkindred_lib::{
    contracts::ProgramPlanSegmentsItem,
    ipc::ProcessSequence,
    library::{
        LibraryRootClock, LibraryRootPicker, LibraryRootPickerError, LibraryRootPickerFuture,
        LibraryRootService, PickAndAddLibraryRootRequest,
    },
    program::{ProgramPlanner, ProgramRepository, SystemProgramClock, SystemProgramIdFactory},
    radio::{
        ConfirmedProgramStart, DomainProgramPlanner, ManualProgramStartAuthorizer,
        ProgramEventState, ProgramPlannerContextSource, ProgramPlayback, ProgramPlaybackSignal,
        ProgramRadioPlanner, RadioClock, RadioEvent, RadioEventSink, RadioPlanningContext,
        RadioProgramStore, RadioService, RadioServiceDependencies, StartProgramRequest,
        StartProgramTrigger, StopProgramRequest, SystemRadioIdFactory, TextOnlyProgramSpeech,
        UnavailableAppleCompanion,
    },
    scanner::{
        CancelLibraryScanRequest, CancelLibraryScanState, ScanClock, ScanEvent, ScanEventSink,
        ScanEventState, ScannerService, StartLibraryScanRequest,
    },
    storage::{AppPaths, Storage},
};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{ConnectOptions, Connection, Row, sqlite::SqliteConnectOptions};
use tempfile::TempDir;
use tokio::sync::{Notify, mpsc, watch};
use uuid::Uuid;

const SUPPORTED_FIXTURES: [&str; 6] = [
    "tone.mp3",
    "tone.flac",
    "tone.m4a",
    "tone.aac",
    "tone.wav",
    "tone.ogg",
];
const CORRUPT_FIXTURE: &str = "corrupt.mp3";
const UNSUPPORTED_FIXTURE: &str = "unsupported.txt";

#[derive(Clone)]
struct FixedClock {
    now_ms: i64,
}

impl LibraryRootClock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.now_ms
    }
}

impl ScanClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        Utc.timestamp_millis_opt(self.now_ms)
            .single()
            .unwrap_or_else(|| panic!("fixed scan clock must be valid"))
    }
}

impl RadioClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        Utc.timestamp_millis_opt(self.now_ms)
            .single()
            .unwrap_or_else(|| panic!("fixed radio clock must be valid"))
    }
}

struct FixedPicker {
    selected: PathBuf,
}

impl LibraryRootPicker for FixedPicker {
    fn pick_directory(&self) -> LibraryRootPickerFuture<'_> {
        let selected = self.selected.clone();
        Box::pin(async move { Ok::<_, LibraryRootPickerError>(Some(selected)) })
    }
}

#[derive(Default)]
struct RecordingEvents {
    events: Mutex<Vec<ScanEvent>>,
    observed_at: Mutex<Vec<(Uuid, ScanEventState, Instant)>>,
    changed: Notify,
    fail_terminal_once: AtomicBool,
}

impl RecordingEvents {
    fn snapshot(&self) -> Vec<ScanEvent> {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn fail_next_terminal(&self) {
        self.fail_terminal_once.store(true, Ordering::Release);
    }

    fn observations(&self, operation_id: Uuid) -> Vec<(ScanEventState, Instant)> {
        self.observed_at
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter_map(|(recorded_id, state, observed_at)| {
                (*recorded_id == operation_id).then_some((*state, *observed_at))
            })
            .collect()
    }

    async fn wait_for_terminal(&self, operation_id: Uuid) -> ScanEvent {
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if let Some(event) = self.snapshot().into_iter().find(|event| {
                    event.operation_id == operation_id
                        && matches!(
                            event.state,
                            ScanEventState::Completed
                                | ScanEventState::Cancelled
                                | ScanEventState::Failed
                        )
                }) {
                    return event;
                }
                self.changed.notified().await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("scan operation {operation_id} did not become terminal"))
    }
}

impl ScanEventSink for RecordingEvents {
    fn publish(&self, event: ScanEvent) -> Result<(), cyberkindred_lib::ipc::ApiError> {
        if event.state.is_terminal() && self.fail_terminal_once.swap(false, Ordering::AcqRel) {
            return Err(cyberkindred_lib::ipc::ApiError::unexpected());
        }
        let observation = (event.operation_id, event.state, Instant::now());
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event);
        self.observed_at
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(observation);
        self.changed.notify_one();
        Ok(())
    }
}

#[derive(Default)]
struct RecordingRadioEvents {
    events: Mutex<Vec<RadioEvent>>,
    changed: Notify,
}

impl RecordingRadioEvents {
    fn snapshot(&self) -> Vec<RadioEvent> {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    async fn wait_for_state(&self, program_id: Uuid, expected: ProgramEventState) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if self.snapshot().iter().any(|event| {
                    matches!(event, RadioEvent::ProgramState(state)
                        if state.program_id == program_id && state.state == expected)
                }) {
                    return;
                }
                self.changed.notified().await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("program {program_id} did not reach {expected:?}"));
    }
}

impl RadioEventSink for RecordingRadioEvents {
    fn publish(&self, event: RadioEvent) -> Result<(), cyberkindred_lib::ipc::ApiError> {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event);
        self.changed.notify_waiters();
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct FixtureProgramContext;

impl ProgramPlannerContextSource for FixtureProgramContext {
    fn load_context(
        &self,
        program_id: Uuid,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<RadioPlanningContext, cyberkindred_lib::ipc::ApiError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            Ok(RadioPlanningContext {
                local_hour: 21,
                profile_tags: vec!["fixture".to_owned()],
                approved_memory_tags: Vec::new(),
                recently_played_track_ids: Vec::new(),
                cooldown_ms: 30 * 24 * 60 * 60 * 1_000,
                allow_cooldown_relaxation: true,
                selection_seed: u64::from_be_bytes(
                    program_id.as_bytes()[..8]
                        .try_into()
                        .unwrap_or_else(|_| panic!("UUID prefix is eight bytes")),
                ),
            })
        })
    }
}

struct LicensedFixturePlayback {
    calls: AtomicUsize,
    stop_calls: AtomicUsize,
    signalled_tracks: AtomicUsize,
    block_at_call: AtomicUsize,
    played_track_ids: Mutex<Vec<String>>,
}

impl Default for LicensedFixturePlayback {
    fn default() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            stop_calls: AtomicUsize::new(0),
            signalled_tracks: AtomicUsize::new(0),
            block_at_call: AtomicUsize::new(usize::MAX),
            played_track_ids: Mutex::new(Vec::new()),
        }
    }
}

impl LicensedFixturePlayback {
    async fn wait_for_calls(&self, expected: usize) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.calls.load(Ordering::Acquire) < expected {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("playback did not reach {expected} batches"));
    }

    async fn wait_for_signalled_tracks(&self, expected: usize) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.signalled_tracks.load(Ordering::Acquire) < expected {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("playback did not signal {expected} tracks"));
    }
}

impl ProgramPlayback for LicensedFixturePlayback {
    fn play_batch<'a>(
        &'a self,
        _program_id: Uuid,
        tracks: &'a [cyberkindred_lib::radio::ProgramTrack],
        _authorization: ConfirmedProgramStart,
        mut cancellation: watch::Receiver<bool>,
        signals: mpsc::Sender<ProgramPlaybackSignal>,
    ) -> Pin<Box<dyn Future<Output = Result<(), cyberkindred_lib::ipc::ApiError>> + Send + 'a>>
    {
        let call = self.calls.fetch_add(1, Ordering::AcqRel) + 1;
        let segments = tracks
            .iter()
            .map(|track| track.segment_id)
            .collect::<Vec<_>>();
        self.played_track_ids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend(tracks.iter().map(|track| track.track_id.clone()));
        Box::pin(async move {
            for segment in segments {
                signals
                    .send(ProgramPlaybackSignal::Started(segment))
                    .await
                    .map_err(|_| cyberkindred_lib::ipc::ApiError::unexpected())?;
                signals
                    .send(ProgramPlaybackSignal::Completed(segment))
                    .await
                    .map_err(|_| cyberkindred_lib::ipc::ApiError::unexpected())?;
                self.signalled_tracks.fetch_add(1, Ordering::AcqRel);
            }
            if call >= self.block_at_call.load(Ordering::Acquire) {
                while !*cancellation.borrow() {
                    cancellation
                        .changed()
                        .await
                        .map_err(|_| cyberkindred_lib::ipc::ApiError::unexpected())?;
                }
                return Err(cyberkindred_lib::ipc::ApiError::unexpected());
            }
            Ok(())
        })
    }

    fn stop(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<(), cyberkindred_lib::ipc::ApiError>> + Send + '_>>
    {
        self.stop_calls.fetch_add(1, Ordering::AcqRel);
        Box::pin(async { Ok(()) })
    }
}

struct ScannerFixture {
    _temp: TempDir,
    storage: Storage,
    service: ScannerService,
    events: Arc<RecordingEvents>,
    library_root: PathBuf,
    app_data: PathBuf,
    app_cache: PathBuf,
    root_id: Uuid,
}

impl ScannerFixture {
    async fn create() -> Self {
        Self::create_with_library(copy_licensed_fixtures).await
    }

    async fn create_with_library(populate: impl FnOnce(&Path)) -> Self {
        let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temporary root: {error}"));
        let library_root = temp.path().join("licensed-library");
        populate(&library_root);
        let app_data = temp.path().join("app-data");
        let app_cache = temp.path().join("app-cache");
        let app_logs = temp.path().join("app-logs");
        let paths = AppPaths::create(&app_data, &app_cache, &app_logs)
            .unwrap_or_else(|error| panic!("scoped app paths: {error}"));
        let storage = Storage::open(&paths, "0.1.0")
            .await
            .unwrap_or_else(|error| panic!("scanner storage: {error}"));
        let repository = storage.repository();
        let clock = Arc::new(FixedClock {
            now_ms: 1_780_000_000_000,
        });
        let root_service = LibraryRootService::new(
            repository.clone(),
            Arc::new(FixedPicker {
                selected: library_root.clone(),
            }),
            clock.clone(),
        );
        let root = root_service
            .pick_and_add_library_root(PickAndAddLibraryRootRequest {
                client_request_id: Uuid::now_v7(),
            })
            .await
            .unwrap_or_else(|_| panic!("authorize fixture root"))
            .root
            .unwrap_or_else(|| panic!("fixture picker must return one root"));
        let events = Arc::new(RecordingEvents::default());
        let event_sink: Arc<dyn ScanEventSink> = events.clone();
        let scan_clock: Arc<dyn ScanClock> = clock;
        let service = ScannerService::new(
            repository.clone(),
            event_sink,
            scan_clock,
            Arc::new(ProcessSequence::default()),
        );
        Self {
            _temp: temp,
            storage,
            service,
            events,
            library_root,
            app_data,
            app_cache,
            root_id: root.root_id,
        }
    }

    async fn scan_to_terminal(&self) -> ScanEvent {
        let accepted = self
            .service
            .start_scan(StartLibraryScanRequest {
                client_request_id: Uuid::now_v7(),
                root_ids: vec![self.root_id],
            })
            .await
            .unwrap_or_else(|_| panic!("start fixture scan"));
        self.events.wait_for_terminal(accepted.operation_id).await
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileFacts {
    size: u64,
    modified: Option<SystemTime>,
    sha256: String,
}

fn copy_licensed_fixtures(destination: &Path) {
    fs::create_dir_all(destination)
        .unwrap_or_else(|error| panic!("create fixture library: {error}"));
    let source = licensed_fixture_source();
    for name in SUPPORTED_FIXTURES.into_iter().chain([CORRUPT_FIXTURE]) {
        fs::copy(source.join(name), destination.join(name))
            .unwrap_or_else(|error| panic!("copy licensed fixture {name}: {error}"));
    }
    fs::write(
        destination.join(UNSUPPORTED_FIXTURE),
        b"CyberKindred generated unsupported scanner fixture\n",
    )
    .unwrap_or_else(|error| panic!("write unsupported fixture: {error}"));
}

fn copy_m7_hundred_track_fixture(destination: &Path) {
    let source = licensed_fixture_source();
    for index in 0..100_usize {
        let source_name = SUPPORTED_FIXTURES[index % SUPPORTED_FIXTURES.len()];
        let extension = Path::new(source_name)
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or_else(|| panic!("fixture extension for {source_name}"));
        let directory = destination.join(format!("disc-{:02}", index / 10));
        fs::create_dir_all(&directory)
            .unwrap_or_else(|error| panic!("create M7 fixture directory: {error}"));
        fs::copy(
            source.join(source_name),
            directory.join(format!("track-{index:03}.{extension}")),
        )
        .unwrap_or_else(|error| panic!("copy M7 fixture {index}: {error}"));
    }
}

fn licensed_fixture_source() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("spikes")
        .join("audio")
        .join("fixtures")
}

fn snapshot_files(root: &Path) -> BTreeMap<PathBuf, FileFacts> {
    let mut files = BTreeMap::new();
    visit_files(root, &mut |path| {
        let bytes = fs::read(path).unwrap_or_else(|error| panic!("read fixture file: {error}"));
        let metadata =
            fs::metadata(path).unwrap_or_else(|error| panic!("read fixture metadata: {error}"));
        files.insert(
            path.strip_prefix(root)
                .unwrap_or_else(|error| panic!("relative fixture path: {error}"))
                .to_path_buf(),
            FileFacts {
                size: metadata.len(),
                modified: metadata.modified().ok(),
                sha256: hex::encode(Sha256::digest(bytes)),
            },
        );
    });
    files
}

fn artifact_hashes(root: &Path) -> BTreeSet<String> {
    let mut hashes = BTreeSet::new();
    visit_files(root, &mut |path| {
        let bytes = fs::read(path).unwrap_or_else(|error| panic!("read app artifact: {error}"));
        hashes.insert(hex::encode(Sha256::digest(bytes)));
    });
    hashes
}

fn visit_files(root: &Path, visitor: &mut impl FnMut(&Path)) {
    if !root.exists() {
        return;
    }
    let entries =
        fs::read_dir(root).unwrap_or_else(|error| panic!("read fixture directory: {error}"));
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| panic!("read fixture entry: {error}"));
        let path = entry.path();
        if entry
            .file_type()
            .unwrap_or_else(|error| panic!("read fixture entry type: {error}"))
            .is_dir()
        {
            visit_files(&path, visitor);
        } else {
            visitor(&path);
        }
    }
}

fn consume_terminal_once(consumed: &mut HashSet<Uuid>, event: &ScanEvent) -> bool {
    matches!(
        event.state,
        ScanEventState::Completed | ScanEventState::Cancelled | ScanEventState::Failed
    ) && consumed.insert(event.operation_id)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TrackFacts {
    id: Uuid,
    relative_path: String,
    file_identity: Option<String>,
    availability: String,
    format: String,
    file_size_bytes: u64,
    modified_at_ms: i64,
    duration_ms: u64,
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    embedded_cover_hash: Option<String>,
}

async fn load_tracks(fixture: &ScannerFixture) -> Vec<TrackFacts> {
    let options = SqliteConnectOptions::new()
        .filename(fixture.app_data.join("cyberkindred.sqlite3"))
        .read_only(true)
        .foreign_keys(true)
        .disable_statement_logging();
    let mut connection = sqlx::SqliteConnection::connect_with(&options)
        .await
        .unwrap_or_else(|error| panic!("open scanner verification connection: {error}"));
    let rows = sqlx::query(
        "SELECT id, relative_path, file_identity, availability, format, file_size_bytes, modified_at_ms, duration_ms, title, artist, album, embedded_cover_hash FROM tracks WHERE root_id = ? ORDER BY relative_path_key",
    )
    .bind(fixture.root_id.to_string())
    .fetch_all(&mut connection)
    .await
    .unwrap_or_else(|error| panic!("load scanner track facts: {error}"));
    connection
        .close()
        .await
        .unwrap_or_else(|error| panic!("close scanner verification connection: {error}"));
    rows.into_iter()
        .map(|row| TrackFacts {
            id: Uuid::parse_str(row.get::<String, _>("id").as_str())
                .unwrap_or_else(|error| panic!("stored track id: {error}")),
            relative_path: row.get("relative_path"),
            file_identity: row.get("file_identity"),
            availability: row.get("availability"),
            format: row.get("format"),
            file_size_bytes: u64::try_from(row.get::<i64, _>("file_size_bytes"))
                .unwrap_or_else(|error| panic!("stored file size: {error}")),
            modified_at_ms: row.get("modified_at_ms"),
            duration_ms: u64::try_from(row.get::<i64, _>("duration_ms"))
                .unwrap_or_else(|error| panic!("stored duration: {error}")),
            title: row.get("title"),
            artist: row.get("artist"),
            album: row.get("album"),
            embedded_cover_hash: row.get("embedded_cover_hash"),
        })
        .collect()
}

async fn load_operation_state(fixture: &ScannerFixture, operation_id: Uuid) -> String {
    let options = SqliteConnectOptions::new()
        .filename(fixture.app_data.join("cyberkindred.sqlite3"))
        .read_only(true)
        .disable_statement_logging();
    let mut connection = sqlx::SqliteConnection::connect_with(&options)
        .await
        .unwrap_or_else(|error| panic!("open operation verification connection: {error}"));
    let state: String = sqlx::query_scalar("SELECT status FROM scan_operations WHERE id = ?")
        .bind(operation_id.to_string())
        .fetch_one(&mut connection)
        .await
        .unwrap_or_else(|error| panic!("load operation state: {error}"));
    connection
        .close()
        .await
        .unwrap_or_else(|error| panic!("close operation verification connection: {error}"));
    state
}

async fn wait_for_operation_terminal(fixture: &ScannerFixture, operation_id: Uuid) -> String {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let state = load_operation_state(fixture, operation_id).await;
            if matches!(
                state.as_str(),
                "completed" | "cancelled" | "failed" | "interrupted"
            ) {
                return state;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("operation {operation_id} did not commit a terminal state"))
}

fn track_map(tracks: Vec<TrackFacts>) -> BTreeMap<String, TrackFacts> {
    tracks
        .into_iter()
        .map(|track| (track.relative_path.clone(), track))
        .collect()
}

#[derive(Debug)]
struct ScanMeasurement {
    terminal: ScanEvent,
    elapsed: Duration,
    running_events: usize,
    maximum_progress_gap: Duration,
}

async fn measure_scan(fixture: &ScannerFixture) -> ScanMeasurement {
    let started_at = Instant::now();
    let accepted = fixture
        .service
        .start_scan(StartLibraryScanRequest {
            client_request_id: Uuid::now_v7(),
            root_ids: vec![fixture.root_id],
        })
        .await
        .unwrap_or_else(|_| panic!("accept measured scan"));
    let terminal = fixture
        .events
        .wait_for_terminal(accepted.operation_id)
        .await;
    let finished_at = Instant::now();
    let observations = fixture.events.observations(accepted.operation_id);
    let mut previous = started_at;
    let mut maximum_progress_gap = Duration::ZERO;
    let mut running_events = 0_usize;
    for (state, observed_at) in observations {
        if state != ScanEventState::Running {
            continue;
        }
        running_events += 1;
        maximum_progress_gap = maximum_progress_gap.max(observed_at.duration_since(previous));
        previous = observed_at;
    }
    maximum_progress_gap = maximum_progress_gap.max(finished_at.duration_since(previous));
    ScanMeasurement {
        terminal,
        elapsed: finished_at.duration_since(started_at),
        running_events,
        maximum_progress_gap,
    }
}

fn assert_scan_performance(measurement: &ScanMeasurement, label: &str) {
    assert_eq!(measurement.terminal.state, ScanEventState::Completed);
    assert!(
        measurement.elapsed <= Duration::from_secs(120),
        "{label} exceeded the two-minute scan limit: {:?}",
        measurement.elapsed
    );
    assert!(
        measurement.running_events > 0,
        "{label} did not publish running progress"
    );
    if measurement.elapsed > Duration::from_millis(500) {
        assert!(
            measurement.maximum_progress_gap <= Duration::from_millis(500),
            "{label} progress gap exceeded 500 ms: {:?}",
            measurement.maximum_progress_gap
        );
    }
}

struct ThreeScanEvidence {
    baseline: BTreeMap<String, TrackFacts>,
    elapsed_ms: Vec<u128>,
    progress_gap_ms: Vec<u128>,
    running_events: Vec<usize>,
}

async fn run_three_hundred_track_scans(fixture: &ScannerFixture) -> ThreeScanEvidence {
    let mut baseline = None;
    let mut elapsed_ms = Vec::with_capacity(3);
    let mut progress_gap_ms = Vec::with_capacity(3);
    let mut running_events = Vec::with_capacity(3);
    for run in 1..=3 {
        let measurement = measure_scan(fixture).await;
        assert_scan_performance(&measurement, &format!("full scan {run}"));
        assert_eq!(measurement.terminal.scanned, 100);
        assert_eq!(measurement.terminal.discovered, 100);
        assert_eq!(measurement.terminal.failed, 0);
        let tracks = track_map(load_tracks(fixture).await);
        assert_eq!(tracks.len(), 100);
        assert!(
            tracks
                .values()
                .all(|track| track.availability == "available")
        );
        assert_eq!(
            tracks
                .values()
                .map(|track| track.format.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["aac", "flac", "m4a", "mp3", "ogg", "wav"])
        );
        if let Some(expected) = &baseline {
            assert_eq!(
                &tracks, expected,
                "unchanged scan {run} changed catalog facts"
            );
        } else {
            baseline = Some(tracks);
        }
        elapsed_ms.push(measurement.elapsed.as_millis());
        progress_gap_ms.push(measurement.maximum_progress_gap.as_millis());
        running_events.push(measurement.running_events);
    }
    ThreeScanEvidence {
        baseline: baseline.unwrap_or_else(|| panic!("three scans produce a baseline")),
        elapsed_ms,
        progress_gap_ms,
        running_events,
    }
}

async fn cancel_and_rescan_hundred_tracks(
    fixture: &ScannerFixture,
    baseline: &BTreeMap<String, TrackFacts>,
) -> (u128, ScanMeasurement) {
    let accepted = fixture
        .service
        .start_scan(StartLibraryScanRequest {
            client_request_id: Uuid::now_v7(),
            root_ids: vec![fixture.root_id],
        })
        .await
        .unwrap_or_else(|_| panic!("accept cancellable M7 scan"));
    let cancel_started = Instant::now();
    let response = fixture
        .service
        .cancel_scan(CancelLibraryScanRequest {
            client_request_id: Uuid::now_v7(),
            operation_id: accepted.operation_id,
        })
        .await
        .unwrap_or_else(|_| panic!("cancel M7 scan"));
    let cancel_ms = cancel_started.elapsed().as_millis();
    assert!(cancel_ms <= 1_000, "cancel acceptance exceeded one second");
    assert_eq!(response.state, CancelLibraryScanState::Cancelled);
    let terminal = fixture
        .events
        .wait_for_terminal(accepted.operation_id)
        .await;
    assert_eq!(terminal.state, ScanEventState::Cancelled);
    assert_eq!(
        load_operation_state(fixture, accepted.operation_id).await,
        "cancelled"
    );
    assert_eq!(
        &track_map(load_tracks(fixture).await),
        baseline,
        "cancellation changed the committed catalog"
    );

    let recovery = measure_scan(fixture).await;
    assert_scan_performance(&recovery, "post-cancel unchanged rescan");
    assert_eq!(recovery.terminal.scanned, 100);
    assert_eq!(recovery.terminal.discovered, 100);
    assert_eq!(recovery.terminal.failed, 0);
    assert_eq!(&track_map(load_tracks(fixture).await), baseline);
    (cancel_ms, recovery)
}

async fn scan_after_mutation(fixture: &ScannerFixture, label: &str) -> ScanMeasurement {
    let measurement = measure_scan(fixture).await;
    assert_scan_performance(&measurement, label);
    assert_eq!(measurement.terminal.failed, 0);
    measurement
}

async fn exercise_m7_scan_mutations(fixture: &ScannerFixture) -> serde_json::Value {
    let source = licensed_fixture_source();
    let added_path = fixture.library_root.join("disc-10/track-100.aac");
    fs::create_dir_all(
        added_path
            .parent()
            .unwrap_or_else(|| panic!("added fixture parent")),
    )
    .unwrap_or_else(|error| panic!("create added fixture directory: {error}"));
    fs::copy(source.join("tone.aac"), &added_path)
        .unwrap_or_else(|error| panic!("add fixture track: {error}"));
    let added = scan_after_mutation(fixture, "added-track rescan").await;
    let after_add = track_map(load_tracks(fixture).await);
    assert_eq!(after_add.len(), 101);
    assert_eq!(after_add["disc-10/track-100.aac"].availability, "available");

    let modified_name = "disc-00/track-000.mp3";
    let modified_id = after_add[modified_name].id;
    let original_size = after_add[modified_name].file_size_bytes;
    let modified_path = fixture.library_root.join(modified_name);
    let mut bytes = fs::read(&modified_path)
        .unwrap_or_else(|error| panic!("read modified M7 fixture: {error}"));
    bytes.extend_from_slice(b"cyberkindred-m7-generated-padding");
    fs::write(&modified_path, bytes)
        .unwrap_or_else(|error| panic!("write modified M7 fixture: {error}"));
    let modified = scan_after_mutation(fixture, "modified-track rescan").await;
    let after_modify = track_map(load_tracks(fixture).await);
    assert_eq!(after_modify[modified_name].id, modified_id);
    assert!(after_modify[modified_name].file_size_bytes > original_size);

    let moved_name = "disc-00/track-001.flac";
    let moved_id = after_modify[moved_name].id;
    let moved_path = fixture.library_root.join("moved/track-001.flac");
    fs::create_dir_all(
        moved_path
            .parent()
            .unwrap_or_else(|| panic!("moved fixture parent")),
    )
    .unwrap_or_else(|error| panic!("create move directory: {error}"));
    fs::rename(fixture.library_root.join(moved_name), &moved_path)
        .unwrap_or_else(|error| panic!("move M7 fixture: {error}"));
    let moved = scan_after_mutation(fixture, "moved-track rescan").await;
    let after_move = track_map(load_tracks(fixture).await);
    assert!(!after_move.contains_key(moved_name));
    assert_eq!(after_move["moved/track-001.flac"].id, moved_id);

    let deleted_name = "disc-00/track-002.m4a";
    let deleted_id = after_move[deleted_name].id;
    fs::remove_file(fixture.library_root.join(deleted_name))
        .unwrap_or_else(|error| panic!("delete M7 fixture: {error}"));
    let deleted = scan_after_mutation(fixture, "deleted-track rescan").await;
    let after_delete = track_map(load_tracks(fixture).await);
    assert_eq!(after_delete[deleted_name].id, deleted_id);
    assert_eq!(after_delete[deleted_name].availability, "missing");
    assert_eq!(
        after_delete
            .values()
            .filter(|track| track.availability == "available")
            .count(),
        100
    );

    json!({
        "addMs": added.elapsed.as_millis(),
        "modifyMs": modified.elapsed.as_millis(),
        "moveMs": moved.elapsed.as_millis(),
        "deleteMs": deleted.elapsed.as_millis()
    })
}

fn assert_event_contains_no_absolute_path(event: &ScanEvent, forbidden: &Path) {
    let value = serde_json::to_value(event)
        .unwrap_or_else(|error| panic!("serialize path-free scan event: {error}"));
    let forbidden = forbidden.to_string_lossy();
    let mut strings = Vec::new();
    collect_json_strings(&value, &mut strings);
    assert!(
        strings
            .iter()
            .all(|value| !value.contains(forbidden.as_ref())),
        "EVT-005 must not expose the authorized absolute root"
    );
}

fn collect_json_strings<'a>(value: &'a serde_json::Value, output: &mut Vec<&'a str>) {
    match value {
        serde_json::Value::String(value) => output.push(value),
        serde_json::Value::Array(values) => {
            for value in values {
                collect_json_strings(value, output);
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values() {
                collect_json_strings(value, output);
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
    }
}

fn assert_radio_fallback_evidence(events: &[RadioEvent], forbidden_root: &Path) {
    let safe_messages = events
        .iter()
        .filter_map(|event| match event {
            RadioEvent::ProgramState(state) => state.safe_message.as_deref(),
            RadioEvent::ProgramSegment(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        safe_messages
            .iter()
            .filter(|message| message.contains("确定性本地队列"))
            .count(),
        1
    );
    assert_eq!(
        safe_messages
            .iter()
            .filter(|message| message.contains("语音不可用"))
            .count(),
        1
    );
    let event_json = events
        .iter()
        .map(|event| match event {
            RadioEvent::ProgramState(state) => serde_json::to_string(state),
            RadioEvent::ProgramSegment(segment) => serde_json::to_string(segment),
        })
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|error| panic!("serialize radio evidence: {error}"))
        .join("\n");
    assert!(!event_json.contains(forbidden_root.to_string_lossy().as_ref()));
    assert!(!event_json.contains("secret"));
}

// TEST-RAD-001/003/004; FR-LIB-001; FR-RAD-001/002/003/007; NFR-COST-002; NFR-PRIV-003.
#[tokio::test]
async fn m3_licensed_fixture_scans_plans_and_runs_six_tracks_with_text_fallback() {
    let fixture = ScannerFixture::create().await;
    let terminal = fixture.scan_to_terminal().await;
    assert_eq!(terminal.state, ScanEventState::Completed);

    let available = load_tracks(&fixture)
        .await
        .into_iter()
        .filter(|track| track.availability == "available")
        .collect::<Vec<_>>();
    assert_eq!(available.len(), 6);
    let licensed_ids = available
        .iter()
        .map(|track| track.id.to_string())
        .collect::<BTreeSet<_>>();
    let repository = fixture.storage.repository();
    let program_repository: Arc<dyn ProgramRepository> = Arc::new(repository.clone());
    let planner = Arc::new(ProgramPlanner::new(
        program_repository,
        None,
        Arc::new(SystemProgramClock),
        Arc::new(SystemProgramIdFactory),
    ));
    let planner: Arc<dyn ProgramRadioPlanner> = Arc::new(DomainProgramPlanner::new(
        planner,
        Arc::new(FixtureProgramContext),
    ));
    let playback = Arc::new(LicensedFixturePlayback::default());
    playback.block_at_call.store(3, Ordering::Release);
    let radio_events = Arc::new(RecordingRadioEvents::default());
    let store: Arc<dyn RadioProgramStore> = Arc::new(repository);
    let service = RadioService::new(RadioServiceDependencies {
        planner,
        authorizer: Arc::new(ManualProgramStartAuthorizer),
        store,
        playback: playback.clone(),
        speech: Arc::new(TextOnlyProgramSpeech),
        apple: Arc::new(UnavailableAppleCompanion),
        event_sink: radio_events.clone(),
        clock: Arc::new(FixedClock {
            now_ms: 1_780_000_000_000,
        }),
        sequence: Arc::new(ProcessSequence::default()),
        id_factory: Arc::new(SystemRadioIdFactory),
    });

    let response = service
        .start_local_program(StartProgramRequest {
            client_request_id: Uuid::now_v7(),
            source_id: "local".to_owned(),
            trigger: StartProgramTrigger::Manual,
        })
        .await
        .unwrap_or_else(|error| panic!("explicit fixture start failed: {error:?}"));
    let plan = response
        .plan
        .unwrap_or_else(|| panic!("M3 local start must return its validated plan"));
    let planned_track_ids = plan
        .segments
        .iter()
        .filter_map(|segment| match segment {
            ProgramPlanSegmentsItem::TrackSegment(track) => Some(track.track_id.clone()),
            ProgramPlanSegmentsItem::VoiceSegment(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(planned_track_ids.len(), 6);
    assert_eq!(
        planned_track_ids.iter().cloned().collect::<BTreeSet<_>>(),
        licensed_ids
    );

    playback.wait_for_calls(3).await;
    playback.wait_for_signalled_tracks(6).await;
    service
        .stop_program(StopProgramRequest {
            client_request_id: Uuid::now_v7(),
            program_id: response.program_id,
        })
        .await
        .unwrap_or_else(|error| panic!("explicit fixture stop failed: {error:?}"));
    radio_events
        .wait_for_state(response.program_id, ProgramEventState::Completed)
        .await;
    assert_eq!(
        *playback
            .played_track_ids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        planned_track_ids
    );

    assert_radio_fallback_evidence(&radio_events.snapshot(), &fixture.library_root);
    assert!(playback.stop_calls.load(Ordering::Acquire) >= 1);
    assert!(radio_events.snapshot().iter().any(|event| {
        matches!(event, RadioEvent::ProgramState(state)
            if state.program_id == response.program_id
                && state.state == ProgramEventState::Stopping)
    }));
}

// FR-LIB-001; NFR-COMPAT-002 prototype subset; NFR-PERF-002 event subset.
#[tokio::test]
async fn scanner_fr_lib_001_scans_six_formats_isolates_bad_files_and_never_copies_audio() {
    let fixture = ScannerFixture::create().await;
    let source_before = snapshot_files(&fixture.library_root);
    let terminal = fixture.scan_to_terminal().await;
    assert_eq!(terminal.state, ScanEventState::Completed);
    assert_eq!(terminal.discovered, 8);
    assert_eq!(terminal.scanned, 8);
    assert_eq!(terminal.failed, 2);

    let tracks = load_tracks(&fixture).await;
    assert_eq!(tracks.len(), 8);
    let available: Vec<&TrackFacts> = tracks
        .iter()
        .filter(|track| track.availability == "available")
        .collect();
    assert_eq!(available.len(), 6);
    let formats: BTreeSet<&str> = available
        .iter()
        .map(|track| track.format.as_str())
        .collect();
    assert_eq!(
        formats,
        BTreeSet::from(["aac", "flac", "m4a", "mp3", "ogg", "wav"])
    );
    assert!(
        available
            .iter()
            .all(|track| (11_000..=13_000).contains(&track.duration_ms))
    );

    let by_path = track_map(tracks);
    assert_eq!(
        by_path
            .get(CORRUPT_FIXTURE)
            .map(|track| track.availability.as_str()),
        Some("corrupt")
    );
    assert_eq!(
        by_path
            .get(UNSUPPORTED_FIXTURE)
            .map(|track| track.availability.as_str()),
        Some("unsupported")
    );
    for tagged in ["tone.mp3", "tone.flac", "tone.m4a"] {
        let track = by_path
            .get(tagged)
            .unwrap_or_else(|| panic!("missing tagged fixture {tagged}"));
        assert!(track.title.is_some(), "{tagged} title");
        assert!(track.artist.is_some(), "{tagged} artist");
        assert!(track.album.is_some(), "{tagged} album");
        assert!(track.embedded_cover_hash.is_some(), "{tagged} cover");
    }

    assert_eq!(snapshot_files(&fixture.library_root), source_before);
    let source_audio_hashes: BTreeSet<String> = source_before
        .iter()
        .filter(|(path, _)| path.as_os_str() != UNSUPPORTED_FIXTURE)
        .map(|(_, facts)| facts.sha256.clone())
        .collect();
    let app_hashes = artifact_hashes(&fixture.app_data)
        .into_iter()
        .chain(artifact_hashes(&fixture.app_cache))
        .collect::<BTreeSet<_>>();
    assert!(source_audio_hashes.is_disjoint(&app_hashes));

    let operation_events: Vec<ScanEvent> = fixture
        .events
        .snapshot()
        .into_iter()
        .filter(|event| event.operation_id == terminal.operation_id)
        .collect();
    assert!(
        operation_events
            .iter()
            .any(|event| event.state == ScanEventState::Running),
        "scan must expose path-free progress before terminal"
    );
    for pair in operation_events.windows(2) {
        assert!(pair[1].discovered >= pair[0].discovered);
        assert!(pair[1].scanned >= pair[0].scanned);
        assert!(pair[1].failed >= pair[0].failed);
    }
    for event in operation_events {
        assert_event_contains_no_absolute_path(&event, &fixture.library_root);
    }
    let mut consumed = HashSet::new();
    assert!(consume_terminal_once(&mut consumed, &terminal));
    assert!(!consume_terminal_once(&mut consumed, &terminal));
    fixture
        .storage
        .verify_integrity()
        .await
        .unwrap_or_else(|error| panic!("scanner database integrity: {error}"));
}

// FR-LIB-003; NFR-COMPAT-002 prototype subset.
#[tokio::test]
async fn scanner_fr_lib_003_incremental_scan_preserves_unchanged_modified_and_moved_identity() {
    let fixture = ScannerFixture::create().await;
    assert_eq!(
        fixture.scan_to_terminal().await.state,
        ScanEventState::Completed
    );
    let initial = track_map(load_tracks(&fixture).await);
    assert_eq!(
        fixture.scan_to_terminal().await.state,
        ScanEventState::Completed
    );
    let unchanged = track_map(load_tracks(&fixture).await);
    for name in SUPPORTED_FIXTURES {
        let before = initial
            .get(name)
            .unwrap_or_else(|| panic!("initial track {name}"));
        let after = unchanged
            .get(name)
            .unwrap_or_else(|| panic!("unchanged track {name}"));
        assert_eq!(after.id, before.id, "unchanged track identity for {name}");
        assert_eq!(after.file_size_bytes, before.file_size_bytes);
        assert_eq!(after.modified_at_ms, before.modified_at_ms);
        assert_eq!(after.duration_ms, before.duration_ms);
    }

    let modified_path = fixture.library_root.join("tone.mp3");
    let modified_id = unchanged
        .get("tone.mp3")
        .unwrap_or_else(|| panic!("modified fixture baseline"))
        .id;
    let mut modified_bytes =
        fs::read(&modified_path).unwrap_or_else(|error| panic!("read modified fixture: {error}"));
    modified_bytes.extend_from_slice(b"cyberkindred-generated-junk");
    fs::write(&modified_path, modified_bytes)
        .unwrap_or_else(|error| panic!("modify fixture in place: {error}"));
    assert_eq!(
        fixture.scan_to_terminal().await.state,
        ScanEventState::Completed
    );
    let modified = track_map(load_tracks(&fixture).await);
    let modified_track = modified
        .get("tone.mp3")
        .unwrap_or_else(|| panic!("modified track remains indexed"));
    assert_eq!(modified_track.id, modified_id);
    assert!(
        modified_track.file_size_bytes
            > unchanged
                .get("tone.mp3")
                .unwrap_or_else(|| panic!("modified fixture baseline"))
                .file_size_bytes
    );

    let moved_id = modified
        .get("tone.flac")
        .unwrap_or_else(|| panic!("move fixture baseline"))
        .id;
    let moved_directory = fixture.library_root.join("moved");
    fs::create_dir(&moved_directory)
        .unwrap_or_else(|error| panic!("create move destination: {error}"));
    fs::rename(
        fixture.library_root.join("tone.flac"),
        moved_directory.join("tone.flac"),
    )
    .unwrap_or_else(|error| panic!("move fixture: {error}"));
    assert_eq!(
        fixture.scan_to_terminal().await.state,
        ScanEventState::Completed
    );
    let moved = track_map(load_tracks(&fixture).await);
    let moved_track = moved
        .get("moved/tone.flac")
        .unwrap_or_else(|| panic!("moved track path updated"));
    assert_eq!(
        moved_track.id, moved_id,
        "move must preserve stable track identity"
    );
    assert!(
        !moved.contains_key("tone.flac"),
        "move must not create a second row"
    );

    let deleted_id = moved
        .get("tone.ogg")
        .unwrap_or_else(|| panic!("delete fixture baseline"))
        .id;
    fs::remove_file(fixture.library_root.join("tone.ogg"))
        .unwrap_or_else(|error| panic!("delete fixture: {error}"));
    assert_eq!(
        fixture.scan_to_terminal().await.state,
        ScanEventState::Completed
    );
    let deleted = track_map(load_tracks(&fixture).await);
    let deleted_track = deleted
        .get("tone.ogg")
        .unwrap_or_else(|| panic!("missing track history retained"));
    assert_eq!(deleted_track.id, deleted_id);
    assert_eq!(deleted_track.availability, "missing");
    assert_eq!(
        deleted
            .values()
            .filter(|track| track.availability == "available")
            .count(),
        5
    );
}

// TEST-LIB-002; FR-LIB-002; NFR-PERF-002.
#[tokio::test(flavor = "current_thread")]
#[ignore = "M7 100-track six-format end-to-end scan performance scenario"]
async fn m7_test_lib_002_hundred_tracks_three_scans_cancel_and_incremental_mutations() {
    let fixture = ScannerFixture::create_with_library(copy_m7_hundred_track_fixture).await;
    let full_scans = run_three_hundred_track_scans(&fixture).await;
    let worst_full_scan_ms = full_scans
        .elapsed_ms
        .iter()
        .copied()
        .max()
        .unwrap_or_default();
    assert!(worst_full_scan_ms <= 120_000);

    let (cancel_acceptance_ms, unchanged_rescan) =
        cancel_and_rescan_hundred_tracks(&fixture, &full_scans.baseline).await;
    let mutation_rescans = exercise_m7_scan_mutations(&fixture).await;
    fixture
        .storage
        .verify_integrity()
        .await
        .unwrap_or_else(|error| panic!("M7 scanner database integrity: {error}"));

    println!(
        "{}",
        json!({
            "schemaVersion": 1,
            "testId": "TEST-LIB-002",
            "fixture": "temporary-copies-of-project-licensed-six-format-assets",
            "tracks": 100,
            "formats": ["aac", "flac", "m4a", "mp3", "ogg", "wav"],
            "fullScanMs": full_scans.elapsed_ms,
            "worstFullScanMs": worst_full_scan_ms,
            "fullScanProgressGapMs": full_scans.progress_gap_ms,
            "fullScanRunningEvents": full_scans.running_events,
            "progressCadenceRequired": worst_full_scan_ms > 500,
            "cancelAcceptanceMs": cancel_acceptance_ms,
            "unchangedRescanMs": unchanged_rescan.elapsed.as_millis(),
            "mutationRescanMs": mutation_rescans,
            "networkUsed": false,
            "audioDeviceOpened": false,
            "uiMainThreadMeasured": false,
            "result": "completed"
        })
    );
}

// FR-LIB-002; NFR-PERF-002 prototype cancellation subset.
#[tokio::test(flavor = "current_thread")]
async fn scanner_fr_lib_002_cancel_keeps_committed_rows_consistent_and_rescan_completes() {
    let fixture = ScannerFixture::create().await;
    let bulk = fixture.library_root.join("bulk");
    fs::create_dir(&bulk).unwrap_or_else(|error| panic!("create bulk fixture: {error}"));
    for index in 0..128 {
        fs::copy(
            fixture.library_root.join("tone.wav"),
            bulk.join(format!("tone-{index:03}.wav")),
        )
        .unwrap_or_else(|error| panic!("copy bulk fixture: {error}"));
    }
    let accepted = fixture
        .service
        .start_scan(StartLibraryScanRequest {
            client_request_id: Uuid::now_v7(),
            root_ids: vec![fixture.root_id],
        })
        .await
        .unwrap_or_else(|_| panic!("accept cancellable scan"));
    let cancel_started = Instant::now();
    let cancelled = fixture
        .service
        .cancel_scan(CancelLibraryScanRequest {
            client_request_id: Uuid::now_v7(),
            operation_id: accepted.operation_id,
        })
        .await
        .unwrap_or_else(|_| panic!("cancel scan"));
    assert!(
        cancel_started.elapsed() <= Duration::from_secs(1),
        "prototype cancel acknowledgement must stay within one second"
    );
    assert_eq!(cancelled.state, CancelLibraryScanState::Cancelled);
    let terminal = fixture
        .events
        .wait_for_terminal(accepted.operation_id)
        .await;
    assert_eq!(terminal.state, ScanEventState::Cancelled);
    assert_eq!(
        load_operation_state(&fixture, accepted.operation_id).await,
        "cancelled"
    );
    let tracks = load_tracks(&fixture).await;
    let unique_paths: HashSet<&str> = tracks
        .iter()
        .map(|track| track.relative_path.as_str())
        .collect();
    assert_eq!(unique_paths.len(), tracks.len());
    assert!(tracks.iter().all(|track| {
        matches!(
            track.availability.as_str(),
            "available" | "corrupt" | "unsupported" | "missing"
        )
    }));

    let repeated_cancel = fixture
        .service
        .cancel_scan(CancelLibraryScanRequest {
            client_request_id: Uuid::now_v7(),
            operation_id: accepted.operation_id,
        })
        .await
        .unwrap_or_else(|_| panic!("repeat terminal cancel"));
    assert_eq!(
        repeated_cancel.state,
        CancelLibraryScanState::AlreadyTerminal
    );
    assert_eq!(
        fixture.scan_to_terminal().await.state,
        ScanEventState::Completed
    );
    let completed_tracks = load_tracks(&fixture).await;
    let available: Vec<&TrackFacts> = completed_tracks
        .iter()
        .filter(|track| track.availability == "available")
        .collect();
    assert_eq!(available.len(), 134);
    let duplicate_content_ids: HashSet<Uuid> = available
        .iter()
        .filter(|track| {
            track.relative_path == "tone.wav" || track.relative_path.starts_with("bulk/")
        })
        .map(|track| track.id)
        .collect();
    assert_eq!(
        duplicate_content_ids.len(),
        129,
        "identical audio bytes at distinct files must never be conflated"
    );
}

// FR-LIB-002; NFR-PERF-002 prototype recovery/event subset.
#[tokio::test]
async fn scanner_fr_lib_002_replays_one_authoritative_path_free_terminal_from_outbox() {
    let fixture = ScannerFixture::create().await;
    fixture.events.fail_next_terminal();
    let accepted = fixture
        .service
        .start_scan(StartLibraryScanRequest {
            client_request_id: Uuid::now_v7(),
            root_ids: vec![fixture.root_id],
        })
        .await
        .unwrap_or_else(|_| panic!("accept recovery scan"));
    assert_eq!(
        wait_for_operation_terminal(&fixture, accepted.operation_id).await,
        "completed"
    );
    assert!(
        fixture
            .events
            .snapshot()
            .iter()
            .all(
                |event| !(event.operation_id == accepted.operation_id && event.state.is_terminal())
            )
    );

    fixture
        .service
        .recover_and_replay()
        .await
        .unwrap_or_else(|_| panic!("replay pending scan terminal"));
    let terminal = fixture
        .events
        .wait_for_terminal(accepted.operation_id)
        .await;
    assert_eq!(terminal.state, ScanEventState::Completed);
    assert_event_contains_no_absolute_path(&terminal, &fixture.library_root);
    let terminal_count = fixture
        .events
        .snapshot()
        .iter()
        .filter(|event| event.operation_id == accepted.operation_id && event.state.is_terminal())
        .count();
    fixture
        .service
        .recover_and_replay()
        .await
        .unwrap_or_else(|_| panic!("idempotent terminal recovery"));
    assert_eq!(
        fixture
            .events
            .snapshot()
            .iter()
            .filter(|event| event.operation_id == accepted.operation_id && event.state.is_terminal())
            .count(),
        terminal_count
    );
    let mut consumed = HashSet::new();
    assert!(consume_terminal_once(&mut consumed, &terminal));
    assert!(!consume_terminal_once(&mut consumed, &terminal));
}
