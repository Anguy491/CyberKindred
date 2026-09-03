use super::*;
use crate::storage::{AppPaths, Storage};
use chrono::{DateTime, Utc};
use std::sync::{
    Mutex as StdMutex,
    atomic::{AtomicI64, Ordering},
};

struct FakeClock(AtomicI64);

impl FakeClock {
    fn new(now_ms: i64) -> Self {
        Self(AtomicI64::new(now_ms))
    }

    fn set(&self, now_ms: i64) {
        self.0.store(now_ms, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> DateTime<Utc> {
        Utc.timestamp_millis_opt(self.0.load(Ordering::SeqCst))
            .single()
            .expect("valid fake time")
    }
}

#[derive(Default)]
struct FakeNotifications {
    notices: StdMutex<Vec<ScheduleNotice>>,
}

impl ScheduleNotificationSink for FakeNotifications {
    fn show(&self, notice: &ScheduleNotice) -> Result<Option<String>, ApiError> {
        self.notices.lock().expect("notices").push(notice.clone());
        Ok(Some(Uuid::now_v7().to_string()))
    }
}

#[derive(Default)]
struct FakeEvents {
    events: StdMutex<Vec<ScheduleDueEvent>>,
}

impl ScheduleEventSink for FakeEvents {
    fn publish(&self, event: &ScheduleDueEvent) -> Result<(), ApiError> {
        self.events.lock().expect("events").push(event.clone());
        Ok(())
    }
}

fn utc(value: &str) -> i64 {
    DateTime::parse_from_rfc3339(value)
        .expect("RFC3339")
        .timestamp_millis()
}

fn rule(timezone: &str, day: ScheduleRuleDaysOfWeekItem, local_time: &str) -> ScheduleRule {
    ScheduleRule {
        schema_version: crate::ipc::IPC_SCHEMA_VERSION.to_owned(),
        schedule_id: Uuid::now_v7().to_string(),
        name: "Morning radio".to_owned(),
        timezone: timezone.to_owned(),
        days_of_week: vec![day],
        local_time: local_time.to_owned(),
        enabled: true,
        notification_only: true,
        created_at: "2026-01-01T00:00:00.000Z".to_owned(),
        updated_at: "2026-01-01T00:00:00.000Z".to_owned(),
        revision: 0,
    }
}

async fn fixture(
    now_ms: i64,
) -> (
    tempfile::TempDir,
    Storage,
    Arc<FakeClock>,
    Arc<FakeNotifications>,
    Arc<FakeEvents>,
    SchedulerService,
) {
    let temp = tempfile::tempdir().expect("temp root");
    let paths = AppPaths::create(
        temp.path().join("data"),
        temp.path().join("cache"),
        temp.path().join("logs"),
    )
    .expect("paths");
    let storage = Storage::open(&paths, "0.1.0").await.expect("storage");
    let clock = Arc::new(FakeClock::new(now_ms));
    let notifications = Arc::new(FakeNotifications::default());
    let events = Arc::new(FakeEvents::default());
    let service = SchedulerService::new(
        storage.repository(),
        clock.clone(),
        notifications.clone(),
        events.clone(),
        Arc::new(ProcessSequence::default()),
    )
    .expect("scheduler");
    (temp, storage, clock, notifications, events, service)
}

async fn enable_notifications(storage: &Storage, now_ms: i64) {
    let repository = storage.repository();
    let mut settings = repository.load_provider_settings().await.expect("settings");
    settings.notifications_enabled = true;
    repository
        .save_provider_settings(settings.revision, &settings, now_ms, false, None)
        .await
        .expect("enable notifications");
}

#[test]
fn scheduler_dst_moves_missing_time_forward_and_uses_first_ambiguous_time() {
    let spring = rule("America/New_York", ScheduleRuleDaysOfWeekItem::Sun, "02:30");
    assert_eq!(
        next_occurrence_at_ms(&spring, utc("2026-03-07T12:00:00Z")).expect("spring"),
        utc("2026-03-08T07:00:00Z")
    );

    let fall = rule("America/New_York", ScheduleRuleDaysOfWeekItem::Sun, "01:30");
    assert_eq!(
        next_occurrence_at_ms(&fall, utc("2026-10-31T12:00:00Z")).expect("fall"),
        utc("2026-11-01T05:30:00Z")
    );
}

#[tokio::test]
async fn scheduler_crud_persists_and_keeps_rust_owned_identity() {
    let now_ms = utc("2026-09-06T00:00:00Z");
    let (_temp, storage, _clock, _notifications, _events, service) = fixture(now_ms).await;
    let requested = rule("Australia/Sydney", ScheduleRuleDaysOfWeekItem::Mon, "07:00");
    let requested_id = requested.schedule_id.clone();
    let created = service
        .upsert_schedule(UpsertScheduleRequest {
            client_request_id: Uuid::now_v7(),
            expected_revision: 0,
            schedule: requested,
        })
        .await
        .expect("create");
    assert_ne!(created.schedule.rule.schedule_id, requested_id);
    assert_eq!(created.schedule.rule.revision, 1);
    assert_eq!(created.revision, 1);
    assert!(created.schedule.next_occurrence_at.is_some());

    let persisted = service.list_schedules().await.expect("list");
    assert_eq!(persisted.schedules, vec![created.schedule.clone()]);
    let mut disabled = created.schedule.rule.clone();
    disabled.enabled = false;
    let updated = service
        .upsert_schedule(UpsertScheduleRequest {
            client_request_id: Uuid::now_v7(),
            expected_revision: 1,
            schedule: disabled,
        })
        .await
        .expect("update");
    assert_eq!(updated.schedule.rule.revision, 2);
    assert_eq!(updated.schedule.next_occurrence_at, None);
    let deleted = service
        .delete_schedule(DeleteScheduleRequest {
            client_request_id: Uuid::now_v7(),
            schedule_id: Uuid::parse_str(&updated.schedule.rule.schedule_id).expect("id"),
            expected_revision: 2,
        })
        .await
        .expect("delete");
    assert_eq!(deleted.revision, 3);
    assert!(
        service
            .list_schedules()
            .await
            .expect("empty")
            .schedules
            .is_empty()
    );
    storage.close().await;
}

#[tokio::test]
async fn scheduler_due_is_silent_until_notification_start_grant_is_consumed_once() {
    let now_ms = utc("2026-09-06T00:00:00Z");
    let (_temp, storage, clock, notifications, events, service) = fixture(now_ms).await;
    enable_notifications(&storage, now_ms).await;
    let created = service
        .upsert_schedule(UpsertScheduleRequest {
            client_request_id: Uuid::now_v7(),
            expected_revision: 0,
            schedule: rule("Australia/Sydney", ScheduleRuleDaysOfWeekItem::Mon, "07:00"),
        })
        .await
        .expect("create");
    let due_at = DateTime::parse_from_rfc3339(
        created
            .schedule
            .next_occurrence_at
            .as_deref()
            .expect("next"),
    )
    .expect("next timestamp")
    .timestamp_millis();
    clock.set(due_at);
    service.process_due(due_at).await.expect("due");
    assert_eq!(notifications.notices.lock().expect("notices").len(), 1);
    let event = events.events.lock().expect("events")[0].clone();
    assert!(event.notification_shown);

    let start = service
        .handle_notification_action(NotificationActionRequest {
            client_request_id: Uuid::now_v7(),
            schedule_id: event.schedule_id,
            occurrence_id: event.occurrence_id,
            action: NotificationAction::Start,
            snooze_minutes: None,
        })
        .await
        .expect("start action");
    assert_eq!(start.status, NotificationActionStatus::Starting);
    let start_request = StartProgramRequest {
        client_request_id: Uuid::now_v7(),
        source_id: "local".to_owned(),
        trigger: StartProgramTrigger::Notification,
    };
    assert_eq!(
        service.authorize(&start_request).await.expect("grant"),
        ConfirmedProgramStart::ConfirmedNotification
    );
    assert!(service.authorize(&start_request).await.is_err());
    storage.close().await;
}

#[tokio::test]
async fn scheduler_snooze_atomically_replaces_delay_without_changing_rule() {
    let now_ms = utc("2026-09-06T00:00:00Z");
    let (_temp, storage, clock, notifications, events, service) = fixture(now_ms).await;
    enable_notifications(&storage, now_ms).await;
    let created = service
        .upsert_schedule(UpsertScheduleRequest {
            client_request_id: Uuid::now_v7(),
            expected_revision: 0,
            schedule: rule("Australia/Sydney", ScheduleRuleDaysOfWeekItem::Mon, "07:00"),
        })
        .await
        .expect("create");
    let due_at = DateTime::parse_from_rfc3339(
        created
            .schedule
            .next_occurrence_at
            .as_deref()
            .expect("next"),
    )
    .expect("due")
    .timestamp_millis();
    clock.set(due_at);
    service.process_due(due_at).await.expect("due");
    let event = events.events.lock().expect("events")[0].clone();

    for minutes in [10_u8, 30] {
        clock.set(due_at + i64::from(minutes) * 1_000);
        service
            .handle_notification_action(NotificationActionRequest {
                client_request_id: Uuid::now_v7(),
                schedule_id: event.schedule_id,
                occurrence_id: event.occurrence_id,
                action: NotificationAction::Snooze,
                snooze_minutes: Some(minutes),
            })
            .await
            .expect("snooze");
    }
    clock.set(due_at + 10 * 60 * 1_000);
    service
        .process_due(due_at + 10 * 60 * 1_000)
        .await
        .expect("old snooze");
    assert_eq!(notifications.notices.lock().expect("notices").len(), 1);
    clock.set(due_at + 30 * 60 * 1_000 + 30_000);
    service
        .process_due(due_at + 30 * 60 * 1_000 + 30_000)
        .await
        .expect("replacement snooze");
    assert_eq!(notifications.notices.lock().expect("notices").len(), 2);
    let resnoozed_at = due_at + 30 * 60 * 1_000 + 30_000;
    clock.set(resnoozed_at);
    service
        .handle_notification_action(NotificationActionRequest {
            client_request_id: Uuid::now_v7(),
            schedule_id: event.schedule_id,
            occurrence_id: event.occurrence_id,
            action: NotificationAction::Snooze,
            snooze_minutes: Some(60),
        })
        .await
        .expect("60 minute snooze");
    service
        .process_due(resnoozed_at + 60 * 60 * 1_000 - 1)
        .await
        .expect("before 60 minute snooze");
    assert_eq!(notifications.notices.lock().expect("notices").len(), 2);
    service
        .process_due(resnoozed_at + 60 * 60 * 1_000)
        .await
        .expect("60 minute snooze");
    assert_eq!(notifications.notices.lock().expect("notices").len(), 3);
    let persisted = service.list_schedules().await.expect("schedule");
    assert_eq!(persisted.schedules[0].rule.local_time, "07:00");
    storage.close().await;
}

#[tokio::test]
async fn scheduler_resume_beyond_fifteen_minutes_never_notifies_or_starts() {
    let now_ms = utc("2026-09-06T00:00:00Z");
    let (_temp, storage, clock, notifications, events, service) = fixture(now_ms).await;
    enable_notifications(&storage, now_ms).await;
    let created = service
        .upsert_schedule(UpsertScheduleRequest {
            client_request_id: Uuid::now_v7(),
            expected_revision: 0,
            schedule: rule("Australia/Sydney", ScheduleRuleDaysOfWeekItem::Mon, "07:00"),
        })
        .await
        .expect("create");
    let due_at = DateTime::parse_from_rfc3339(
        created
            .schedule
            .next_occurrence_at
            .as_deref()
            .expect("next"),
    )
    .expect("due")
    .timestamp_millis();
    let resumed_at = due_at + MISSED_NOTIFICATION_WINDOW_MS + 1;
    clock.set(resumed_at);
    service.process_due(resumed_at).await.expect("resume");
    assert!(notifications.notices.lock().expect("notices").is_empty());
    {
        let delivered = events.events.lock().expect("events");
        assert_eq!(delivered.len(), 1);
        assert!(!delivered[0].notification_shown);
    }
    let schedules = service.list_schedules().await.expect("schedule");
    let next = DateTime::parse_from_rfc3339(
        schedules.schedules[0]
            .next_occurrence_at
            .as_deref()
            .expect("future occurrence"),
    )
    .expect("next timestamp")
    .timestamp_millis();
    assert!(next > resumed_at);
    storage.close().await;
}
