use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use tauri::{
    App, AppHandle, Manager, Wry,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};
use tauri_plugin_autostart::ManagerExt;
use uuid::Uuid;

use crate::{
    contracts::PlaybackStateStatus,
    data_control::DataResetIntegration,
    ipc::{ApiError, EmptyRequest, InternalReason},
    playback::{PlaybackControlRequest, PlaybackService},
    providers::{AppBehaviorSettings, AppSettingsEffect},
    radio::{RadioService, StopProgramRequest},
};

const TRAY_ID: &str = "cyberkindred-tray";
const OPEN_ID: &str = "cyberkindred-open";
const PLAY_PAUSE_ID: &str = "cyberkindred-play-pause";
const STOP_ID: &str = "cyberkindred-stop";
const EXIT_ID: &str = "cyberkindred-exit";

#[derive(Clone)]
pub struct AppIntegrationService {
    app_handle: AppHandle,
    minimize_to_tray: Arc<AtomicBool>,
}

impl AppIntegrationService {
    /// Installs the fixed tray menu and reconciles the persisted per-user autostart state.
    ///
    /// # Errors
    ///
    /// Returns a stable error if the tray or autostart integration cannot be configured.
    pub fn install(app: &App<Wry>, settings: AppBehaviorSettings) -> Result<Self, ApiError> {
        install_tray(app, settings.minimize_to_tray)?;
        let service = Self {
            app_handle: app.handle().clone(),
            minimize_to_tray: Arc::new(AtomicBool::new(settings.minimize_to_tray)),
        };
        service.set_autostart(settings.launch_at_startup)?;
        Ok(service)
    }

    #[must_use]
    pub fn should_minimize_to_tray(&self) -> bool {
        self.minimize_to_tray.load(Ordering::Acquire)
    }

    fn set_autostart(&self, enabled: bool) -> Result<(), ApiError> {
        let manager = self.app_handle.autolaunch();
        let current = manager.is_enabled().map_err(|_| os_error())?;
        if current == enabled {
            return Ok(());
        }
        if enabled {
            manager.enable()
        } else {
            manager.disable()
        }
        .map_err(|_| os_error())
    }

    fn set_tray_visible(&self, visible: bool) -> Result<(), ApiError> {
        let tray = self.app_handle.tray_by_id(TRAY_ID).ok_or_else(os_error)?;
        tray.set_visible(visible).map_err(|_| os_error())?;
        self.minimize_to_tray.store(visible, Ordering::Release);
        Ok(())
    }
}

impl AppSettingsEffect for AppIntegrationService {
    fn apply(
        &self,
        previous: AppBehaviorSettings,
        next: AppBehaviorSettings,
    ) -> Result<(), ApiError> {
        if previous.launch_at_startup != next.launch_at_startup {
            self.set_autostart(next.launch_at_startup)?;
        }
        if previous.minimize_to_tray != next.minimize_to_tray
            && let Err(error) = self.set_tray_visible(next.minimize_to_tray)
        {
            if previous.launch_at_startup != next.launch_at_startup {
                self.set_autostart(previous.launch_at_startup)?;
            }
            return Err(error);
        }
        Ok(())
    }
}

impl DataResetIntegration for AppIntegrationService {
    fn reset(&self) -> Result<(), ApiError> {
        self.set_autostart(false)?;
        self.set_tray_visible(false)
    }
}

fn install_tray(app: &App<Wry>, visible: bool) -> Result<(), ApiError> {
    let open = MenuItem::with_id(app, OPEN_ID, "打开 CyberKindred", true, None::<&str>)
        .map_err(|_| os_error())?;
    let play_pause = MenuItem::with_id(app, PLAY_PAUSE_ID, "播放/暂停", true, None::<&str>)
        .map_err(|_| os_error())?;
    let stop =
        MenuItem::with_id(app, STOP_ID, "结束节目", true, None::<&str>).map_err(|_| os_error())?;
    let exit =
        MenuItem::with_id(app, EXIT_ID, "退出", true, None::<&str>).map_err(|_| os_error())?;
    let menu =
        Menu::with_items(app, &[&open, &play_pause, &stop, &exit]).map_err(|_| os_error())?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip("CyberKindred")
        .on_menu_event(|app, event| handle_tray_menu(app, &event));
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    let tray = builder.build(app).map_err(|_| os_error())?;
    tray.set_visible(visible).map_err(|_| os_error())
}

fn handle_tray_menu(app: &AppHandle, event: &tauri::menu::MenuEvent) {
    match event.id().as_ref() {
        OPEN_ID => show_main_window(app),
        PLAY_PAUSE_ID => toggle_playback(app),
        STOP_ID => stop_active_program(app),
        EXIT_ID => app.exit(0),
        _ => {}
    }
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn toggle_playback(app: &AppHandle) {
    let playback = app.state::<PlaybackService>().inner().clone();
    tauri::async_runtime::spawn(async move {
        let Ok(state) = playback.get_playback_state(EmptyRequest {}).await else {
            return;
        };
        let request = PlaybackControlRequest {
            client_request_id: Uuid::now_v7(),
            expected_state_revision: state.revision,
        };
        match state.status {
            PlaybackStateStatus::Playing if state.capabilities.pause => {
                let _ = playback.pause(request).await;
            }
            PlaybackStateStatus::Paused | PlaybackStateStatus::Stopped
                if state.capabilities.play =>
            {
                let _ = playback.play(request).await;
            }
            _ => {}
        }
    });
}

fn stop_active_program(app: &AppHandle) {
    let radio = app.state::<RadioService>().inner().clone();
    tauri::async_runtime::spawn(async move {
        if let Some(program_id) = radio.active_program_id().await {
            let _ = radio
                .stop_program(StopProgramRequest {
                    client_request_id: Uuid::now_v7(),
                    program_id,
                })
                .await;
        }
    });
}

fn os_error() -> ApiError {
    ApiError::from_reason(InternalReason::StorageWriteFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_behavior_defaults_are_silent_and_opt_in() {
        let defaults = AppBehaviorSettings {
            minimize_to_tray: false,
            launch_at_startup: false,
        };
        assert!(!defaults.minimize_to_tray);
        assert!(!defaults.launch_at_startup);
    }
}
