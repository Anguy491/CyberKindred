//! M4 understanding, conversation, feedback, memory and YOU-page services.

pub mod commands;
mod dto;
mod events;
mod provider;
mod service;

pub use dto::*;
pub(crate) use events::ChatEventSink;
pub use events::TauriChatEventSink;
pub(crate) use provider::{
    ChatFuture, ChatProvider, ChatProviderError, ChatProviderOutput, ProposedMemory,
};
#[cfg(test)]
pub(crate) use service::DEGRADED_CHAT_MESSAGE;
pub(crate) use service::UnderstandingService;

#[cfg(test)]
mod tests;
