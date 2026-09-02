use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::error::{ApiError, IPC_SCHEMA_VERSION, InternalReason, PublicField};

/// API-001 accepts exactly an empty object.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EmptyRequest {}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Windows,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Llm,
    Tts,
    Metadata,
    Weather,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Local,
    SystemSession,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)]
pub struct SourceCapabilities {
    pub play: bool,
    pub pause: bool,
    pub seek: bool,
    pub next: bool,
    pub previous: bool,
    pub set_queue: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceSummary {
    pub source_id: String,
    pub kind: SourceKind,
    pub display_name: String,
    pub connected: bool,
    pub capabilities: SourceCapabilities,
}

impl SourceSummary {
    /// Builds a safe source view and enforces the local-only queue capability.
    ///
    /// # Errors
    ///
    /// Returns `ERR-1001` when an identifier/display label is unsafe or a
    /// system-session source advertises `setQueue`.
    pub fn new(
        source_id: impl Into<String>,
        kind: SourceKind,
        display_name: impl Into<String>,
        connected: bool,
        capabilities: SourceCapabilities,
    ) -> Result<Self, ApiError> {
        let source_id = source_id.into();
        let display_name = display_name.into();
        if !is_safe_id(&source_id)
            || !is_safe_label(&display_name)
            || (kind == SourceKind::SystemSession && capabilities.set_queue)
        {
            return Err(ApiError::from_reason(InternalReason::RequestInvalid)
                .with_field(PublicField::Request));
        }
        Ok(Self {
            source_id,
            kind,
            display_name,
            connected,
            capabilities,
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)]
pub struct AppFeatures {
    pub local_library: bool,
    pub system_media_session: bool,
    pub music_kit: bool,
    pub apple_music_dom_control: bool,
    pub external_http_api: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppCapabilities {
    pub protocol_version: String,
    pub app_version: String,
    pub platform: Platform,
    pub os_version: String,
    pub features: AppFeatures,
    pub sources: Vec<SourceSummary>,
    pub providers: Vec<ProviderKind>,
}

/// Read-only API-001 service. Construction validates all adapter-provided views.
#[derive(Clone, Debug)]
pub struct CapabilitiesService {
    snapshot: AppCapabilities,
}

impl CapabilitiesService {
    /// Creates a capabilities snapshot from trusted, dependency-injected probes.
    ///
    /// # Errors
    ///
    /// Returns `ERR-1001` for unsafe labels, duplicate IDs/providers, a system
    /// source without the advertised feature, or forbidden queue capability.
    pub fn new(
        app_version: impl Into<String>,
        os_version: impl Into<String>,
        system_media_session: bool,
        sources: Vec<SourceSummary>,
        providers: Vec<ProviderKind>,
    ) -> Result<Self, ApiError> {
        let app_version = app_version.into();
        let os_version = os_version.into();
        if !is_safe_version(&app_version) || !is_safe_label(&os_version) {
            return Err(ApiError::from_reason(InternalReason::RequestInvalid)
                .with_field(PublicField::Request));
        }

        let mut source_ids = HashSet::with_capacity(sources.len());
        for source in &sources {
            if !source_ids.insert(source.source_id.as_str())
                || (source.kind == SourceKind::SystemSession && !system_media_session)
                || (source.kind == SourceKind::SystemSession && source.capabilities.set_queue)
            {
                return Err(ApiError::from_reason(InternalReason::RequestInvalid)
                    .with_field(PublicField::Request));
            }
        }
        let unique_providers = providers.iter().copied().collect::<HashSet<_>>();
        if unique_providers.len() != providers.len() {
            return Err(ApiError::from_reason(InternalReason::RequestInvalid)
                .with_field(PublicField::Request));
        }

        Ok(Self {
            snapshot: AppCapabilities {
                protocol_version: IPC_SCHEMA_VERSION.to_owned(),
                app_version,
                platform: Platform::Windows,
                os_version,
                features: AppFeatures {
                    local_library: true,
                    system_media_session,
                    music_kit: false,
                    apple_music_dom_control: false,
                    external_http_api: false,
                },
                sources,
                providers,
            },
        })
    }

    /// Creates the no-adapter M2 foundation snapshot.
    ///
    /// # Errors
    ///
    /// Returns `ERR-1001` if a version label is unsafe.
    pub fn foundation(
        app_version: impl Into<String>,
        os_version: impl Into<String>,
    ) -> Result<Self, ApiError> {
        Self::new(app_version, os_version, false, Vec::new(), Vec::new())
    }

    /// Implements API-001 without file, database, provider, or network access.
    #[must_use]
    pub fn get_capabilities(&self, _request: EmptyRequest) -> AppCapabilities {
        self.snapshot.clone()
    }
}

fn is_safe_id(value: &str) -> bool {
    (1..=100).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn is_safe_version(value: &str) -> bool {
    (1..=100).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'+'))
}

fn is_safe_label(value: &str) -> bool {
    let count = value.chars().count();
    (1..=200).contains(&count)
        && value
            .chars()
            .all(|character| !character.is_control() && !matches!(character, '\\' | '/'))
}
