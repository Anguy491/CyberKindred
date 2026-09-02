use crate::storage::SecretValue;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretKind {
    OpenaiApiKey,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderTestKind {
    Llm,
    Tts,
    Metadata,
    Weather,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Integration {
    Openai,
    AppleMusic,
    Musicbrainz,
    Weather,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationState {
    Connected,
    Degraded,
    Disabled,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NarrationDensity {
    Quiet,
    Balanced,
    Frequent,
}

impl NarrationDensity {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Quiet => "quiet",
            Self::Balanced => "balanced",
            Self::Frequent => "frequent",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioOutputBehavior {
    FollowSystemDefault,
    FixedDevice,
}

impl AudioOutputBehavior {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FollowSystemDefault => "follow_system_default",
            Self::FixedDevice => "fixed_device",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WeatherLocationAction {
    Clear,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceProvider {
    Tts,
}

/// API-004 request. The candidate value has no `Debug`, `Clone`, or `Serialize` path.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidateSecretRequest {
    pub client_request_id: Uuid,
    pub kind: SecretKind,
    pub origin: String,
    pub value: SecretValue,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidateSecretResponse {
    pub request_id: Uuid,
    pub configured: bool,
    pub verified_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteSecretRequest {
    pub client_request_id: Uuid,
    pub kind: SecretKind,
    pub origin: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteSecretResponse {
    pub request_id: Uuid,
    pub configured: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TestProviderRequest {
    pub client_request_id: Uuid,
    pub kind: ProviderTestKind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TestProviderResponse {
    pub request_id: Uuid,
    pub ok: bool,
    pub latency_ms: u64,
    pub safe_message: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WeatherLocation {
    pub city: String,
    pub region: Option<String>,
    pub country: String,
    pub country_code: String,
    pub latitude: f64,
    pub longitude: f64,
    pub timezone: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OriginSecretStatus {
    pub origin: String,
    pub openai_api_key_configured: bool,
    pub last_verified_at: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecretStatus {
    pub origins: Vec<OriginSecretStatus>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntegrationStatus {
    pub integration: Integration,
    pub state: IntegrationState,
    pub last_success_at: Option<String>,
    pub safe_message: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)]
pub struct SettingsView {
    pub provider_origin: String,
    pub llm_model_id: String,
    pub tts_model_id: String,
    pub tts_voice_id: String,
    pub metadata_enabled: bool,
    pub weather_enabled: bool,
    pub default_source_id: Option<String>,
    pub narration_density: NarrationDensity,
    pub tts_enabled: bool,
    pub audio_output_device_id: Option<String>,
    pub audio_output_behavior: AudioOutputBehavior,
    pub minimize_to_tray: bool,
    pub launch_at_startup: bool,
    pub notifications_enabled: bool,
    pub weather_location: Option<WeatherLocation>,
    pub secret_status: SecretStatus,
    pub integration_statuses: Vec<IntegrationStatus>,
    pub revision: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum PatchField<T> {
    #[default]
    Missing,
    Value(T),
}

impl<T> PatchField<T> {
    #[must_use]
    pub const fn is_missing(&self) -> bool {
        matches!(self, Self::Missing)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for PatchField<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        T::deserialize(deserializer).map(Self::Value)
    }
}

impl<T: Serialize> Serialize for PatchField<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Missing => serializer.serialize_unit(),
            Self::Value(value) => value.serialize(serializer),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsPatch {
    #[serde(default, skip_serializing_if = "PatchField::is_missing")]
    pub provider_origin: PatchField<String>,
    #[serde(default, skip_serializing_if = "PatchField::is_missing")]
    pub llm_model_id: PatchField<String>,
    #[serde(default, skip_serializing_if = "PatchField::is_missing")]
    pub tts_model_id: PatchField<String>,
    #[serde(default, skip_serializing_if = "PatchField::is_missing")]
    pub tts_voice_id: PatchField<String>,
    #[serde(default, skip_serializing_if = "PatchField::is_missing")]
    pub metadata_enabled: PatchField<bool>,
    #[serde(default, skip_serializing_if = "PatchField::is_missing")]
    pub weather_enabled: PatchField<bool>,
    #[serde(default, skip_serializing_if = "PatchField::is_missing")]
    pub default_source_id: PatchField<Option<String>>,
    #[serde(default, skip_serializing_if = "PatchField::is_missing")]
    pub narration_density: PatchField<NarrationDensity>,
    #[serde(default, skip_serializing_if = "PatchField::is_missing")]
    pub tts_enabled: PatchField<bool>,
    #[serde(default, skip_serializing_if = "PatchField::is_missing")]
    pub audio_output_device_id: PatchField<Option<String>>,
    #[serde(default, skip_serializing_if = "PatchField::is_missing")]
    pub audio_output_behavior: PatchField<AudioOutputBehavior>,
    #[serde(default, skip_serializing_if = "PatchField::is_missing")]
    pub minimize_to_tray: PatchField<bool>,
    #[serde(default, skip_serializing_if = "PatchField::is_missing")]
    pub launch_at_startup: PatchField<bool>,
    #[serde(default, skip_serializing_if = "PatchField::is_missing")]
    pub notifications_enabled: PatchField<bool>,
    #[serde(default, skip_serializing_if = "PatchField::is_missing")]
    pub weather_location_action: PatchField<WeatherLocationAction>,
}

impl SettingsPatch {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.provider_origin.is_missing()
            && self.llm_model_id.is_missing()
            && self.tts_model_id.is_missing()
            && self.tts_voice_id.is_missing()
            && self.metadata_enabled.is_missing()
            && self.weather_enabled.is_missing()
            && self.default_source_id.is_missing()
            && self.narration_density.is_missing()
            && self.tts_enabled.is_missing()
            && self.audio_output_device_id.is_missing()
            && self.audio_output_behavior.is_missing()
            && self.minimize_to_tray.is_missing()
            && self.launch_at_startup.is_missing()
            && self.notifications_enabled.is_missing()
            && self.weather_location_action.is_missing()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateSettingsRequest {
    pub client_request_id: Uuid,
    pub expected_revision: u64,
    pub patch: SettingsPatch,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Ack {
    pub request_id: Uuid,
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListVoicesRequest {
    pub provider: VoiceProvider,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VoiceView {
    pub voice_id: String,
    pub display_name: String,
    pub preview_available: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VoicesResponse {
    pub voices: Vec<VoiceView>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewVoiceRequest {
    pub client_request_id: Uuid,
    pub voice_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationAccepted {
    pub operation_id: Uuid,
    pub accepted_at: String,
}
