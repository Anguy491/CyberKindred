use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::contracts::ProgramPlan;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StartProgramTrigger {
    Manual,
    Notification,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartProgramRequest {
    pub client_request_id: Uuid,
    pub source_id: String,
    pub trigger: StartProgramTrigger,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartProgramResponse {
    pub request_id: Uuid,
    pub program_id: Uuid,
    pub plan: Option<ProgramPlan>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StopProgramRequest {
    pub client_request_id: Uuid,
    pub program_id: Uuid,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgramAck {
    pub request_id: Uuid,
    pub revision: u64,
}
