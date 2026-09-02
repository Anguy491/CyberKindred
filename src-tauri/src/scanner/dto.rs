use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// API-013 request. An empty root list means every enabled authorized root.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartLibraryScanRequest {
    pub client_request_id: Uuid,
    pub root_ids: Vec<Uuid>,
}

/// API-013 acknowledgement. Completion is delivered through EVT-005.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationAccepted {
    pub operation_id: Uuid,
    pub accepted_at: String,
}

/// API-014 cancellation request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CancelLibraryScanRequest {
    pub client_request_id: Uuid,
    pub operation_id: Uuid,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelLibraryScanState {
    Cancelled,
    AlreadyTerminal,
}

/// API-014 response. `requestId` is the caller's idempotency key.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CancelLibraryScanResponse {
    pub request_id: Uuid,
    pub operation_id: Uuid,
    pub state: CancelLibraryScanState,
}
