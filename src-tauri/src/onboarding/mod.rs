//! Strict onboarding DTOs, commands, and application service.

pub mod commands;
mod dto;
mod service;

pub use dto::{
    Ack, CityScheduleMode, CompanionStyle, MusicSourceKind, NarrationDensity, OnboardingAiMode,
    OnboardingProfile, OnboardingState, OnboardingStep, OnboardingStepSubmission,
    OnboardingVoiceMode, PrivacyConfirmations, SaveOnboardingStepRequest,
};
pub use service::OnboardingService;

#[cfg(test)]
mod tests;
