use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const IPC_SCHEMA_VERSION: &str = "1.0.0";

/// Public error identifiers fixed by API Contract v1.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ErrorId {
    #[serde(rename = "ERR-1001")]
    RequestInvalid,
    #[serde(rename = "ERR-1002")]
    ProtocolUnsupported,
    #[serde(rename = "ERR-1003")]
    Conflict,
    #[serde(rename = "ERR-1004")]
    NotFound,
    #[serde(rename = "ERR-1005")]
    ResourceBusy,
    #[serde(rename = "ERR-1006")]
    OperationCancelled,
    #[serde(rename = "ERR-1101")]
    SecretMissing,
    #[serde(rename = "ERR-1102")]
    SecretRejected,
    #[serde(rename = "ERR-1201")]
    CapabilityUnsupported,
    #[serde(rename = "ERR-1202")]
    SourceUnavailable,
    #[serde(rename = "ERR-1203")]
    MediaSessionChanged,
    #[serde(rename = "ERR-1204")]
    MediaInvalid,
    #[serde(rename = "ERR-1301")]
    ProviderAuthentication,
    #[serde(rename = "ERR-1302")]
    ProviderRateLimited,
    #[serde(rename = "ERR-1303")]
    ProviderTimedOut,
    #[serde(rename = "ERR-1304")]
    ProviderUnavailable,
    #[serde(rename = "ERR-1305")]
    ProviderInvalidResponse,
    #[serde(rename = "ERR-1401")]
    StorageFailed,
    #[serde(rename = "ERR-1402")]
    MigrationFailed,
    #[serde(rename = "ERR-1501")]
    PathDenied,
    #[serde(rename = "ERR-1502")]
    ConfirmationMissing,
    #[serde(rename = "ERR-1601")]
    UnexpectedInternal,
}

impl ErrorId {
    const fn safe_message(self) -> &'static str {
        match self {
            Self::RequestInvalid => "请求内容无效，请检查后重试。",
            Self::ProtocolUnsupported => "应用协议版本不受支持，请更新应用。",
            Self::Conflict => "状态已经变化，请刷新后重试。",
            Self::NotFound => "请求的内容已不存在，请刷新后重试。",
            Self::ResourceBusy => "资源正忙，请稍后重试。",
            Self::OperationCancelled => "操作已取消。",
            Self::SecretMissing => "尚未配置所需凭据。",
            Self::SecretRejected => "凭据验证失败，请更换后重试。",
            Self::CapabilityUnsupported => "当前来源不支持此操作。",
            Self::SourceUnavailable => "音乐来源当前不可用，请重新连接。",
            Self::MediaSessionChanged => "媒体会话已变化，请刷新后重试。",
            Self::MediaInvalid => "媒体文件或元数据无效，已跳过此项目。",
            Self::ProviderAuthentication => "服务认证失败，请检查连接设置。",
            Self::ProviderRateLimited => "服务请求过于频繁，请稍后重试。",
            Self::ProviderTimedOut => "服务响应超时，请稍后重试。",
            Self::ProviderUnavailable => "服务或网络当前不可用。",
            Self::ProviderInvalidResponse => "服务返回了无效内容，已安全忽略。",
            Self::StorageFailed => "本地存储操作失败，未继续写入。",
            Self::MigrationFailed => "本地数据升级失败，应用已进入恢复模式。",
            Self::PathDenied => "无法访问该位置，请重新选择授权目录。",
            Self::ConfirmationMissing => "需要输入准确的删除确认文字。",
            Self::UnexpectedInternal => "发生了内部错误，请重新打开当前页面。",
        }
    }

    const fn retryable(self) -> bool {
        matches!(
            self,
            Self::ResourceBusy
                | Self::SourceUnavailable
                | Self::MediaSessionChanged
                | Self::ProviderRateLimited
                | Self::ProviderTimedOut
                | Self::ProviderUnavailable
        )
    }
}

