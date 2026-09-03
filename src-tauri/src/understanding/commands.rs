use tauri::State;

use crate::{
    contracts::MemoryRecord,
    ipc::{ApiError, EmptyRequest, parse_command_request},
    providers::{Ack, OperationAccepted},
};

use super::{
    DeleteSummaryRequest, ListMemoriesRequest, MemoryMutationRequest, MemoryPage, PageRequest,
    ProfileViewResponse, RejectMemoryResponse, SessionSummaryPage, SubmitChatRequest,
    SubmitFeedbackRequest, UnderstandingService, UpdateMemoryRequest, UpdateProfileRequest,
};

macro_rules! service_command {
    ($name:ident, $request:ty, $response:ty, $method:ident) => {
        /// Strict Tauri adapter for one M4 API command.
        ///
        /// # Errors
        ///
        /// Returns a stable validated service error.
        #[tauri::command]
        #[allow(clippy::needless_pass_by_value)]
        pub async fn $name(
            request: tauri::ipc::Request<'_>,
            service: State<'_, UnderstandingService>,
        ) -> Result<$response, ApiError> {
            service
                .$method(parse_command_request::<$request>(&request)?)
                .await
        }
    };
}

service_command!(
    api_v1_submit_chat,
    SubmitChatRequest,
    OperationAccepted,
    submit_chat
);
service_command!(
    api_v1_submit_feedback,
    SubmitFeedbackRequest,
    Ack,
    submit_feedback
);
service_command!(
    api_v1_list_memories,
    ListMemoriesRequest,
    MemoryPage,
    list_memories
);
service_command!(
    api_v1_approve_memory,
    MemoryMutationRequest,
    MemoryRecord,
    approve_memory
);
service_command!(
    api_v1_update_memory,
    UpdateMemoryRequest,
    MemoryRecord,
    update_memory
);
service_command!(
    api_v1_delete_memory,
    MemoryMutationRequest,
    Ack,
    delete_memory
);
service_command!(
    api_v1_reject_memory_proposal,
    MemoryMutationRequest,
    RejectMemoryResponse,
    reject_memory
);
service_command!(
    api_v1_update_profile,
    UpdateProfileRequest,
    Ack,
    update_profile
);
service_command!(
    api_v1_list_session_summaries,
    PageRequest,
    SessionSummaryPage,
    list_summaries
);
service_command!(
    api_v1_delete_session_summary,
    DeleteSummaryRequest,
    Ack,
    delete_summary
);

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
/// Returns the current path-free profile projection.
///
/// # Errors
///
/// Returns a stable validation or storage error.
pub async fn api_v1_get_profile_view(
    request: tauri::ipc::Request<'_>,
    service: State<'_, UnderstandingService>,
) -> Result<ProfileViewResponse, ApiError> {
    let EmptyRequest {} = parse_command_request::<EmptyRequest>(&request)?;
    service.get_profile_view().await
}
