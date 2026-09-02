use tauri::State;

use crate::{
    contracts::PlaybackState,
    ipc::{ApiError, EmptyRequest, parse_command_request},
};

use super::{
    ListMusicSourcesResponse, PlaybackControlRequest, PlaybackService, SeekPlaybackRequest,
    SelectMusicSourceRequest, SelectMusicSourceResponse,
};

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
/// Handles API-016 without contacting a provider or opening an audio device.
///
/// # Errors
///
/// Returns a safe validation error when the IPC request is malformed.
pub fn api_v1_list_music_sources(
    request: tauri::ipc::Request<'_>,
    service: State<'_, PlaybackService>,
) -> Result<ListMusicSourcesResponse, ApiError> {
    let request = parse_command_request::<EmptyRequest>(&request)?;
    Ok(service.list_music_sources(request))
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
/// Handles API-017 source selection.
///
/// # Errors
///
/// Returns a safe validation, capability, or actor-availability error.
pub async fn api_v1_select_music_source(
    request: tauri::ipc::Request<'_>,
    service: State<'_, PlaybackService>,
) -> Result<SelectMusicSourceResponse, ApiError> {
    let request = parse_command_request::<SelectMusicSourceRequest>(&request)?;
    service.select_music_source(request).await
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
/// Handles API-018 authoritative playback snapshots.
///
/// # Errors
///
/// Returns a safe validation, source, or actor-availability error.
pub async fn api_v1_get_playback_state(
    request: tauri::ipc::Request<'_>,
    service: State<'_, PlaybackService>,
) -> Result<PlaybackState, ApiError> {
    let request = parse_command_request::<EmptyRequest>(&request)?;
    service.get_playback_state(request).await
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
/// Handles API-019 playback start/resume.
///
/// # Errors
///
/// Returns a safe validation, revision, capability, media, or output error.
pub async fn api_v1_play(
    request: tauri::ipc::Request<'_>,
    service: State<'_, PlaybackService>,
) -> Result<PlaybackState, ApiError> {
    let request = parse_command_request::<PlaybackControlRequest>(&request)?;
    service.play(request).await
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
/// Handles API-020 playback pause.
///
/// # Errors
///
/// Returns a safe validation, revision, capability, media, or output error.
pub async fn api_v1_pause(
    request: tauri::ipc::Request<'_>,
    service: State<'_, PlaybackService>,
) -> Result<PlaybackState, ApiError> {
    let request = parse_command_request::<PlaybackControlRequest>(&request)?;
    service.pause(request).await
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
/// Handles API-021 local seek.
///
/// # Errors
///
/// Returns a safe validation, revision, capability, media, or output error.
pub async fn api_v1_seek(
    request: tauri::ipc::Request<'_>,
    service: State<'_, PlaybackService>,
) -> Result<PlaybackState, ApiError> {
    let request = parse_command_request::<SeekPlaybackRequest>(&request)?;
    service.seek(request).await
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
/// Handles API-022 next-track control.
///
/// # Errors
///
/// Returns a safe validation, revision, capability, media, or output error.
pub async fn api_v1_next(
    request: tauri::ipc::Request<'_>,
    service: State<'_, PlaybackService>,
) -> Result<PlaybackState, ApiError> {
    let request = parse_command_request::<PlaybackControlRequest>(&request)?;
    service.next(request).await
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
/// Handles API-023 previous-track control.
///
/// # Errors
///
/// Returns a safe validation, revision, capability, media, or output error.
pub async fn api_v1_previous(
    request: tauri::ipc::Request<'_>,
    service: State<'_, PlaybackService>,
) -> Result<PlaybackState, ApiError> {
    let request = parse_command_request::<PlaybackControlRequest>(&request)?;
    service.previous(request).await
}
