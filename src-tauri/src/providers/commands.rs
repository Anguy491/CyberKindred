//! Tauri command boundary for provider configuration and explicit provider actions.

use tauri::State;

use super::events::StartupVoicePreviewOutboxRecovery;
use super::{
    Ack, CancelOperationRequest, CancelOperationResponse, DeleteSecretRequest,
    DeleteSecretResponse, ListVoicesRequest, OperationAccepted, PreviewVoiceRequest,
    ProviderService, SettingsView, TestProviderRequest, TestProviderResponse,
    UpdateSettingsRequest, ValidateSecretRequest, ValidateSecretResponse, VoicesResponse,
};
use crate::ipc::{ApiError, EmptyRequest, parse_command_request};
use crate::understanding::UnderstandingService;

/// API-004: validates a candidate credential before storing it for its exact origin.
///
/// # Errors
///
/// Returns the service's stable, redacted validation or provider error.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub async fn api_v1_validate_and_set_secret(
    request: tauri::ipc::Request<'_>,
    provider_service: State<'_, ProviderService>,
) -> Result<ValidateSecretResponse, ApiError> {
    provider_service
        .validate_and_set_secret(parse_command_request::<ValidateSecretRequest>(&request)?)
        .await
}

/// API-005: deletes only the credential identified by the strict request DTO.
///
/// # Errors
///
/// Returns the service's stable, redacted credential or storage error.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub async fn api_v1_delete_secret(
    request: tauri::ipc::Request<'_>,
    provider_service: State<'_, ProviderService>,
) -> Result<DeleteSecretResponse, ApiError> {
    provider_service
        .delete_secret(parse_command_request::<DeleteSecretRequest>(&request)?)
        .await
}

/// API-006: runs a single user-triggered provider connectivity test.
///
/// # Errors
///
/// Returns the service's stable, redacted provider or configuration error.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub async fn api_v1_test_provider(
    request: tauri::ipc::Request<'_>,
    provider_service: State<'_, ProviderService>,
) -> Result<TestProviderResponse, ApiError> {
    provider_service
        .test_provider(parse_command_request::<TestProviderRequest>(&request)?)
        .await
}

/// API-007: returns the exact non-secret settings view.
///
/// # Errors
///
/// Returns the service's stable, redacted credential or storage error.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub async fn api_v1_get_settings(
    request: tauri::ipc::Request<'_>,
    provider_service: State<'_, ProviderService>,
) -> Result<SettingsView, ApiError> {
    let EmptyRequest {} = parse_command_request::<EmptyRequest>(&request)?;
    provider_service.get_settings().await
}

/// API-008: applies a strict settings patch with optimistic concurrency.
///
/// # Errors
///
/// Returns the service's stable, redacted validation, conflict, or storage error.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub async fn api_v1_update_settings(
    request: tauri::ipc::Request<'_>,
    provider_service: State<'_, ProviderService>,
) -> Result<Ack, ApiError> {
    provider_service
        .update_settings(parse_command_request::<UpdateSettingsRequest>(&request)?)
        .await
}

/// API-009: accepts one user-triggered voice preview operation.
///
/// # Errors
///
/// Returns the service's stable, redacted validation or provider error.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub async fn api_v1_preview_voice(
    request: tauri::ipc::Request<'_>,
    provider_service: State<'_, ProviderService>,
    startup_recovery: State<'_, StartupVoicePreviewOutboxRecovery>,
) -> Result<OperationAccepted, ApiError> {
    let request = parse_command_request::<PreviewVoiceRequest>(&request)?;
    startup_recovery.ensure_recovered().await?;
    provider_service.preview_voice(request).await
}

/// API-038: idempotently cancels an accepted operation of the asserted kind.
///
/// # Errors
///
/// Returns a stable validation, capability, not-found, or storage error.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub async fn api_v1_cancel_operation(
    request: tauri::ipc::Request<'_>,
    provider_service: State<'_, ProviderService>,
    understanding_service: State<'_, UnderstandingService>,
    startup_recovery: State<'_, StartupVoicePreviewOutboxRecovery>,
) -> Result<CancelOperationResponse, ApiError> {
    let request = parse_command_request::<CancelOperationRequest>(&request)?;
    if request.expected_kind == super::OperationKind::Chat {
        return understanding_service
            .cancel_chat(request.client_request_id, request.operation_id)
            .await;
    }
    startup_recovery.ensure_recovered().await?;
    provider_service.cancel_operation(request).await
}

/// API-043: returns the validated TTS voice catalog.
///
/// # Errors
///
/// Returns the service's stable, redacted provider-response validation error.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
#[allow(clippy::unused_async)] // All public command adapters share one async boundary.
pub async fn api_v1_list_voices(
    request: tauri::ipc::Request<'_>,
    provider_service: State<'_, ProviderService>,
) -> Result<VoicesResponse, ApiError> {
    provider_service.list_voices(parse_command_request::<ListVoicesRequest>(&request)?)
}
