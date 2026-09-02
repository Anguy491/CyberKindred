#![forbid(unsafe_code)]

/// Starts the minimal desktop shell without registering commands or plugins.
///
/// # Errors
///
/// Returns a Tauri error when the desktop runtime cannot be initialized.
pub fn run() -> tauri::Result<()> {
    tauri::Builder::default().run(tauri::generate_context!())
}
