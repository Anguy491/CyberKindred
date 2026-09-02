use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OnboardingStep {
    Welcome,
    MusicSource,
    OpenaiKey,
    Voice,
    Profile,
    CitySchedule,
    Privacy,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MusicSourceKind {
    Local,
    AppleMusic,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OnboardingAiMode {
    Verified,
    LocalOnly,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OnboardingVoiceMode {
    Selected,
    TextOnly,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CityScheduleMode {
    Configured,
    NotNow,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompanionStyle {
    QuietWarm,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NarrationDensity {
    Quiet,
    Balanced,
    Frequent,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OnboardingProfile {
    pub display_name: String,
    pub companion_style: CompanionStyle,
    pub initial_preferences: Vec<String>,
    pub narration_density: NarrationDensity,
}

impl Default for OnboardingProfile {
    fn default() -> Self {
        Self {
            display_name: String::new(),
            companion_style: CompanionStyle::QuietWarm,
            initial_preferences: Vec::new(),
            narration_density: NarrationDensity::Balanced,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrivacyConfirmations {
    pub explicit_sound: bool,
    pub raw_conversation_retention: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OnboardingState {
    pub completed: bool,
    pub completed_steps: Vec<OnboardingStep>,
    pub source_selection: Vec<MusicSourceKind>,
    pub ai_mode: Option<OnboardingAiMode>,
    pub voice_mode: Option<OnboardingVoiceMode>,
    pub city_schedule_mode: Option<CityScheduleMode>,
    pub profile: OnboardingProfile,
    pub privacy_confirmations: PrivacyConfirmations,
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "step", rename_all = "snake_case", deny_unknown_fields)]
pub enum OnboardingStepSubmission {
    Welcome {},
    MusicSource { sources: Vec<MusicSourceKind> },
    OpenaiKey { mode: OnboardingAiMode },
    Voice { mode: OnboardingVoiceMode },
    Profile { profile: OnboardingProfile },
    CitySchedule { mode: CityScheduleMode },
    Privacy { confirmations: PrivacyConfirmations },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveOnboardingStepRequest {
    pub client_request_id: Uuid,
    pub expected_revision: u64,
    pub submission: OnboardingStepSubmission,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Ack {
    pub request_id: Uuid,
    pub revision: u64,
}
