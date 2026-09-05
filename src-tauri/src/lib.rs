#![forbid(unsafe_code)]

pub mod contracts;
pub mod data_control;
pub mod diagnostics;
pub mod ipc;
pub mod library;
pub mod llm;
mod llm_repository;
pub mod metadata;
mod onboarding;
mod os_integration;
pub mod playback;
mod playback_repository;
pub mod program;
pub mod providers;
pub mod radio;
mod radio_repository;
mod radio_speech_repository;
pub mod scanner;
mod schedule;
pub mod speech;
pub mod storage;
mod understanding;
mod weather;

use chrono::Utc;
use data_control::{
    DataControlService, DataControlServiceDependencies, TauriDataExportEventSink,
    TauriDataExportPicker, api_v1_delete_all_user_data, api_v1_delete_data_category,
    api_v1_export_user_data, api_v1_get_data_inventory, api_v1_preview_data_deletion,
};
use diagnostics::{DiagnosticEvent, DiagnosticLog, ValidatedLogDirectory};
use ipc::{
    ApiError, AppCapabilities, CapabilitiesService, EmptyRequest, ProcessSequence,
    SourceCapabilities, SourceKind, SourceSummary, parse_command_request,
};
use library::{
    LibraryRootService, SystemLibraryRootClock, TauriLibraryRootPicker, TrackCatalogService,
    commands::{
        api_v1_list_library_roots, api_v1_list_tracks, api_v1_pick_and_add_library_root,
        api_v1_remove_library_root,
    },
};
use llm::{OpenAiChatProvider, OpenAiProgramPlanProvider, SystemProgramCallContextFactory};
use llm_repository::RepositoryProgramCredentialSource;
use onboarding::{
    OnboardingService,
    commands::{api_v1_get_onboarding_state, api_v1_save_onboarding_step},
};
use os_integration::AppIntegrationService;
use playback::{
    PlaybackEventSink, PlaybackService, RodioAudioEngine, SystemPlaybackClock,
    TauriPlaybackEventSink,
    commands::{
        api_v1_get_playback_state, api_v1_list_music_sources, api_v1_next, api_v1_pause,
        api_v1_play, api_v1_previous, api_v1_seek, api_v1_select_music_source,
    },
};
use playback_repository::RepositoryTrackResolver;
use program::{
    ProgramPlanProvider, ProgramPlanner, ProgramRepository, SystemProgramClock,
    SystemProgramIdFactory,
};
use providers::{
    AppBehaviorSettings, AppSettingsEffect, CandidateSecretValidator, ProviderHealthProbe,
    ProviderRuntime, ProviderService, SystemClock, VoicePreviewEventSink, VoicePreviewer,
    commands::{
        api_v1_cancel_operation, api_v1_delete_secret, api_v1_get_settings, api_v1_list_voices,
        api_v1_preview_voice, api_v1_test_provider, api_v1_update_settings,
        api_v1_validate_and_set_secret,
    },
    events::{StartupVoicePreviewOutboxRecovery, TauriVoicePreviewEventSink},
};
use radio::{
    AppleCompanion, AppleCompanionMonitor, DomainProgramPlanner, LocalProgramPlayback,
    PlaybackEventHub, ProgramPlannerContextSource, ProgramRadioPlanner, ProgramSpeech,
    RadioProgramStore, RadioService, RadioServiceDependencies, SystemProgramSpeech,
    SystemRadioClock, SystemRadioIdFactory, TauriRadioEventSink,
    commands::{api_v1_start_program, api_v1_stop_program},
};
use radio_repository::RepositoryProgramContextSource;
use radio_speech_repository::{RepositoryProgramSpeech, RepositorySystemProgramSpeech};
use scanner::{
    ScannerService, SystemScanClock, TauriScanEventSink,
    commands::{api_v1_cancel_library_scan, api_v1_start_library_scan},
};
use schedule::{
    NotificationProgramStarter, SchedulerRuntime, SchedulerService, SchedulerStartAuthorizer,
    TauriScheduleEventSink, TauriScheduleNotificationSink,
    commands::{
        api_v1_delete_schedule, api_v1_handle_notification_action, api_v1_list_schedules,
        api_v1_upsert_schedule,
    },
};
use speech::SpeechVoicePreviewer;
use std::{
    error::Error,
    io,
    sync::{Arc, Mutex},
};
use storage::{AppPaths, RetentionResult, Storage, WindowsCredentialVault};
use tauri::Manager;
use understanding::{
    TauriChatEventSink, UnderstandingService,
    commands::{
        api_v1_approve_memory, api_v1_delete_memory, api_v1_delete_session_summary,
        api_v1_get_profile_view, api_v1_list_memories, api_v1_list_session_summaries,
        api_v1_reject_memory_proposal, api_v1_submit_chat, api_v1_submit_feedback,
        api_v1_update_memory, api_v1_update_profile,
    },
};
use uuid::Uuid;
use weather::{
    CompositeProviderHealthProbe, OpenMeteoWeatherProvider, WeatherProvider, WeatherService,
    commands::{api_v1_search_weather_locations, api_v1_select_weather_location},
};

