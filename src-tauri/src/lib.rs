#![forbid(unsafe_code)]

pub mod contracts;
pub mod storage;

use chrono::Utc;
use storage::{AppPaths, Storage};
use tauri::Manager;

/// Starts the desktop shell with the M2 storage and read-only IPC foundation.
///
/// # Errors
///
/// Returns a Tauri error when the desktop runtime cannot be initialized.
pub fn run() -> tauri::Result<()> {
    tauri::Builder::default()
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
        .run(tauri::generate_context!())
}
