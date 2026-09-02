//! Cancellable local-library scanning and strict API-013/API-014 boundaries.

pub mod commands;

mod dto;
mod events;
mod extractor;
mod service;
mod traversal;

pub use dto::{
    CancelLibraryScanRequest, CancelLibraryScanResponse, CancelLibraryScanState, OperationAccepted,
    StartLibraryScanRequest,
};
pub use events::{
    LIBRARY_SCAN_EVENT, ScanEvent, ScanEventSink, ScanEventState, TauriScanEventSink,
};
pub use service::{ScanClock, ScannerService, SystemScanClock};

#[cfg(test)]
mod tests;
