//! Persistent weekly schedules and notification-only due handling.

use crate::{
    contracts::{ContractRegistry, ScheduleRule, ScheduleRuleDaysOfWeekItem},
    ipc::{
        ApiError, EventEnvelope, InternalReason, ProcessSequence, PublicField, RequestHash,
        canonical_request_hash,
    },
    providers::Clock,
    radio::{
        ConfirmedProgramStart, ProgramStartAuthorizer, RadioFuture, RadioService,
        StartProgramRequest, StartProgramTrigger,
    },
    storage::{
        Repository, StorageError, StorageReason, StoredDueOccurrence, StoredNotificationAction,
        StoredSchedule,
    },
};
use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Offset, SecondsFormat, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    sync::{Arc, Mutex as StdMutex, OnceLock, Weak},
    time::{Duration, Instant},
};
use tauri::{Emitter, Manager, Runtime};
use tauri_plugin_notification::{NotificationExt, PermissionState};
use tokio::sync::{Mutex, Notify};
use uuid::Uuid;

const IDEMPOTENCY_CAPACITY: usize = 256;
const MISSED_NOTIFICATION_WINDOW_MS: i64 = 15 * 60 * 1_000;
const ACTOR_POLL_INTERVAL: Duration = Duration::from_secs(30);
const START_GRANT_CAPACITY: usize = 128;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScheduleView {
    pub rule: ScheduleRule,
    pub next_occurrence_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListSchedulesResponse {
    pub schedules: Vec<ScheduleView>,
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpsertScheduleRequest {
    pub client_request_id: Uuid,
    pub expected_revision: u64,
    pub schedule: ScheduleRule,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpsertScheduleResponse {
    pub request_id: Uuid,
    pub schedule: ScheduleView,
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteScheduleRequest {
    pub client_request_id: Uuid,
    pub schedule_id: Uuid,
    pub expected_revision: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationAction {
    Open,
    Dismiss,
    Start,
    Snooze,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationActionRequest {
    pub client_request_id: Uuid,
    pub schedule_id: Uuid,
    pub occurrence_id: Uuid,
    pub action: NotificationAction,
    pub snooze_minutes: Option<u8>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationActionStatus {
    AwaitingUser,
    Snoozed,
    Starting,
    Dismissed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationActionResponse {
    pub request_id: Uuid,
    pub occurrence_id: Uuid,
    pub status: NotificationActionStatus,
    pub next_notification_at: Option<String>,
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScheduleDueEvent {
    #[serde(flatten)]
    pub envelope: EventEnvelope,
    pub schedule_id: Uuid,
    pub occurrence_id: Uuid,
    pub notification_shown: bool,
}

#[derive(Clone)]
pub struct ScheduleNotice {
    schedule_id: Uuid,
    occurrence_id: Uuid,
    name: String,
    missed: bool,
}

pub trait ScheduleNotificationSink: Send + Sync {
    fn show(&self, notice: &ScheduleNotice) -> Result<Option<String>, ApiError>;
}

pub trait ScheduleEventSink: Send + Sync {
    fn publish(&self, event: &ScheduleDueEvent) -> Result<(), ApiError>;
}

pub struct TauriScheduleNotificationSink<R: Runtime> {
    app: tauri::AppHandle<R>,
    scheduler: OnceLock<Weak<SchedulerService>>,
}

impl<R: Runtime> TauriScheduleNotificationSink<R> {
    pub fn new(app: tauri::AppHandle<R>) -> Self {
        Self {
            app,
            scheduler: OnceLock::new(),
        }
    }

    pub(crate) fn bind_scheduler(&self, scheduler: Weak<SchedulerService>) -> Result<(), ApiError> {
        self.scheduler
            .set(scheduler)
            .map_err(|_| ApiError::from_reason(InternalReason::UnexpectedInternal))
    }
}

impl<R: Runtime> ScheduleNotificationSink for TauriScheduleNotificationSink<R> {
    fn show(&self, notice: &ScheduleNotice) -> Result<Option<String>, ApiError> {
        if self
            .app
            .notification()
            .permission_state()
            .map_err(|_| ApiError::from_reason(InternalReason::UnexpectedInternal))?
            != PermissionState::Granted
        {
            return Ok(None);
        }
        let notification_id = Uuid::now_v7().to_string();
        let prefix = if notice.missed {
            "错过的节目"
        } else {
            "节目时间到了"
        };
        #[cfg(windows)]
        {
            let scheduler = self
                .scheduler
                .get()
                .cloned()
                .ok_or_else(|| ApiError::from_reason(InternalReason::UnexpectedInternal))?;
            let mut notification = notify_rust::Notification::new();
            notification
                .summary(&format!("{prefix} · {}", notice.name))
                .body("选择开始、稍后提醒或忽略；点击通知正文只会打开 CyberKindred。")
                .action("start", "开始节目")
                .action("snooze_10", "10 分钟后")
                .action("snooze_30", "30 分钟后")
                .action("snooze_60", "60 分钟后")
                .action("dismiss", "忽略");
            if !tauri::is_dev() {
                notification.app_id(&self.app.config().identifier);
            }
            let handle = notification
                .show()
                .map_err(|_| ApiError::from_reason(InternalReason::UnexpectedInternal))?;
            let app = self.app.clone();
            let schedule_id = notice.schedule_id;
            let occurrence_id = notice.occurrence_id;
            tauri::async_runtime::spawn_blocking(move || {
                let _ = handle.wait_for_response(
                    move |response: &notify_rust::NotificationResponse| {
                        let action = match response {
                            notify_rust::NotificationResponse::Default => {
                                if let Some(window) = app.get_webview_window("main") {
                                    let _ = window.show();
                                    let _ = window.set_focus();
                                }
                                Some((NotificationAction::Open, None))
                            }
                            notify_rust::NotificationResponse::Action(identifier) => {
                                native_action(identifier)
                            }
                            notify_rust::NotificationResponse::Reply(_)
                            | notify_rust::NotificationResponse::Closed(_) => None,
                        };
                        if let Some((action, snooze_minutes)) = action
                            && let Some(service) = scheduler.upgrade()
                        {
                            tauri::async_runtime::spawn(async move {
                                let _ = service
                                    .handle_native_notification_action(
                                        schedule_id,
                                        occurrence_id,
                                        action,
                                        snooze_minutes,
                                    )
                                    .await;
                            });
                        }
                    },
                );
            });
            Ok(Some(notification_id))
        }

        #[cfg(not(windows))]
        {
            // The desktop adapter receives no sound name. Its Windows backend emits
            // an explicit silent audio element; `.silent()` is kept for platforms
            // that also honor the public flag.
            self.app
                .notification()
                .builder()
                .title(format!("{prefix} · {}", notice.name))
                .body("打开 CyberKindred 后选择：开始节目、稍后提醒或忽略。")
                .extra("scheduleId", notice.schedule_id)
                .extra("occurrenceId", notice.occurrence_id)
                .silent()
                .show()
                .map_err(|_| ApiError::from_reason(InternalReason::UnexpectedInternal))?;
            Ok(Some(notification_id))
        }
    }
}

#[cfg(windows)]
fn native_action(identifier: &str) -> Option<(NotificationAction, Option<u8>)> {
    match identifier {
        "start" => Some((NotificationAction::Start, None)),
        "snooze_10" => Some((NotificationAction::Snooze, Some(10))),
        "snooze_30" => Some((NotificationAction::Snooze, Some(30))),
        "snooze_60" => Some((NotificationAction::Snooze, Some(60))),
        "dismiss" => Some((NotificationAction::Dismiss, None)),
        _ => None,
    }
}

pub struct TauriScheduleEventSink<R: Runtime> {
    app: tauri::AppHandle<R>,
}

impl<R: Runtime> TauriScheduleEventSink<R> {
    pub fn new(app: tauri::AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: Runtime> ScheduleEventSink for TauriScheduleEventSink<R> {
    fn publish(&self, event: &ScheduleDueEvent) -> Result<(), ApiError> {
        self.app
            .emit("cyberkindred://v1/schedule/due", event)
            .map_err(|_| ApiError::from_reason(InternalReason::UnexpectedInternal))
    }
}

struct Entry<T> {
    request_hash: RequestHash,
    expires_at: StdMutex<Option<Instant>>,
    result: Mutex<Option<Result<T, ApiError>>>,
}

impl<T> Entry<T> {
    fn retain_at(&self, now: Instant) -> bool {
        self.expires_at
            .lock()
            .map_or(true, |expires| expires.is_none_or(|value| value > now))
    }

    fn mark_completed(&self) {
        if let Ok(mut expires) = self.expires_at.lock() {
            *expires = Some(Instant::now() + Duration::from_mins(10));
        }
    }
}

struct AsyncIdempotency<T> {
    entries: Mutex<HashMap<Uuid, Arc<Entry<T>>>>,
}

impl<T: Clone> AsyncIdempotency<T> {
    fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::with_capacity(IDEMPOTENCY_CAPACITY)),
        }
    }

    async fn execute<F, Fut>(
        &self,
        request_id: Uuid,
        request_hash: RequestHash,
        operation: F,
    ) -> Result<T, ApiError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, ApiError>>,
    {
        let now = Instant::now();
        let entry = {
            let mut entries = self.entries.lock().await;
            entries.retain(|_, entry| entry.retain_at(now));
            if let Some(entry) = entries.get(&request_id) {
                if entry.request_hash != request_hash {
                    return Err(
                        ApiError::from_reason(InternalReason::IdempotencyPayloadConflict)
                            .with_field(PublicField::ClientRequestId),
                    );
                }
                Arc::clone(entry)
            } else {
                if entries.len() >= IDEMPOTENCY_CAPACITY {
                    return Err(ApiError::from_reason(InternalReason::ResourceBusy));
                }
                let entry = Arc::new(Entry {
                    request_hash,
                    expires_at: StdMutex::new(None),
                    result: Mutex::new(None),
                });
                entries.insert(request_id, Arc::clone(&entry));
                entry
            }
        };
        let mut cached = entry.result.lock().await;
        if let Some(result) = cached.as_ref() {
            return result.clone();
        }
        let result = operation().await;
        *cached = Some(result.clone());
        entry.mark_completed();
        result
    }
}

#[derive(Clone, Copy)]
struct StartGrant {
    occurrence_id: Uuid,
}

pub struct SchedulerService {
    repository: Repository,
    clock: Arc<dyn Clock>,
    notifications: Arc<dyn ScheduleNotificationSink>,
    events: Arc<dyn ScheduleEventSink>,
    sequence: Arc<ProcessSequence>,
    contracts: ContractRegistry,
    signal: Notify,
    start_grants: Mutex<VecDeque<StartGrant>>,
    program_starter: OnceLock<Arc<dyn NotificationProgramStarter>>,
    upsert_requests: AsyncIdempotency<UpsertScheduleResponse>,
    delete_requests: AsyncIdempotency<crate::providers::Ack>,
    action_requests: AsyncIdempotency<NotificationActionResponse>,
}

impl SchedulerService {
    pub fn new(
        repository: Repository,
        clock: Arc<dyn Clock>,
        notifications: Arc<dyn ScheduleNotificationSink>,
        events: Arc<dyn ScheduleEventSink>,
        sequence: Arc<ProcessSequence>,
    ) -> Result<Self, ApiError> {
        Ok(Self {
            repository,
            clock,
            notifications,
            events,
            sequence,
            contracts: ContractRegistry::new()
                .map_err(|_| ApiError::from_reason(InternalReason::UnexpectedInternal))?,
            signal: Notify::new(),
            start_grants: Mutex::new(VecDeque::with_capacity(START_GRANT_CAPACITY)),
            program_starter: OnceLock::new(),
            upsert_requests: AsyncIdempotency::new(),
            delete_requests: AsyncIdempotency::new(),
            action_requests: AsyncIdempotency::new(),
        })
    }

    pub(crate) fn bind_program_starter(
        &self,
        starter: Arc<dyn NotificationProgramStarter>,
    ) -> Result<(), ApiError> {
        self.program_starter
            .set(starter)
            .map_err(|_| ApiError::from_reason(InternalReason::UnexpectedInternal))
    }

    pub async fn list_schedules(&self) -> Result<ListSchedulesResponse, ApiError> {
        let collection = self
            .repository
            .load_schedules()
            .await
            .map_err(|error| map_storage_error(&error))?;
        let schedules = collection
            .schedules
            .into_iter()
            .map(stored_schedule_view)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListSchedulesResponse {
            schedules,
            revision: collection.revision,
        })
    }

    pub async fn upsert_schedule(
        &self,
        request: UpsertScheduleRequest,
    ) -> Result<UpsertScheduleResponse, ApiError> {
        let hash = canonical_request_hash(&request)?;
        self.upsert_requests
            .execute(request.client_request_id, hash, || {
                self.upsert_once(request)
            })
            .await
    }

    async fn upsert_once(
        &self,
        request: UpsertScheduleRequest,
    ) -> Result<UpsertScheduleResponse, ApiError> {
        validate_schedule_rule(&self.contracts, &request.schedule)?;
        let next = request
            .schedule
            .enabled
            .then(|| next_occurrence_at_ms(&request.schedule, self.clock.now_ms()))
            .transpose()?;
        let (stored, revision) = self
            .repository
            .upsert_schedule(
                request.expected_revision,
                &request.schedule,
                next,
                self.clock.now_ms(),
            )
            .await
            .map_err(|error| map_storage_error(&error))?;
        self.signal.notify_one();
        Ok(UpsertScheduleResponse {
            request_id: request.client_request_id,
            schedule: stored_schedule_view(stored)?,
            revision,
        })
    }

    pub async fn delete_schedule(
        &self,
        request: DeleteScheduleRequest,
    ) -> Result<crate::providers::Ack, ApiError> {
        let hash = canonical_request_hash(&request)?;
        self.delete_requests
            .execute(request.client_request_id, hash, || {
                self.delete_once(request)
            })
            .await
    }

    async fn delete_once(
        &self,
        request: DeleteScheduleRequest,
    ) -> Result<crate::providers::Ack, ApiError> {
        let revision = self
            .repository
            .delete_schedule(
                request.schedule_id,
                request.expected_revision,
                self.clock.now_ms(),
            )
            .await
            .map_err(|error| map_storage_error(&error))?;
        self.signal.notify_one();
        Ok(crate::providers::Ack {
            request_id: request.client_request_id,
            revision,
        })
    }

    pub async fn handle_notification_action(
        &self,
        request: NotificationActionRequest,
    ) -> Result<NotificationActionResponse, ApiError> {
        validate_action(&request)?;
        let hash = canonical_request_hash(&request)?;
        self.action_requests
            .execute(request.client_request_id, hash, || {
                self.action_once(request)
            })
            .await
    }

    async fn action_once(
        &self,
        request: NotificationActionRequest,
    ) -> Result<NotificationActionResponse, ApiError> {
        let action = match request.action {
            NotificationAction::Open => StoredNotificationAction::Open,
            NotificationAction::Dismiss => StoredNotificationAction::Dismiss,
            NotificationAction::Start => StoredNotificationAction::Start,
            NotificationAction::Snooze => {
                StoredNotificationAction::Snooze(request.snooze_minutes.unwrap_or_default())
            }
        };
        // Reserve notification-start capacity before changing the persisted
        // occurrence to `starting`, so a saturated in-memory grant queue can
        // never leave an unconsumable durable state behind.
        let mut start_grants = if matches!(request.action, NotificationAction::Start) {
            let grants = self.start_grants.lock().await;
            if grants.len() >= START_GRANT_CAPACITY {
                return Err(ApiError::from_reason(InternalReason::ResourceBusy));
            }
            Some(grants)
        } else {
            None
        };
        let result = self
            .repository
            .handle_notification_action(
                request.schedule_id,
                request.occurrence_id,
                action,
                self.clock.now_ms(),
            )
            .await
            .map_err(|error| map_storage_error(&error))?;
        if result.newly_starting {
            let grants = start_grants
                .as_mut()
                .ok_or_else(|| ApiError::from_reason(InternalReason::UnexpectedInternal))?;
            grants.push_back(StartGrant {
                occurrence_id: request.occurrence_id,
            });
        }
        if matches!(request.action, NotificationAction::Snooze) {
            self.signal.notify_one();
        }
        Ok(NotificationActionResponse {
            request_id: request.client_request_id,
            occurrence_id: request.occurrence_id,
            status: parse_action_status(&result.status)?,
            next_notification_at: result.next_notification_at_ms.map(timestamp).transpose()?,
            revision: result.revision,
        })
    }

    async fn handle_native_notification_action(
        &self,
        schedule_id: Uuid,
        occurrence_id: Uuid,
        action: NotificationAction,
        snooze_minutes: Option<u8>,
    ) -> Result<(), ApiError> {
        let response = self
            .handle_notification_action(NotificationActionRequest {
                client_request_id: Uuid::now_v7(),
                schedule_id,
                occurrence_id,
                action,
                snooze_minutes,
            })
            .await?;
        if response.status == NotificationActionStatus::Starting {
            let starter = self
                .program_starter
                .get()
                .ok_or_else(|| ApiError::from_reason(InternalReason::UnexpectedInternal))?;
            starter
                .start(StartProgramRequest {
                    client_request_id: Uuid::now_v7(),
                    source_id: "local".to_owned(),
                    trigger: StartProgramTrigger::Notification,
                })
                .await?;
        }
        Ok(())
    }

    pub fn start(self: Arc<Self>) -> tauri::async_runtime::JoinHandle<()> {
        tauri::async_runtime::spawn(async move {
            loop {
                let _ = self.process_due(self.clock.now_ms()).await;
                tokio::select! {
                    () = self.signal.notified() => {}
                    () = tokio::time::sleep(ACTOR_POLL_INTERVAL) => {}
                }
            }
        })
    }

    /// Reconciles elapsed occurrences after a power boundary. This may show a
    /// notification but cannot start a program without a fresh notification
    /// action grant.
    pub(crate) async fn reconcile_after_resume(&self) -> Result<(), ApiError> {
        self.process_due(self.clock.now_ms()).await
    }

    async fn process_due(&self, now_ms: i64) -> Result<(), ApiError> {
        let collection = self
            .repository
            .load_schedules()
            .await
            .map_err(|error| map_storage_error(&error))?;
        for stored in collection.schedules {
            let Some(due_at_ms) = stored.next_occurrence_at_ms else {
                continue;
            };
            if !stored.rule.enabled || due_at_ms > now_ms {
                continue;
            }
            // A long sleep may span many weekly occurrences. Reconcile it in one
            // bounded step instead of emitting one historical miss per poll.
            let next_after_ms = if now_ms.saturating_sub(due_at_ms) > MISSED_NOTIFICATION_WINDOW_MS
            {
                now_ms
            } else {
                due_at_ms
            };
            let next = next_occurrence_at_ms(&stored.rule, next_after_ms)?;
            let occurrence_key = occurrence_key(&stored.rule, due_at_ms)?;
            if let Some(occurrence) = self
                .repository
                .claim_due_schedule(
                    Uuid::parse_str(&stored.rule.schedule_id)
                        .map_err(|_| ApiError::from_reason(InternalReason::UnexpectedInternal))?,
                    due_at_ms,
                    &occurrence_key,
                    Some(next),
                    now_ms,
                )
                .await
                .map_err(|error| map_storage_error(&error))?
            {
                self.deliver_occurrence(occurrence, now_ms).await?;
            }
        }
        for occurrence in self
            .repository
            .load_due_snoozes(now_ms)
            .await
            .map_err(|error| map_storage_error(&error))?
        {
            self.deliver_occurrence(occurrence, now_ms).await?;
        }
        Ok(())
    }

    async fn deliver_occurrence(
        &self,
        occurrence: StoredDueOccurrence,
        now_ms: i64,
    ) -> Result<(), ApiError> {
        let settings = self
            .repository
            .load_provider_settings()
            .await
            .map_err(|error| map_storage_error(&error))?;
        let age_ms = now_ms.saturating_sub(occurrence.due_at_ms);
        let missed = age_ms > 0;
        let notification_id =
            if settings.notifications_enabled && age_ms <= MISSED_NOTIFICATION_WINDOW_MS {
                self.notifications.show(&ScheduleNotice {
                    schedule_id: occurrence.schedule_id,
                    occurrence_id: occurrence.occurrence_id,
                    name: occurrence.name,
                    missed,
                })?
            } else {
                None
            };
        let shown = notification_id.is_some();
        if self
            .repository
            .mark_occurrence_notification(
                occurrence.occurrence_id,
                shown,
                notification_id.as_deref(),
                now_ms,
            )
            .await
            .map_err(|error| map_storage_error(&error))?
        {
            let sequence = self.sequence.next()?;
            self.events.publish(&ScheduleDueEvent {
                envelope: EventEnvelope {
                    schema_version: crate::ipc::IPC_SCHEMA_VERSION.to_owned(),
                    sequence,
                    occurred_at: timestamp(now_ms)?,
                },
                schedule_id: occurrence.schedule_id,
                occurrence_id: occurrence.occurrence_id,
                notification_shown: shown,
            })?;
        }
        Ok(())
    }
}

impl ProgramStartAuthorizer for SchedulerService {
    fn authorize<'a>(
        &'a self,
        request: &'a StartProgramRequest,
    ) -> RadioFuture<'a, Result<ConfirmedProgramStart, ApiError>> {
        Box::pin(async move {
            match request.trigger {
                StartProgramTrigger::Manual => Ok(ConfirmedProgramStart::Manual),
                StartProgramTrigger::Notification => {
                    let grant = self
                        .start_grants
                        .lock()
                        .await
                        .pop_front()
                        .ok_or_else(|| ApiError::from_reason(InternalReason::RequestInvalid))?;
                    let consumed = self
                        .repository
                        .consume_notification_start(grant.occurrence_id, self.clock.now_ms())
                        .await
                        .map_err(|error| map_storage_error(&error))?;
                    if !consumed {
                        return Err(ApiError::from_reason(InternalReason::RequestInvalid));
                    }
                    Ok(ConfirmedProgramStart::ConfirmedNotification)
                }
            }
        })
    }
}

pub(crate) struct SchedulerStartAuthorizer {
    service: Weak<SchedulerService>,
}

impl SchedulerStartAuthorizer {
    pub(crate) fn new(service: Weak<SchedulerService>) -> Self {
        Self { service }
    }
}

impl ProgramStartAuthorizer for SchedulerStartAuthorizer {
    fn authorize<'a>(
        &'a self,
        request: &'a StartProgramRequest,
    ) -> RadioFuture<'a, Result<ConfirmedProgramStart, ApiError>> {
        Box::pin(async move {
            self.service
                .upgrade()
                .ok_or_else(|| ApiError::from_reason(InternalReason::UnexpectedInternal))?
                .authorize(request)
                .await
        })
    }
}

pub(crate) trait NotificationProgramStarter: Send + Sync {
    fn start(&self, request: StartProgramRequest) -> RadioFuture<'_, Result<(), ApiError>>;
}

impl NotificationProgramStarter for RadioService {
    fn start(&self, request: StartProgramRequest) -> RadioFuture<'_, Result<(), ApiError>> {
        Box::pin(async move { self.start_local_program(request).await.map(|_| ()) })
    }
}

pub(crate) struct SchedulerRuntime(tauri::async_runtime::JoinHandle<()>);

impl SchedulerRuntime {
    pub(crate) fn new(handle: tauri::async_runtime::JoinHandle<()>) -> Self {
        Self(handle)
    }

    pub(crate) fn abort(&self) {
        self.0.abort();
    }

    pub(crate) fn is_finished(&self) -> bool {
        match &self.0 {
            tauri::async_runtime::JoinHandle::Tokio(handle) => handle.is_finished(),
        }
    }
}

impl Drop for SchedulerRuntime {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub mod commands {
    use super::{
        DeleteScheduleRequest, ListSchedulesResponse, NotificationActionRequest,
        NotificationActionResponse, SchedulerService, UpsertScheduleRequest,
        UpsertScheduleResponse,
    };
    use crate::{
        ipc::{ApiError, EmptyRequest, parse_command_request},
        providers::Ack,
    };
    use std::sync::Arc;
    use tauri::State;

    #[tauri::command]
    #[allow(clippy::needless_pass_by_value)]
    pub async fn api_v1_list_schedules(
        request: tauri::ipc::Request<'_>,
        service: State<'_, Arc<SchedulerService>>,
    ) -> Result<ListSchedulesResponse, ApiError> {
        let EmptyRequest {} = parse_command_request::<EmptyRequest>(&request)?;
        service.list_schedules().await
    }

    #[tauri::command]
    #[allow(clippy::needless_pass_by_value)]
    pub async fn api_v1_upsert_schedule(
        request: tauri::ipc::Request<'_>,
        service: State<'_, Arc<SchedulerService>>,
    ) -> Result<UpsertScheduleResponse, ApiError> {
        service
            .upsert_schedule(parse_command_request::<UpsertScheduleRequest>(&request)?)
            .await
    }

    #[tauri::command]
    #[allow(clippy::needless_pass_by_value)]
    pub async fn api_v1_delete_schedule(
        request: tauri::ipc::Request<'_>,
        service: State<'_, Arc<SchedulerService>>,
    ) -> Result<Ack, ApiError> {
        service
            .delete_schedule(parse_command_request::<DeleteScheduleRequest>(&request)?)
            .await
    }

    #[tauri::command]
    #[allow(clippy::needless_pass_by_value)]
    pub async fn api_v1_handle_notification_action(
        request: tauri::ipc::Request<'_>,
        service: State<'_, Arc<SchedulerService>>,
    ) -> Result<NotificationActionResponse, ApiError> {
        service
            .handle_notification_action(parse_command_request::<NotificationActionRequest>(
                &request,
            )?)
            .await
    }
}

fn validate_schedule_rule(
    contracts: &ContractRegistry,
    schedule: &ScheduleRule,
) -> Result<(), ApiError> {
    let value = serde_json::to_value(schedule)
        .map_err(|_| ApiError::from_reason(InternalReason::RequestInvalid))?;
    contracts
        .validate("schedule-rule", &value)
        .map_err(|_| ApiError::from_reason(InternalReason::RequestInvalid))?;
    schedule
        .timezone
        .parse::<chrono_tz::Tz>()
        .map_err(|_| ApiError::from_reason(InternalReason::InvalidCandidate))?;
    if chrono::DateTime::parse_from_rfc3339(&schedule.created_at).is_err()
        || chrono::DateTime::parse_from_rfc3339(&schedule.updated_at).is_err()
    {
        return Err(ApiError::from_reason(InternalReason::RequestInvalid));
    }
    Ok(())
}

fn validate_action(request: &NotificationActionRequest) -> Result<(), ApiError> {
    let valid = match request.action {
        NotificationAction::Snooze => matches!(request.snooze_minutes, Some(10 | 30 | 60)),
        NotificationAction::Open | NotificationAction::Dismiss | NotificationAction::Start => {
            request.snooze_minutes.is_none()
        }
    };
    if !valid {
        return Err(ApiError::from_reason(InternalReason::RequestInvalid));
    }
    Ok(())
}

fn stored_schedule_view(stored: StoredSchedule) -> Result<ScheduleView, ApiError> {
    Ok(ScheduleView {
        rule: stored.rule,
        next_occurrence_at: stored.next_occurrence_at_ms.map(timestamp).transpose()?,
    })
}

fn next_occurrence_at_ms(rule: &ScheduleRule, after_ms: i64) -> Result<i64, ApiError> {
    let timezone = rule
        .timezone
        .parse::<chrono_tz::Tz>()
        .map_err(|_| ApiError::from_reason(InternalReason::InvalidCandidate))?;
    let after = Utc
        .timestamp_millis_opt(after_ms)
        .single()
        .ok_or_else(|| ApiError::from_reason(InternalReason::RequestInvalid))?;
    let local_date = after.with_timezone(&timezone).date_naive();
    let time = NaiveTime::parse_from_str(&rule.local_time, "%H:%M")
        .map_err(|_| ApiError::from_reason(InternalReason::RequestInvalid))?;
    for day_offset in 0..=8 {
        let date = local_date
            .checked_add_days(chrono::Days::new(day_offset))
            .ok_or_else(|| ApiError::from_reason(InternalReason::UnexpectedInternal))?;
        if !matches_weekday(rule, date) {
            continue;
        }
        let intended = date.and_time(time);
        let candidate = resolve_local_datetime(timezone, intended)?;
        let candidate_ms = candidate.with_timezone(&Utc).timestamp_millis();
        if candidate_ms > after_ms {
            return Ok(candidate_ms);
        }
    }
    Err(ApiError::from_reason(InternalReason::UnexpectedInternal))
}

fn resolve_local_datetime(
    timezone: chrono_tz::Tz,
    intended: NaiveDateTime,
) -> Result<chrono::DateTime<chrono_tz::Tz>, ApiError> {
    match timezone.from_local_datetime(&intended) {
        chrono::LocalResult::Single(value) => Ok(value),
        chrono::LocalResult::Ambiguous(first, second) => {
            Ok(if first.with_timezone(&Utc) <= second.with_timezone(&Utc) {
                first
            } else {
                second
            })
        }
        chrono::LocalResult::None => {
            for minutes in 1..=180 {
                let shifted = intended
                    .checked_add_signed(chrono::Duration::minutes(minutes))
                    .ok_or_else(|| ApiError::from_reason(InternalReason::UnexpectedInternal))?;
                if shifted.date() != intended.date() {
                    break;
                }
                if let Some(value) = timezone.from_local_datetime(&shifted).earliest() {
                    return Ok(value);
                }
            }
            Err(ApiError::from_reason(InternalReason::InvalidCandidate))
        }
    }
}

fn matches_weekday(rule: &ScheduleRule, date: NaiveDate) -> bool {
    let expected = match date.weekday() {
        chrono::Weekday::Mon => ScheduleRuleDaysOfWeekItem::Mon,
        chrono::Weekday::Tue => ScheduleRuleDaysOfWeekItem::Tue,
        chrono::Weekday::Wed => ScheduleRuleDaysOfWeekItem::Wed,
        chrono::Weekday::Thu => ScheduleRuleDaysOfWeekItem::Thu,
        chrono::Weekday::Fri => ScheduleRuleDaysOfWeekItem::Fri,
        chrono::Weekday::Sat => ScheduleRuleDaysOfWeekItem::Sat,
        chrono::Weekday::Sun => ScheduleRuleDaysOfWeekItem::Sun,
    };
    rule.days_of_week.contains(&expected)
}

fn occurrence_key(rule: &ScheduleRule, due_at_ms: i64) -> Result<String, ApiError> {
    let timezone = rule
        .timezone
        .parse::<chrono_tz::Tz>()
        .map_err(|_| ApiError::from_reason(InternalReason::UnexpectedInternal))?;
    let due = Utc
        .timestamp_millis_opt(due_at_ms)
        .single()
        .ok_or_else(|| ApiError::from_reason(InternalReason::UnexpectedInternal))?
        .with_timezone(&timezone);
    Ok(format!(
        "{}:{}:{}:{}",
        rule.schedule_id,
        due.date_naive(),
        rule.local_time,
        due.offset().fix().local_minus_utc()
    ))
}

fn parse_action_status(value: &str) -> Result<NotificationActionStatus, ApiError> {
    match value {
        "awaiting_user" => Ok(NotificationActionStatus::AwaitingUser),
        "snoozed" => Ok(NotificationActionStatus::Snoozed),
        "starting" => Ok(NotificationActionStatus::Starting),
        "dismissed" => Ok(NotificationActionStatus::Dismissed),
        _ => Err(ApiError::from_reason(InternalReason::UnexpectedInternal)),
    }
}

fn timestamp(timestamp_ms: i64) -> Result<String, ApiError> {
    Utc.timestamp_millis_opt(timestamp_ms)
        .single()
        .map(|value| value.to_rfc3339_opts(SecondsFormat::Millis, true))
        .ok_or_else(|| ApiError::from_reason(InternalReason::UnexpectedInternal))
}

fn map_storage_error(error: &StorageError) -> ApiError {
    let reason = match error.reason() {
        StorageReason::PathDenied => InternalReason::PathDenied,
        StorageReason::PathOutsideScope => InternalReason::PathOutsideRoot,
        StorageReason::UnsafeReparsePoint => InternalReason::UnsafeReparsePoint,
        StorageReason::StorageReadFailed => InternalReason::StorageReadFailed,
        StorageReason::StorageWriteFailed | StorageReason::InvalidSetting => {
            InternalReason::StorageWriteFailed
        }
        StorageReason::StorageIntegrityFailed | StorageReason::ForeignDatabase => {
            InternalReason::StorageIntegrityFailed
        }
        StorageReason::MigrationFailed => InternalReason::MigrationFailed,
        StorageReason::DatabaseVersionUnsupported => InternalReason::DatabaseVersionUnsupported,
        StorageReason::EntityNotFound => InternalReason::EntityNotFound,
        StorageReason::RevisionConflict => InternalReason::RevisionConflict,
        StorageReason::ResourceBusy => InternalReason::ResourceBusy,
    };
    ApiError::from_reason(reason)
}

#[cfg(test)]
mod tests;