struct RetentionMaintenance(tauri::async_runtime::JoinHandle<()>);

impl Drop for RetentionMaintenance {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn run_daily_retention<C, R>(repository: storage::Repository, mut now_ms: C, mut report: R)
where
    C: FnMut() -> i64 + Send,
    R: FnMut(i64, Result<RetentionResult, storage::StorageError>) + Send,
{
    let mut interval = tokio::time::interval(std::time::Duration::from_hours(24));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    interval.tick().await;
    loop {
        interval.tick().await;
        let occurred_at_ms = now_ms();
        let result = repository.run_retention_batch(occurred_at_ms, 500).await;
        report(occurred_at_ms, result);
    }
}

const fn retention_item_count(retention: &RetentionResult) -> u64 {
    retention
        .messages_deleted
        .saturating_add(retention.voice_text_cleared)
        .saturating_add(retention.rejected_proposal_content_cleared)
        .saturating_add(retention.delivered_outbox_deleted)
        .saturating_add(retention.expired_undelivered_outbox_deleted)
        .saturating_add(retention.weather_cache_deleted)
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // Tauri extracts command arguments and managed State by value.
fn api_v1_get_capabilities(
    request: tauri::ipc::Request<'_>,
    capabilities: tauri::State<'_, CapabilitiesService>,
) -> Result<AppCapabilities, ApiError> {
    let request = parse_command_request::<EmptyRequest>(&request)?;
    Ok(capabilities.get_capabilities(request))
}

#[allow(clippy::too_many_lines)] // Composition stays explicit so managed state ownership is auditable.
fn setup_application(app: &mut tauri::App) -> Result<(), Box<dyn Error>> {
    let now_ms = Utc::now().timestamp_millis();
    let paths = AppPaths::create(
        app.path().app_data_dir()?,
        app.path().app_cache_dir()?,
        app.path().app_log_dir()?,
    )?;
    let log_directory = ValidatedLogDirectory::new(paths.log_directory().to_path_buf())?;
    let (mut diagnostic_log, _maintenance) = DiagnosticLog::open(log_directory, now_ms)?;
    diagnostic_log.write(DiagnosticEvent::ApplicationStarted {
        correlation_id: Uuid::now_v7(),
        occurred_at_ms: now_ms,
    })?;
    let (storage, retention) = tauri::async_runtime::block_on(async {
        let storage = Storage::open(&paths, env!("CARGO_PKG_VERSION")).await?;
        let retention = storage
            .repository()
            .run_retention_batch(now_ms, 500)
            .await?;
        Ok::<_, storage::StorageError>((storage, retention))
    })?;
    diagnostic_log.write(DiagnosticEvent::MaintenanceCompleted {
        correlation_id: Uuid::now_v7(),
        occurred_at_ms: now_ms,
        item_count: retention_item_count(&retention),
        bytes_removed: 0,
    })?;
    if retention.expired_undelivered_outbox_deleted > 0 {
        diagnostic_log.write(DiagnosticEvent::OutboxDeliveryExpired {
            correlation_id: Uuid::now_v7(),
            occurred_at_ms: now_ms,
            item_count: retention.expired_undelivered_outbox_deleted,
        })?;
    }
    let storage = Arc::new(storage);
    let repository = storage.repository();
    let retention_repository = repository.clone();
    let retention_app_handle = app.handle().clone();
    let retention_maintenance = RetentionMaintenance(tauri::async_runtime::spawn(async move {
        run_daily_retention(
            retention_repository,
            || Utc::now().timestamp_millis(),
            move |occurred_at_ms, result| {
                let event = match result {
                    Ok(retention) => DiagnosticEvent::MaintenanceCompleted {
                        correlation_id: Uuid::now_v7(),
                        occurred_at_ms,
                        item_count: retention_item_count(&retention),
                        bytes_removed: 0,
                    },
                    Err(_) => DiagnosticEvent::OperationFailed {
                        correlation_id: Uuid::now_v7(),
                        occurred_at_ms,
                        duration_ms: 0,
                        attempt: 1,
                    },
                };
                if let Some(log) = retention_app_handle.try_state::<Mutex<DiagnosticLog>>()
                    && let Ok(mut log) = log.lock()
                {
                    let _ = log.write(event);
                }
            },
        )
        .await;
    }));
    let stored_settings = tauri::async_runtime::block_on(repository.load_provider_settings())?;
    let app_integrations = Arc::new(
        AppIntegrationService::install(
            app,
            AppBehaviorSettings {
                minimize_to_tray: stored_settings.minimize_to_tray,
                launch_at_startup: stored_settings.launch_at_startup,
            },
        )
        .map_err(|_| io::Error::other("app integration unavailable"))?,
    );
    let provider_runtime = Arc::new(
        ProviderRuntime::new().map_err(|_| io::Error::other("provider runtime unavailable"))?,
    );
    let validator: Arc<dyn CandidateSecretValidator> = provider_runtime.clone();
    let voice_previewer: Arc<dyn VoicePreviewer> = Arc::new(SpeechVoicePreviewer::production());
    let process_sequence = Arc::new(ProcessSequence::default());
    let preview_events: Arc<dyn VoicePreviewEventSink> = Arc::new(TauriVoicePreviewEventSink::new(
        app.handle().clone(),
        process_sequence.clone(),
    ));
    let clock = Arc::new(SystemClock);
    let weather_provider: Arc<dyn WeatherProvider> = Arc::new(
        OpenMeteoWeatherProvider::new()
            .map_err(|_| io::Error::other("weather runtime unavailable"))?,
    );
    let weather_service = Arc::new(WeatherService::new(
        repository.clone(),
        weather_provider,
        clock.clone(),
    ));
    let health_probe: Arc<dyn ProviderHealthProbe> = Arc::new(CompositeProviderHealthProbe::new(
        provider_runtime.clone(),
        weather_service.clone(),
    ));
    let provider_service = ProviderService::new(
        repository.clone(),
        Box::new(WindowsCredentialVault::new()?),
        validator,
        health_probe,
        voice_previewer,
        preview_events.clone(),
        clock.clone(),
    )
    .with_settings_effect(Arc::clone(&app_integrations) as Arc<dyn AppSettingsEffect>);
    let schedule_notifications = Arc::new(TauriScheduleNotificationSink::new(app.handle().clone()));
    let scheduler_service = Arc::new(
        SchedulerService::new(
            repository.clone(),
            clock.clone(),
            schedule_notifications.clone(),
            Arc::new(TauriScheduleEventSink::new(app.handle().clone())),
            process_sequence.clone(),
        )
        .map_err(|_| io::Error::other("scheduler runtime unavailable"))?,
    );
    schedule_notifications
        .bind_scheduler(Arc::downgrade(&scheduler_service))
        .map_err(|_| io::Error::other("scheduler notification binding unavailable"))?;
    let onboarding_service = OnboardingService::new(repository.clone());
    let library_root_service = LibraryRootService::new(
        repository.clone(),
        Arc::new(TauriLibraryRootPicker::new(app.handle().clone())),
        Arc::new(SystemLibraryRootClock),
    );
    let track_catalog_service = TrackCatalogService::new(Arc::new(repository.clone()));
    let scanner_service = ScannerService::new(
        repository.clone(),
        Arc::new(TauriScanEventSink::new(app.handle().clone())),
        Arc::new(SystemScanClock),
        process_sequence.clone(),
    );
    tauri::async_runtime::block_on(scanner_service.recover_and_replay())
        .map_err(|_| io::Error::other("scanner recovery unavailable"))?;
    tauri::async_runtime::block_on(repository.recover_interrupted_programs(now_ms))
        .map_err(|_| io::Error::other("program recovery unavailable"))?;
    let repository_track_resolver = RepositoryTrackResolver::new(repository.clone());
    let track_resolver = Arc::new(repository_track_resolver.clone());
    let playback_events: Arc<dyn PlaybackEventSink> =
        Arc::new(TauriPlaybackEventSink::new(app.handle().clone()));
    let playback_event_hub = PlaybackEventHub::new(playback_events, 256);
    let playback_service = PlaybackService::new(
        track_resolver.clone(),
        Box::new(RodioAudioEngine::new()),
        Arc::new(playback_event_hub.clone()),
        Arc::new(SystemPlaybackClock),
        process_sequence.clone(),
        PlaybackService::local_capabilities(),
    )
    .map_err(|_| io::Error::other("playback runtime unavailable"))?;
    let data_control_service = DataControlService::new(DataControlServiceDependencies {
        repository: repository.clone(),
        storage: Arc::clone(&storage),
        paths: paths.clone(),
        vault: Box::new(WindowsCredentialVault::new()?),
        playback: playback_service.clone(),
        picker: Arc::new(TauriDataExportPicker::new(app.handle().clone())),
        events: Arc::new(TauriDataExportEventSink::new(
            app.handle().clone(),
            process_sequence.clone(),
        )),
        reset_integration: app_integrations.clone() as Arc<dyn data_control::DataResetIntegration>,
    });
    let program_credentials = Arc::new(RepositoryProgramCredentialSource::new(
        repository.clone(),
        Box::new(WindowsCredentialVault::new()?),
    ));
    let program_provider = OpenAiProgramPlanProvider::new(
        program_credentials.clone(),
        weather_service.clone(),
        Arc::new(SystemProgramCallContextFactory),
    )
    .ok()
    .map(|provider| Arc::new(provider) as Arc<dyn ProgramPlanProvider>);
    let program_repository: Arc<dyn ProgramRepository> = Arc::new(repository.clone());
    let program_planner = Arc::new(ProgramPlanner::new(
        program_repository,
        program_provider,
        Arc::new(SystemProgramClock),
        Arc::new(SystemProgramIdFactory),
    ));
    let program_context: Arc<dyn ProgramPlannerContextSource> =
        Arc::new(RepositoryProgramContextSource::new(repository.clone()));
    let radio_planner: Arc<dyn ProgramRadioPlanner> =
        Arc::new(DomainProgramPlanner::new(program_planner, program_context));
    let radio_playback = Arc::new(LocalProgramPlayback::new(
        playback_service.clone(),
        repository_track_resolver,
        playback_event_hub.clone(),
    ));
    let radio_speech: Arc<dyn ProgramSpeech> = Arc::new(RepositoryProgramSpeech::production(
        repository.clone(),
        Box::new(WindowsCredentialVault::new()?),
    ));
    let system_radio_speech: Arc<dyn SystemProgramSpeech> =
        Arc::new(RepositorySystemProgramSpeech::production(
            repository.clone(),
            Box::new(WindowsCredentialVault::new()?),
            playback_service.clone(),
        ));
    let apple_companion: Arc<dyn AppleCompanion> = Arc::new(AppleCompanionMonitor::new(
        playback_service.clone(),
        playback_event_hub,
        system_radio_speech,
        Arc::new(repository.clone()),
    ));
    let radio_store: Arc<dyn RadioProgramStore> = Arc::new(repository.clone());
    let radio_service = RadioService::new(RadioServiceDependencies {
        planner: radio_planner,
        authorizer: Arc::new(SchedulerStartAuthorizer::new(Arc::downgrade(
            &scheduler_service,
        ))),
        store: radio_store,
        playback: radio_playback,
        speech: radio_speech,
        apple: apple_companion,
        event_sink: Arc::new(TauriRadioEventSink::new(app.handle().clone())),
        clock: Arc::new(SystemRadioClock),
        sequence: process_sequence.clone(),
        id_factory: Arc::new(SystemRadioIdFactory),
    });
    scheduler_service
        .bind_program_starter(Arc::new(radio_service.clone()) as Arc<dyn NotificationProgramStarter>)
        .map_err(|_| io::Error::other("scheduler program binding unavailable"))?;
    let scheduler_runtime = SchedulerRuntime::new(scheduler_service.clone().start());
    let chat_provider = OpenAiChatProvider::new(program_credentials)
        .ok()
        .map(|provider| Arc::new(provider) as Arc<dyn understanding::ChatProvider>);
    let understanding_service = UnderstandingService::new(
        repository.clone(),
        chat_provider,
        Arc::new(TauriChatEventSink::new(
            app.handle().clone(),
            process_sequence.clone(),
        )),
        clock.clone(),
    );
    let startup_preview_recovery =
        StartupVoicePreviewOutboxRecovery::new(repository, preview_events, clock);
    app.manage(Mutex::new(diagnostic_log));
    app.manage(process_sequence);
    app.manage(storage);
    app.manage(retention_maintenance);
    app.manage(provider_service);
    app.manage(app_integrations);
    app.manage(scheduler_service);
    app.manage(scheduler_runtime);
    app.manage(weather_service);
    app.manage(onboarding_service);
    app.manage(library_root_service);
    app.manage(track_catalog_service);
    app.manage(scanner_service);
    app.manage(track_resolver);
    app.manage(playback_service);
    app.manage(data_control_service);
    app.manage(radio_service);
    app.manage(understanding_service);
    app.manage(startup_preview_recovery);
    Ok(())
}

fn static_capabilities() -> Result<CapabilitiesService, io::Error> {
    let local_source = SourceSummary::new(
        "local",
        SourceKind::Local,
        "本地曲库",
        true,
        PlaybackService::local_capabilities(),
    )
    .map_err(|_| io::Error::other("invalid local capability snapshot"))?;
    let apple_source = SourceSummary::new(
        "apple_music",
        SourceKind::SystemSession,
        "Apple Music Windows App",
        false,
        SourceCapabilities {
            play: false,
            pause: false,
            seek: false,
            next: false,
            previous: false,
            set_queue: false,
        },
    )
    .map_err(|_| io::Error::other("invalid system capability snapshot"))?;
    CapabilitiesService::new(
        env!("CARGO_PKG_VERSION"),
        "Windows",
        true,
        vec![local_source, apple_source],
        Vec::new(),
    )
    .map_err(|_| io::Error::other("invalid static capability snapshot"))
}

/// Starts the desktop shell with the approved local and Windows system-session sources.
///
/// # Errors
///
/// Returns a Tauri error when the desktop runtime cannot be initialized.
pub fn run() -> tauri::Result<()> {
    let capabilities = static_capabilities()?;

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .arg("--silent-start")
                .build(),
        )
        .manage(capabilities)
        .setup(setup_application)
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event
                && let Some(integrations) = window.try_state::<Arc<AppIntegrationService>>()
                && integrations.should_minimize_to_tray()
            {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            api_v1_get_capabilities,
            api_v1_get_onboarding_state,
            api_v1_save_onboarding_step,
            api_v1_list_library_roots,
            api_v1_pick_and_add_library_root,
            api_v1_remove_library_root,
            api_v1_list_tracks,
            api_v1_start_library_scan,
            api_v1_cancel_library_scan,
            api_v1_list_music_sources,
            api_v1_select_music_source,
            api_v1_get_playback_state,
            api_v1_play,
            api_v1_pause,
            api_v1_seek,
            api_v1_next,
            api_v1_previous,
            api_v1_validate_and_set_secret,
            api_v1_delete_secret,
            api_v1_test_provider,
            api_v1_get_settings,
            api_v1_update_settings,
            api_v1_preview_voice,
            api_v1_cancel_operation,
            api_v1_list_voices,
            api_v1_search_weather_locations,
            api_v1_select_weather_location,
            api_v1_list_schedules,
            api_v1_upsert_schedule,
            api_v1_delete_schedule,
            api_v1_handle_notification_action,
            api_v1_start_program,
            api_v1_stop_program,
            api_v1_submit_chat,
            api_v1_submit_feedback,
            api_v1_list_memories,
            api_v1_approve_memory,
            api_v1_update_memory,
            api_v1_delete_memory,
            api_v1_reject_memory_proposal,
            api_v1_get_profile_view,
            api_v1_update_profile,
            api_v1_list_session_summaries,
            api_v1_delete_session_summary,
            api_v1_get_data_inventory,
            api_v1_preview_data_deletion,
            api_v1_delete_data_category,
            api_v1_export_user_data,
            api_v1_delete_all_user_data,
        ])
        .run(tauri::generate_context!())
}

