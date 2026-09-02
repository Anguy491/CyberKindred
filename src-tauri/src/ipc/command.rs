use serde::{Deserialize, de::DeserializeOwned};
use tauri::ipc::{InvokeBody, Request};

use super::{ApiError, InternalReason, PublicField};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandEnvelope<T> {
    request: T,
}

/// Deserializes a strict request DTO inside the command boundary so every
/// malformed or missing payload becomes the same redacted `ApiError`.
///
/// # Errors
///
/// Returns `ERR-1001` without retaining or echoing the rejected value.
pub fn parse_command_request<T: DeserializeOwned>(request: &Request<'_>) -> Result<T, ApiError> {
    parse_invoke_body(request.body())
}

fn parse_invoke_body<T: DeserializeOwned>(body: &InvokeBody) -> Result<T, ApiError> {
    let InvokeBody::Json(value) = body else {
        return Err(invalid_request());
    };
    serde_json::from_value::<CommandEnvelope<T>>(value.clone())
        .map(|envelope| envelope.request)
        .map_err(|_| invalid_request())
}

fn invalid_request() -> ApiError {
    ApiError::from_reason(InternalReason::RequestInvalid).with_field(PublicField::Request)
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::json;

    use super::*;
    use crate::ipc::ErrorId;

    #[derive(Debug, Deserialize, Eq, PartialEq)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct StrictFixture {
        expected_revision: u64,
    }

    #[test]
    fn command_boundary_accepts_only_the_strict_nested_request() {
        assert_eq!(
            parse_invoke_body::<StrictFixture>(&InvokeBody::Json(json!({
                "request": { "expectedRevision": 7 }
            }))),
            Ok(StrictFixture {
                expected_revision: 7,
            })
        );
        for body in [
            InvokeBody::Raw(b"secret-sk-canary".to_vec()),
            InvokeBody::Json(json!(null)),
            InvokeBody::Json(json!({})),
            InvokeBody::Json(json!({
                "request": { "expectedRevision": 7 },
                "unexpected": true
            })),
            InvokeBody::Json(json!({
                "request": { "expectedRevision": 7, "secret": "sk-canary" }
            })),
        ] {
            let error = parse_invoke_body::<StrictFixture>(&body)
                .expect_err("malformed command payload must fail closed");
            assert_eq!(error.error_id, ErrorId::RequestInvalid);
            let encoded = serde_json::to_string(&error).expect("safe error must serialize");
            assert!(!encoded.contains("sk-canary"));
        }
    }
}
