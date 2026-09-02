#![forbid(unsafe_code)]

pub mod contracts;
pub mod diagnostics;
pub mod ipc;
pub mod library;
mod onboarding;
pub mod providers;
pub mod scanner;
pub mod storage;

use chrono::Utc;
use diagnostics::{DiagnosticEvent, DiagnosticLog, ValidatedLogDirectory};
use ipc::{
    ApiError, AppCapabilities, CapabilitiesService, EmptyRequest, ProcessSequence,
    parse_command_request,
};
use library::{
    LibraryRootService, SystemLibraryRootClock, TauriLibraryRootPicker,
    commands::{
        api_v1_list_library_roots, api_v1_pick_and_add_library_root, api_v1_remove_library_root,
    },
};
use onboarding::{
    OnboardingService,
    commands::{api_v1_get_onboarding_state, api_v1_save_onboarding_step},
};
use providers::{
    CandidateSecretValidator, ProviderHealthProbe, ProviderRuntime, ProviderService, SystemClock,
    VoicePreviewEventSink, VoicePreviewer,
    commands::{
        api_v1_delete_secret, api_v1_get_settings, api_v1_list_voices, api_v1_preview_voice,
        api_v1_test_provider, api_v1_update_settings, api_v1_validate_and_set_secret,
    },
    events::{StartupVoicePreviewOutboxRecovery, TauriVoicePreviewEventSink},
};
use scanner::{
    ScannerService, SystemScanClock, TauriScanEventSink,
    commands::{api_v1_cancel_library_scan, api_v1_start_library_scan},
};
use std::{
    error::Error,
    io,
    sync::{Arc, Mutex},
};
use storage::{AppPaths, Storage, WindowsCredentialVault};
use tauri::Manager;
use uuid::Uuid;

#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // Tauri extracts command arguments and managed State by value.
fn api_v1_get_capabilities(
    request: tauri::ipc::Request<'_>,
    capabilities: tauri::State<'_, CapabilitiesService>,
) -> Result<AppCapabilities, ApiError> {
    let request = parse_command_request::<EmptyRequest>(&request)?;
    Ok(capabilities.get_capabilities(request))
}

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
    let retention_item_count = retention
        .messages_deleted
        .saturating_add(retention.voice_text_cleared)
        .saturating_add(retention.delivered_outbox_deleted)
        .saturating_add(retention.expired_undelivered_outbox_deleted);
    diagnostic_log.write(DiagnosticEvent::MaintenanceCompleted {
        correlation_id: Uuid::now_v7(),
        occurred_at_ms: now_ms,
        item_count: retention_item_count,
        bytes_removed: 0,
    })?;
    if retention.expired_undelivered_outbox_deleted > 0 {
        diagnostic_log.write(DiagnosticEvent::OutboxDeliveryExpired {
            correlation_id: Uuid::now_v7(),
            occurred_at_ms: now_ms,
            item_count: retention.expired_undelivered_outbox_deleted,
        })?;
    }
    let repository = storage.repository();
    let provider_runtime = Arc::new(
        ProviderRuntime::new().map_err(|_| io::Error::other("provider runtime unavailable"))?,
    );
    let validator: Arc<dyn CandidateSecretValidator> = provider_runtime.clone();
    let health_probe: Arc<dyn ProviderHealthProbe> = provider_runtime.clone();
    let voice_previewer: Arc<dyn VoicePreviewer> = provider_runtime;
    let process_sequence = Arc::new(ProcessSequence::default());
    let preview_events: Arc<dyn VoicePreviewEventSink> = Arc::new(TauriVoicePreviewEventSink::new(
        app.handle().clone(),
        process_sequence.clone(),
    ));
    let clock = Arc::new(SystemClock);
    let provider_service = ProviderService::new(
        repository.clone(),
        Box::new(WindowsCredentialVault::new()?),
        validator,
        health_probe,
        voice_previewer,
        preview_events.clone(),
        clock.clone(),
    );
    let onboarding_service = OnboardingService::new(repository.clone());
    let library_root_service = LibraryRootService::new(
        repository.clone(),
        Arc::new(TauriLibraryRootPicker::new(app.handle().clone())),
        Arc::new(SystemLibraryRootClock),
    );
    let scanner_service = ScannerService::new(
        repository.clone(),
        Arc::new(TauriScanEventSink::new(app.handle().clone())),
        Arc::new(SystemScanClock),
        process_sequence.clone(),
    );
    tauri::async_runtime::block_on(scanner_service.recover_and_replay())
        .map_err(|_| io::Error::other("scanner recovery unavailable"))?;
    let startup_preview_recovery =
        StartupVoicePreviewOutboxRecovery::new(repository, preview_events, clock);
    app.manage(Mutex::new(diagnostic_log));
    app.manage(process_sequence);
    app.manage(storage);
    app.manage(provider_service);
    app.manage(onboarding_service);
    app.manage(library_root_service);
    app.manage(scanner_service);
    app.manage(startup_preview_recovery);
    Ok(())
}

/// Starts the desktop shell with the M2 storage and read-only IPC foundation.
///
/// # Errors
///
/// Returns a Tauri error when the desktop runtime cannot be initialized.
pub fn run() -> tauri::Result<()> {
    let capabilities = CapabilitiesService::foundation(env!("CARGO_PKG_VERSION"), "Windows")
        .map_err(|_| io::Error::other("invalid static capability snapshot"))?;

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(capabilities)
        .setup(setup_application)
        .invoke_handler(tauri::generate_handler![
            api_v1_get_capabilities,
            api_v1_get_onboarding_state,
            api_v1_save_onboarding_step,
            api_v1_list_library_roots,
            api_v1_pick_and_add_library_root,
            api_v1_remove_library_root,
            api_v1_start_library_scan,
            api_v1_cancel_library_scan,
            api_v1_validate_and_set_secret,
            api_v1_delete_secret,
            api_v1_test_provider,
            api_v1_get_settings,
            api_v1_update_settings,
            api_v1_preview_voice,
            api_v1_list_voices,
        ])
        .run(tauri::generate_context!())
}
