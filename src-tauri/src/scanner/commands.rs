//! Strict Tauri command adapters for API-013 and API-014.

use std::sync::Arc;
use tauri::State;

use super::{
    CancelLibraryScanRequest, CancelLibraryScanResponse, OperationAccepted, ScannerService,
    StartLibraryScanRequest,
};
use crate::ipc::{ApiError, parse_command_request};

/// API-013: accepts one bounded, cancellable local-library scan.
///
/// # Errors
///
/// Returns a strict request, root authorization, busy, path, or storage error.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub async fn api_v1_start_library_scan(
    request: tauri::ipc::Request<'_>,
    scanner_service: State<'_, Arc<ScannerService>>,
) -> Result<OperationAccepted, ApiError> {
    scanner_service
        .start_scan(parse_command_request::<StartLibraryScanRequest>(&request)?)
        .await
}

/// API-014: requests cancellation without waiting for traversal to unwind.
///
/// # Errors
///
/// Returns a strict request, not-found, conflict, or storage error.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub async fn api_v1_cancel_library_scan(
    request: tauri::ipc::Request<'_>,
    scanner_service: State<'_, Arc<ScannerService>>,
) -> Result<CancelLibraryScanResponse, ApiError> {
    scanner_service
        .cancel_scan(parse_command_request::<CancelLibraryScanRequest>(&request)?)
        .await
}
