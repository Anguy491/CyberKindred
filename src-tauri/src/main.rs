#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

fn main() -> tauri::Result<()> {
    cyberkindred_lib::run()
}
