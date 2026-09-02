use std::time::{Duration, Instant};

/// Fixed caller deadlines from API Contract v1.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandTimeout {
    Read,
    Mutation,
    Search,
    Delete,
    VoiceOperation,
    ProviderOperation,
}

impl CommandTimeout {
    #[must_use]
    pub const fn duration(self) -> Duration {
        match self {
            Self::Read => Duration::from_secs(2),
            Self::Mutation => Duration::from_secs(5),
            Self::Search => Duration::from_secs(10),
            Self::Delete => Duration::from_secs(30),
            Self::VoiceOperation => Duration::from_secs(45),
            Self::ProviderOperation => Duration::from_secs(60),
        }
    }
}

/// Observable deadline which never implies cancellation of the underlying work.
#[derive(Clone, Copy, Debug)]
pub struct Deadline {
    started_at: Instant,
    budget: Duration,
}

impl Deadline {
    #[must_use]
    pub fn new(started_at: Instant, timeout: CommandTimeout) -> Self {
        Self {
            started_at,
            budget: timeout.duration(),
        }
    }

    #[must_use]
    pub fn is_elapsed(self, now: Instant) -> bool {
        now.saturating_duration_since(self.started_at) >= self.budget
    }

    #[must_use]
    pub fn remaining(self, now: Instant) -> Duration {
        self.budget
            .saturating_sub(now.saturating_duration_since(self.started_at))
    }
}
