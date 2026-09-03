use std::{future::Future, pin::Pin};

use crate::{providers::ProviderCallContext, storage::ChatContextSnapshot};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChatProviderError {
    Cancelled,
    Unavailable,
    InvalidResponse,
}

#[derive(Clone, PartialEq)]
pub(crate) struct ProposedMemory {
    pub kind: crate::storage::StoredMemoryKind,
    pub content: String,
    pub confidence: f64,
}

#[derive(Clone, PartialEq)]
pub(crate) struct ChatProviderOutput {
    pub text: String,
    pub proposed_memories: Vec<ProposedMemory>,
    pub provider: &'static str,
    pub model: String,
    pub prompt_version: &'static str,
}

pub(crate) type ChatFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ChatProviderOutput, ChatProviderError>> + Send + 'a>>;

pub(crate) trait ChatProvider: Send + Sync {
    fn respond<'a>(
        &'a self,
        context: &'a ChatContextSnapshot,
        call: &'a ProviderCallContext,
    ) -> ChatFuture<'a>;
}
