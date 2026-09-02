//! Tauri command boundary for the persisted onboarding aggregate.

use tauri::State;

use super::{Ack, OnboardingService, OnboardingState, SaveOnboardingStepRequest};
use crate::ipc::{ApiError, EmptyRequest, parse_command_request};

/// API-002: returns the exact authoritative onboarding state.
///
/// # Errors
///
/// Returns a stable storage error if the persisted aggregate is unavailable or invalid.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub async fn api_v1_get_onboarding_state(
    request: tauri::ipc::Request<'_>,
    onboarding_service: State<'_, OnboardingService>,
) -> Result<OnboardingState, ApiError> {
    let EmptyRequest {} = parse_command_request::<EmptyRequest>(&request)?;
    onboarding_service.get_state().await
}

/// API-003: atomically saves one allowed onboarding step.
///
/// # Errors
///
/// Returns a stable request, conflict, transition, or storage error.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub async fn api_v1_save_onboarding_step(
    request: tauri::ipc::Request<'_>,
    onboarding_service: State<'_, OnboardingService>,
) -> Result<Ack, ApiError> {
    onboarding_service
        .save_step(parse_command_request::<SaveOnboardingStepRequest>(
            &request,
        )?)
        .await
}
