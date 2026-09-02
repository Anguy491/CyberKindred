//! Strict Tauri adapters for API-024 and API-025.

use tauri::State;

use crate::ipc::{ApiError, parse_command_request};

use super::{
    ProgramAck, RadioService, StartProgramRequest, StartProgramResponse, StopProgramRequest,
};

/// API-024: starts one confirmed local program.
///
/// # Errors
///
/// Returns only strict, redacted public errors.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub async fn api_v1_start_program(
    request: tauri::ipc::Request<'_>,
    service: State<'_, RadioService>,
) -> Result<StartProgramResponse, ApiError> {
    service
        .start_local_program(parse_command_request::<StartProgramRequest>(&request)?)
        .await
}

/// API-025: idempotently stops the addressed local program.
///
/// # Errors
///
/// Returns only strict, redacted public errors.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub async fn api_v1_stop_program(
    request: tauri::ipc::Request<'_>,
    service: State<'_, RadioService>,
) -> Result<ProgramAck, ApiError> {
    service
        .stop_program(parse_command_request::<StopProgramRequest>(&request)?)
        .await
}