/// Internal reasons are exhaustive so call sites cannot invent public errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InternalReason {
    RequestInvalid,
    UnknownField,
    InvalidPatch,
    InvalidCandidate,
    ProtocolUnsupported,
    RevisionConflict,
    IdempotencyPayloadConflict,
    PreviewTokenStale,
    EntityNotFound,
    OperationNotFound,
    ResourceBusy,
    OperationAlreadyRunning,
    OperationCancelled,
    SecretMissing,
    SecretValidationRejected,
    CapabilityAbsent,
    SourceUnavailable,
    SessionIdentityChanged,
    StateRevisionChanged,
    UserMediaOverride,
    MediaUnreadable,
    MediaMetadataInvalid,
    ProviderAuthentication,
    ProviderRateLimit,
    ProviderTimeout,
    ProviderUnavailable,
    NetworkUnavailable,
    ProviderInvalidResponse,
    ProviderOutputPolicyViolation,
    StorageReadFailed,
    StorageWriteFailed,
    StorageIntegrityFailed,
    MigrationFailed,
    DatabaseVersionUnsupported,
    PathDenied,
    PathOutsideRoot,
    UnsafeReparsePoint,
    DestructiveConfirmationMissing,
    DestructiveConfirmationMismatch,
    UnexpectedInternal,
}

impl InternalReason {
    /// Every allowed reason, used by exhaustive contract tests.
    pub const ALL: [Self; 40] = [
        Self::RequestInvalid,
        Self::UnknownField,
        Self::InvalidPatch,
        Self::InvalidCandidate,
        Self::ProtocolUnsupported,
        Self::RevisionConflict,
        Self::IdempotencyPayloadConflict,
        Self::PreviewTokenStale,
        Self::EntityNotFound,
        Self::OperationNotFound,
        Self::ResourceBusy,
        Self::OperationAlreadyRunning,
        Self::OperationCancelled,
        Self::SecretMissing,
        Self::SecretValidationRejected,
        Self::CapabilityAbsent,
        Self::SourceUnavailable,
        Self::SessionIdentityChanged,
        Self::StateRevisionChanged,
        Self::UserMediaOverride,
        Self::MediaUnreadable,
        Self::MediaMetadataInvalid,
        Self::ProviderAuthentication,
        Self::ProviderRateLimit,
        Self::ProviderTimeout,
        Self::ProviderUnavailable,
        Self::NetworkUnavailable,
        Self::ProviderInvalidResponse,
        Self::ProviderOutputPolicyViolation,
        Self::StorageReadFailed,
        Self::StorageWriteFailed,
        Self::StorageIntegrityFailed,
        Self::MigrationFailed,
        Self::DatabaseVersionUnsupported,
        Self::PathDenied,
        Self::PathOutsideRoot,
        Self::UnsafeReparsePoint,
        Self::DestructiveConfirmationMissing,
        Self::DestructiveConfirmationMismatch,
        Self::UnexpectedInternal,
    ];