#[cfg(test)]
#[test]
fn static_apple_capability_snapshot_starts_disconnected_and_path_free() {
    let snapshot = static_capabilities()
        .expect("static capabilities")
        .get_capabilities(EmptyRequest {});
    let apple = snapshot
        .sources
        .iter()
        .find(|source| source.source_id == "apple_music")
        .expect("Apple source");
    assert_eq!(apple.display_name, "Apple Music Windows App");
    assert!(!apple.connected);
    assert!(!apple.capabilities.play);
    assert!(!apple.capabilities.set_queue);
}

#[cfg(test)]
mod retention_schedule_tests {
    use super::*;
    use storage::{ChatRole, NewChatMessage, Repository};

    const DAY_MS: i64 = 24 * 60 * 60 * 1_000;

    async fn insert_expiring_message(repository: &Repository) {
        repository
            .create_chat_session("daily-retention-session", 0)
            .await
            .expect("session");
        repository
            .insert_message(NewChatMessage::new(
                "daily-retention-message".to_owned(),
                "daily-retention-session".to_owned(),
                ChatRole::User,
                "private retention fixture".to_owned(),
                None,
                None,
                None,
                0,
            ))
            .await
            .expect("message");
    }

    #[tokio::test]
    async fn daily_retention_runs_after_virtual_twenty_four_hours() {
        let temp = tempfile::tempdir().expect("temporary root");
        let paths = AppPaths::create(
            temp.path().join("data"),
            temp.path().join("cache"),
            temp.path().join("logs"),
        )
        .expect("paths");
        let storage = Storage::open(&paths, "0.4.0").await.expect("storage");
        let repository = storage.repository();
        insert_expiring_message(&repository).await;
        tokio::time::pause();
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(run_daily_retention(
            repository.clone(),
            || 30 * DAY_MS,
            move |_, result| {
                let _ = sender.send(result);
            },
        ));
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_hours(24)).await;
        let result = receiver
            .recv()
            .await
            .expect("scheduled result")
            .expect("cleanup");
        assert_eq!(result.messages_deleted, 1);
        task.abort();
        tokio::time::resume();
        storage.close().await;
    }

    #[tokio::test]
    async fn failed_daily_retention_is_retried_by_next_startup_run() {
        let temp = tempfile::tempdir().expect("temporary root");
        let data = temp.path().join("data");
        let cache = temp.path().join("cache");
        let logs = temp.path().join("logs");
        let paths = AppPaths::create(&data, &cache, &logs).expect("paths");
        let storage = Storage::open(&paths, "0.4.0").await.expect("storage");
        let repository = storage.repository();
        insert_expiring_message(&repository).await;
        storage.close().await;
        tokio::time::pause();

        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(run_daily_retention(
            repository,
            || 30 * DAY_MS,
            move |_, result| {
                let _ = sender.send(result);
            },
        ));
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_hours(24)).await;
        assert!(receiver.recv().await.expect("scheduled failure").is_err());
        task.abort();
        tokio::time::resume();

        let reopened_paths = AppPaths::create(data, cache, logs).expect("reopened paths");
        let reopened = Storage::open(&reopened_paths, "0.4.0")
            .await
            .expect("reopened storage");
        let retry = reopened
            .repository()
            .run_retention_batch(30 * DAY_MS, 500)
            .await
            .expect("startup retry");
        assert_eq!(retry.messages_deleted, 1);
        reopened.close().await;
    }
}
