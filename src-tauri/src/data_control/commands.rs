#![allow(clippy::missing_errors_doc)] // Thin Tauri adapters forward documented service errors.

use tauri::State;

use super::{
    DataControlService, DeleteAllUserDataRequest, DeleteAllUserDataResponse,
    DeleteDataCategoryRequest, DeleteDataCategoryResponse, ExportUserDataRequest,
    GetDataInventoryResponse, PreviewDataDeletionRequest, PreviewDataDeletionResponse,
};
use crate::{
    ipc::{ApiError, EmptyRequest, parse_command_request},
    providers::OperationAccepted,
};

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub async fn api_v1_get_data_inventory(
    request: tauri::ipc::Request<'_>,
    service: State<'_, DataControlService>,
) -> Result<GetDataInventoryResponse, ApiError> {
    service
        .get_inventory(parse_command_request::<EmptyRequest>(&request)?)
        .await
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub async fn api_v1_preview_data_deletion(
    request: tauri::ipc::Request<'_>,
    service: State<'_, DataControlService>,
) -> Result<PreviewDataDeletionResponse, ApiError> {
    service
        .preview_deletion(parse_command_request::<PreviewDataDeletionRequest>(
            &request,
        )?)
        .await
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub async fn api_v1_delete_data_category(
    request: tauri::ipc::Request<'_>,
    service: State<'_, DataControlService>,
) -> Result<DeleteDataCategoryResponse, ApiError> {
    service
        .delete_category(parse_command_request::<DeleteDataCategoryRequest>(
            &request,
        )?)
        .await
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub async fn api_v1_export_user_data(
    request: tauri::ipc::Request<'_>,
    service: State<'_, DataControlService>,
) -> Result<OperationAccepted, ApiError> {
    service
        .export_user_data(parse_command_request::<ExportUserDataRequest>(&request)?)
        .await
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub async fn api_v1_delete_all_user_data(
    request: tauri::ipc::Request<'_>,
    service: State<'_, DataControlService>,
) -> Result<DeleteAllUserDataResponse, ApiError> {
    service
        .delete_all_user_data(parse_command_request::<DeleteAllUserDataRequest>(&request)?)
        .await
}