    /// Stable snake-case reason code allowed to cross IPC.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RequestInvalid => "request_invalid",
            Self::UnknownField => "unknown_field",
            Self::InvalidPatch => "invalid_patch",
            Self::InvalidCandidate => "invalid_candidate",
            Self::ProtocolUnsupported => "protocol_unsupported",
            Self::RevisionConflict => "revision_conflict",
            Self::IdempotencyPayloadConflict => "idempotency_payload_conflict",
            Self::PreviewTokenStale => "preview_token_stale",
            Self::EntityNotFound => "entity_not_found",
            Self::OperationNotFound => "operation_not_found",
            Self::ResourceBusy => "resource_busy",
            Self::OperationAlreadyRunning => "operation_already_running",
            Self::OperationCancelled => "operation_cancelled",
            Self::SecretMissing => "secret_missing",
            Self::SecretValidationRejected => "secret_validation_rejected",
            Self::CapabilityAbsent => "capability_absent",
            Self::SourceUnavailable => "source_unavailable",
            Self::SessionIdentityChanged => "session_identity_changed",
            Self::StateRevisionChanged => "state_revision_changed",
            Self::UserMediaOverride => "user_media_override",
            Self::MediaUnreadable => "media_unreadable",
            Self::MediaMetadataInvalid => "media_metadata_invalid",
            Self::ProviderAuthentication => "provider_authentication",
            Self::ProviderRateLimit => "provider_rate_limit",
            Self::ProviderTimeout => "provider_timeout",
            Self::ProviderUnavailable => "provider_unavailable",
            Self::NetworkUnavailable => "network_unavailable",
            Self::ProviderInvalidResponse => "provider_invalid_response",
            Self::ProviderOutputPolicyViolation => "provider_output_policy_violation",
            Self::StorageReadFailed => "storage_read_failed",
            Self::StorageWriteFailed => "storage_write_failed",
            Self::StorageIntegrityFailed => "storage_integrity_failed",
            Self::MigrationFailed => "migration_failed",
            Self::DatabaseVersionUnsupported => "database_version_unsupported",
            Self::PathDenied => "path_denied",
            Self::PathOutsideRoot => "path_outside_root",
            Self::UnsafeReparsePoint => "unsafe_reparse_point",
            Self::DestructiveConfirmationMissing => "destructive_confirmation_missing",
            Self::DestructiveConfirmationMismatch => "destructive_confirmation_mismatch",
            Self::UnexpectedInternal => "unexpected_internal",
        }
    }

    /// Unique public mapping required by API Contract section 5.1.
    #[must_use]
    pub const fn error_id(self) -> ErrorId {
        match self {
            Self::RequestInvalid
            | Self::UnknownField
            | Self::InvalidPatch
            | Self::InvalidCandidate => ErrorId::RequestInvalid,
            Self::ProtocolUnsupported => ErrorId::ProtocolUnsupported,
            Self::RevisionConflict | Self::IdempotencyPayloadConflict | Self::PreviewTokenStale => {
                ErrorId::Conflict
            }
            Self::EntityNotFound | Self::OperationNotFound => ErrorId::NotFound,
            Self::ResourceBusy | Self::OperationAlreadyRunning => ErrorId::ResourceBusy,
            Self::OperationCancelled => ErrorId::OperationCancelled,
            Self::SecretMissing => ErrorId::SecretMissing,
            Self::SecretValidationRejected => ErrorId::SecretRejected,
            Self::CapabilityAbsent => ErrorId::CapabilityUnsupported,
            Self::SourceUnavailable => ErrorId::SourceUnavailable,
            Self::SessionIdentityChanged | Self::StateRevisionChanged | Self::UserMediaOverride => {
                ErrorId::MediaSessionChanged
            }
            Self::MediaUnreadable | Self::MediaMetadataInvalid => ErrorId::MediaInvalid,
            Self::ProviderAuthentication => ErrorId::ProviderAuthentication,
            Self::ProviderRateLimit => ErrorId::ProviderRateLimited,
            Self::ProviderTimeout => ErrorId::ProviderTimedOut,
            Self::ProviderUnavailable | Self::NetworkUnavailable => ErrorId::ProviderUnavailable,
            Self::ProviderInvalidResponse | Self::ProviderOutputPolicyViolation => {
                ErrorId::ProviderInvalidResponse
            }
            Self::StorageReadFailed | Self::StorageWriteFailed | Self::StorageIntegrityFailed => {
                ErrorId::StorageFailed
            }
            Self::MigrationFailed | Self::DatabaseVersionUnsupported => ErrorId::MigrationFailed,
            Self::PathDenied | Self::PathOutsideRoot | Self::UnsafeReparsePoint => {
                ErrorId::PathDenied
            }
            Self::DestructiveConfirmationMissing | Self::DestructiveConfirmationMismatch => {
                ErrorId::ConfirmationMissing
            }
            Self::UnexpectedInternal => ErrorId::UnexpectedInternal,
        }
    }
}

