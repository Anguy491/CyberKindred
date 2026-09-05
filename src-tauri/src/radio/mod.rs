//! Confirmed-start local program orchestration and public API-024/025 events.

mod apple;
pub mod commands;
mod dto;
mod events;
mod idempotency;
mod playback;
mod runner;
mod service;
mod speech;
mod store;
mod traits;

pub use apple::{
    AppleCompanion, AppleCompanionMonitor, AppleCompanionPolicySource, AppleCompanionSignal,
    AppleReactionDensity, SystemProgramSpeech, UnavailableAppleCompanion,
};
pub use dto::{
    ProgramAck, StartProgramRequest, StartProgramResponse, StartProgramTrigger, StopProgramRequest,
};
pub use events::{
    PROGRAM_SEGMENT_EVENT, PROGRAM_STATE_EVENT, ProgramEventState, ProgramSegmentEvent,
    ProgramSegmentEventState, ProgramStateEvent, RadioClock, RadioEvent, RadioEventSink,
    SystemRadioClock, TauriRadioEventSink,
};
pub use playback::{LocalProgramPlayback, PlaybackEventHub, ProgramTrack};
pub use service::{RadioIdFactory, RadioService, RadioServiceDependencies, SystemRadioIdFactory};
pub use speech::{SpeechActorProgramAdapter, SpeechRuntimeConfig, TextOnlyProgramSpeech};
pub use store::{ProgramRunPhase, ProgramSegmentPhase, RadioProgramStore};
pub use traits::{
    ConfirmedProgramStart, DomainProgramPlanner, ManualProgramStartAuthorizer,
    ProgramPlannerContextSource, ProgramPlayback, ProgramPlaybackSignal, ProgramRadioPlanner,
    ProgramSpeech, ProgramSpeechOutcome, ProgramStartAuthorizer, RadioFuture, RadioPlanningContext,
};

#[cfg(test)]
mod tests;
