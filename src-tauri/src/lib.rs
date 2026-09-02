#![forbid(unsafe_code)]

pub mod contracts;
pub mod ipc;
pub mod storage;

use chrono::Utc;
use ipc::{AppCapabilities, CapabilitiesService, EmptyRequest};
use std::io;
use storage::{AppPaths, Storage};
use tauri::Manager;

#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // Tauri extracts managed State by value.
fn api_v1_get_capabilities(capabilities: tauri::State<'_, CapabilitiesService>) -> AppCapabilities {
    capabilities.get_capabilities(EmptyRequest {})
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
        .manage(capabilities)
        .setup(|app| {
            let paths = AppPaths::create(
                app.path().app_data_dir()?,
                app.path().app_cache_dir()?,
                app.path().app_log_dir()?,
            )?;
            let storage = tauri::async_runtime::block_on(async {
                let storage = Storage::open(&paths, env!("CARGO_PKG_VERSION")).await?;
                storage
                    .repository()
                    .run_retention_batch(Utc::now().timestamp_millis(), 500)
                    .await?;
                Ok::<Storage, storage::StorageError>(storage)
            })?;
            app.manage(storage);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![api_v1_get_capabilities])
        .run(tauri::generate_context!())
}