/// Allowlisted request field names that may be returned to the `WebView`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublicField {
    Request,
    ProtocolVersion,
    ClientRequestId,
    ExpectedRevision,
}

impl PublicField {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::ProtocolVersion => "protocolVersion",
            Self::ClientRequestId => "clientRequestId",
            Self::ExpectedRevision => "expectedRevision",
        }
    }
}

/// Allowlisted capability names that may be returned to the `WebView`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityName {
    Play,
    Pause,
    Seek,
    Next,
    Previous,
    SetQueue,
}

impl CapabilityName {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Play => "play",
            Self::Pause => "pause",
            Self::Seek => "seek",
            Self::Next => "next",
            Self::Previous => "previous",
            Self::SetQueue => "setQueue",
        }
    }
}

/// Optional public context. Every string originates from a fixed allowlist or UUID.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApiErrorDetails {
    pub field: Option<String>,
    pub reason: Option<String>,
    pub current_revision: Option<u64>,
    pub capability: Option<String>,
    pub operation_id: Option<String>,
}

/// The only error structure returned from a rejected IPC invocation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApiError {
    pub schema_version: String,
    pub error_id: ErrorId,
    pub safe_message: String,
    pub retryable: bool,
    pub retry_after_ms: Option<u64>,
    pub correlation_id: String,
    pub details: Option<Box<ApiErrorDetails>>,
}

impl ApiError {
    /// Maps an allowlisted internal reason without accepting upstream text.
    #[must_use]
    pub fn from_reason(reason: InternalReason) -> Self {
        let error_id = reason.error_id();
        Self {
            schema_version: IPC_SCHEMA_VERSION.to_owned(),
            error_id,
            safe_message: error_id.safe_message().to_owned(),
            retryable: error_id.retryable(),
            retry_after_ms: None,
            correlation_id: Uuid::now_v7().to_string(),
            details: Some(Box::new(ApiErrorDetails {
                field: None,
                reason: Some(reason.as_str().to_owned()),
                current_revision: None,
                capability: None,
                operation_id: None,
            })),
        }
    }

    /// Creates the mandatory release fallback for an unclassified failure.
    #[must_use]
    pub fn unexpected() -> Self {
        Self::from_reason(InternalReason::UnexpectedInternal)
    }

    #[must_use]
    pub fn with_field(mut self, field: PublicField) -> Self {
        if let Some(details) = &mut self.details {
            details.field = Some(field.as_str().to_owned());
        }
        self
    }

    #[must_use]
    pub fn with_current_revision(mut self, current_revision: u64) -> Self {
        if let Some(details) = &mut self.details {
            details.current_revision = Some(current_revision);
        }
        self
    }

    #[must_use]
    pub fn with_capability(mut self, capability: CapabilityName) -> Self {
        if let Some(details) = &mut self.details {
            details.capability = Some(capability.as_str().to_owned());
        }
        self
    }

    #[must_use]
    pub fn with_operation_id(mut self, operation_id: Uuid) -> Self {
        if let Some(details) = &mut self.details {
            details.operation_id = Some(operation_id.to_string());
        }
        self
    }

    #[must_use]
    pub fn with_retry_after_ms(mut self, retry_after_ms: u64) -> Self {
        self.retry_after_ms = Some(retry_after_ms);
        self
    }
}

/// A diagnostic placeholder which intentionally retains none of its raw input.
#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub struct RedactedDiagnostic;

impl RedactedDiagnostic {
    pub fn from_untrusted<T: AsRef<str>>(_value: T) -> Self {
        Self
    }
}

impl fmt::Debug for RedactedDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[redacted]")
    }
}

impl fmt::Display for RedactedDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[redacted]")
    }
}
