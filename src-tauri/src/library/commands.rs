//! Strict Tauri command adapters for API-010–012.

use tauri::State;

use super::{
    LibraryRootAck, LibraryRootService, LibraryRootsResponse, PickAndAddLibraryRootRequest,
    PickAndAddLibraryRootResponse, RemoveLibraryRootRequest,
};
use crate::ipc::{ApiError, EmptyRequest, parse_command_request};

/// API-010: lists enabled roots without exposing absolute paths.
///
/// # Errors
///
/// Returns a stable, redacted storage error.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub async fn api_v1_list_library_roots(
    request: tauri::ipc::Request<'_>,
    library_root_service: State<'_, LibraryRootService>,
) -> Result<LibraryRootsResponse, ApiError> {
    let EmptyRequest {} = parse_command_request::<EmptyRequest>(&request)?;
    library_root_service.list_library_roots().await
}

/// API-011: opens the native picker and adds its selected directory.
///
/// # Errors
///
/// Returns stable, redacted picker, path, idempotency, or storage errors.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub async fn api_v1_pick_and_add_library_root(
    request: tauri::ipc::Request<'_>,
    library_root_service: State<'_, LibraryRootService>,
) -> Result<PickAndAddLibraryRootResponse, ApiError> {
    library_root_service
        .pick_and_add_library_root(parse_command_request::<PickAndAddLibraryRootRequest>(
            &request,
        )?)
        .await
}

/// API-012: soft-disables one opaque root ID at the expected revision.
///
/// # Errors
///
/// Returns stable, redacted validation, conflict, idempotency, or storage errors.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub async fn api_v1_remove_library_root(
    request: tauri::ipc::Request<'_>,
    library_root_service: State<'_, LibraryRootService>,
) -> Result<LibraryRootAck, ApiError> {
    library_root_service
        .remove_library_root(parse_command_request::<RemoveLibraryRootRequest>(&request)?)
        .await
}
