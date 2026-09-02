use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use super::error::{ApiError, IPC_SCHEMA_VERSION, InternalReason, PublicField};

/// Common fields carried by every public event.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventEnvelope {
    pub schema_version: String,
    pub sequence: u64,
    pub occurred_at: String,
}

impl EventEnvelope {
    #[must_use]
    pub fn now(sequence: u64) -> Self {
        Self {
            schema_version: IPC_SCHEMA_VERSION.to_owned(),
            sequence,
            occurred_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        }
    }

    /// Validates an envelope before it reaches event consumers.
    ///
    /// # Errors
    ///
    /// Returns a stable validation/protocol error without echoing the payload.
    pub fn validate(&self) -> Result<(), ApiError> {
        if self.schema_version != IPC_SCHEMA_VERSION {
            return Err(ApiError::from_reason(InternalReason::ProtocolUnsupported)
                .with_field(PublicField::ProtocolVersion));
        }
        if self.sequence == 0 || DateTime::parse_from_rfc3339(&self.occurred_at).is_err() {
            return Err(ApiError::from_reason(InternalReason::RequestInvalid)
                .with_field(PublicField::Request));
        }
        Ok(())
    }
}

/// Process-local event sequence. A new process starts again at one.
#[derive(Debug, Default)]
pub struct ProcessSequence {
    current: AtomicU64,
}

impl ProcessSequence {
    /// Returns the next process-local sequence without wrapping.
    ///
    /// # Errors
    ///
    /// Returns `ERR-1601` after the sequence space is exhausted.
    pub fn next(&self) -> Result<u64, ApiError> {
        self.current
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .map(|previous| previous + 1)
            .map_err(|_| ApiError::unexpected())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SequenceObservation {
    Apply,
    AwaitingSnapshot,
    Gap { expected: u64, received: u64 },
    IgnoreStale,
}

/// Tracks the global process event sequence and blocks deltas during resync.
#[derive(Clone, Copy, Debug, Default)]
pub struct SequenceTracker {
    last_applied: Option<u64>,
    high_water: Option<u64>,
    resync_required: bool,
}

impl SequenceTracker {
    /// A new/reloaded subscriber always starts by reading an authoritative snapshot.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            last_applied: None,
            high_water: None,
            resync_required: true,
        }
    }

    #[must_use]
    pub const fn requires_snapshot(self) -> bool {
        self.resync_required
    }

    /// Marks resume or another reconciliation boundary.
    pub fn require_snapshot(&mut self) {
        self.resync_required = true;
    }

    /// Accepts a sequence hint. Deltas are not applied while a snapshot is pending.
    pub fn observe(&mut self, sequence: u64) -> SequenceObservation {
        if sequence == 0 {
            self.resync_required = true;
            return SequenceObservation::Gap {
                expected: self.last_applied.map_or(1, |value| value.saturating_add(1)),
                received: sequence,
            };
        }

        self.high_water = Some(
            self.high_water
                .map_or(sequence, |value| value.max(sequence)),
        );
        if self.resync_required {
            return SequenceObservation::AwaitingSnapshot;
        }

        match self.last_applied {
            None => {
                self.last_applied = Some(sequence);
                SequenceObservation::Apply
            }
            Some(last) if sequence <= last => SequenceObservation::IgnoreStale,
            Some(last) if sequence == last.saturating_add(1) => {
                self.last_applied = Some(sequence);
                SequenceObservation::Apply
            }
            Some(last) => {
                self.resync_required = true;
                SequenceObservation::Gap {
                    expected: last.saturating_add(1),
                    received: sequence,
                }
            }
        }
    }

    /// Completes a snapshot read at the latest sequence observed during the read.
    pub fn snapshot_applied(&mut self) {
        self.last_applied = self.high_water.or(self.last_applied);
        self.resync_required = false;
    }
}
