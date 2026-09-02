//! Bounded, stateless `OpenAI` Responses adapter for local program planning.

mod context;
mod provider;
mod transport;

pub use context::{
    ContextTurn, ContextTurnRole, EmptyProgramContextSource, MAX_PROVIDER_INPUT_TOKENS,
    MAX_PROVIDER_OUTPUT_TOKENS, ProgramContextExtras, ProgramContextSource,
};
pub use provider::{
    OpenAiProgramCredential, OpenAiProgramPlanProvider, ProgramCallContextFactory,
    ProgramCredentialSource, SystemProgramCallContextFactory,
};
pub use transport::{
    ReqwestResponsesTransport, ResponsesFuture, ResponsesHttpRequest, ResponsesHttpResponse,
    ResponsesTransport, ResponsesTransportError,
};

#[cfg(test)]
mod tests;
