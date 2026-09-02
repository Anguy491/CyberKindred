//! Bounded, redacted diagnostic logging for the trusted Rust core.
//!
//! The public write surface intentionally accepts only enumerated events,
//! UUID correlation identifiers, and numeric measurements. It has no field for
//! free-form messages, paths, provider bodies, chat text, or secrets.

use serde::Serialize;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub const ACTIVE_FILE_NAME: &str = "cyberkindred-diagnostics.jsonl";
pub const ROTATED_FILE_PREFIX: &str = "cyberkindred-diagnostics.";
pub const ROTATED_FILE_SUFFIX: &str = ".jsonl";
pub const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: u64 = 50 * 1024 * 1024;
pub const RETENTION_MS: i64 = 14 * 24 * 60 * 60 * 1000;
const REDACTED: &str = "[redacted]";

/// A stable diagnostic logging error that never contains an OS error or path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticError {
    InvalidDirectory,
    UnsafeDirectory,
    UnsafeEntry,
    RecordTooLarge,
    SerializeFailed,
    OpenFailed,
    WriteFailed,
    MaintenanceFailed,
}

impl fmt::Display for DiagnosticError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidDirectory => "diagnostic directory is invalid",
            Self::UnsafeDirectory => "diagnostic directory is unsafe",
            Self::UnsafeEntry => "diagnostic log entry is unsafe",
            Self::RecordTooLarge => "diagnostic record exceeds the size limit",
            Self::SerializeFailed => "diagnostic record serialization failed",
            Self::OpenFailed => "diagnostic log open failed",
            Self::WriteFailed => "diagnostic log write failed",
            Self::MaintenanceFailed => "diagnostic log maintenance failed",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for DiagnosticError {}

/// A log directory that has been verified by the Rust side.
///
/// Validation rejects relative paths, missing/non-directory paths, and any
/// symlink or Windows reparse point in the existing ancestor chain.
#[derive(Clone)]
pub struct ValidatedLogDirectory {
    path: PathBuf,
}

impl ValidatedLogDirectory {
    /// Validates an already-created log directory.
    ///
    /// # Errors
    ///
    /// Returns a stable error when the directory is absent, relative, not a
    /// directory, or contains a symlink/reparse point in its ancestor chain.
    pub fn new(path: PathBuf) -> Result<Self, DiagnosticError> {
        if !path.is_absolute() {
            return Err(DiagnosticError::InvalidDirectory);
        }

        validate_path_chain(&path)?;
        let metadata =
            fs::symlink_metadata(&path).map_err(|_error| DiagnosticError::InvalidDirectory)?;
        if !metadata.is_dir() {
            return Err(DiagnosticError::InvalidDirectory);
        }

        Ok(Self { path })
    }

    fn join(&self, file_name: &str) -> PathBuf {
        self.path.join(file_name)
    }
}

/// The only accepted diagnostic inputs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticEvent {
    ApplicationStarted {
        correlation_id: Uuid,
        occurred_at_ms: i64,
    },
    OperationFailed {
        correlation_id: Uuid,
        occurred_at_ms: i64,
        duration_ms: u64,
        attempt: u8,
    },
    OutboxDeliveryExpired {
        correlation_id: Uuid,
        occurred_at_ms: i64,
        item_count: u64,
    },
    MaintenanceCompleted {
        correlation_id: Uuid,
        occurred_at_ms: i64,
        item_count: u64,
        bytes_removed: u64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum DiagnosticCode {
    ApplicationLifecycle,
    OperationOutcome,
    DiagnosticMaintenance,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum DiagnosticLevel {
    Info,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum DiagnosticReason {
    FoundationStarted,
    OperationFailed,
    OutboxDeliveryExpired,
    RetentionApplied,
}

/// A fully redacted, allowlisted JSONL record.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactedDiagnostic {
    schema_version: &'static str,
    code: DiagnosticCode,
    level: DiagnosticLevel,
    reason: DiagnosticReason,
    correlation_id: Uuid,
    occurred_at_ms: i64,
    duration_ms: Option<u64>,
    attempt: Option<u8>,
    item_count: Option<u64>,
    bytes_removed: Option<u64>,
    detail: &'static str,
}

impl RedactedDiagnostic {
    #[must_use]
    pub fn from_event(event: DiagnosticEvent) -> Self {
        match event {
            DiagnosticEvent::ApplicationStarted {
                correlation_id,
                occurred_at_ms,
            } => Self {
                schema_version: "1.0.0",
                code: DiagnosticCode::ApplicationLifecycle,
                level: DiagnosticLevel::Info,
                reason: DiagnosticReason::FoundationStarted,
                correlation_id,
                occurred_at_ms,
                duration_ms: None,
                attempt: None,
                item_count: None,
                bytes_removed: None,
                detail: REDACTED,
            },
            DiagnosticEvent::OperationFailed {
                correlation_id,
                occurred_at_ms,
                duration_ms,
                attempt,
            } => Self {
                schema_version: "1.0.0",
                code: DiagnosticCode::OperationOutcome,
                level: DiagnosticLevel::Error,
                reason: DiagnosticReason::OperationFailed,
                correlation_id,
                occurred_at_ms,
                duration_ms: Some(duration_ms),
                attempt: Some(attempt),
                item_count: None,
                bytes_removed: None,
                detail: REDACTED,
            },
            DiagnosticEvent::OutboxDeliveryExpired {
                correlation_id,
                occurred_at_ms,
                item_count,
            } => Self {
                schema_version: "1.0.0",
                code: DiagnosticCode::OperationOutcome,
                level: DiagnosticLevel::Error,
                reason: DiagnosticReason::OutboxDeliveryExpired,
                correlation_id,
                occurred_at_ms,
                duration_ms: None,
                attempt: None,
                item_count: Some(item_count),
                bytes_removed: None,
                detail: REDACTED,
            },
            DiagnosticEvent::MaintenanceCompleted {
                correlation_id,
                occurred_at_ms,
                item_count,
                bytes_removed,
            } => Self {
                schema_version: "1.0.0",
                code: DiagnosticCode::DiagnosticMaintenance,
                level: DiagnosticLevel::Info,
                reason: DiagnosticReason::RetentionApplied,
                correlation_id,
                occurred_at_ms,
                duration_ms: None,
                attempt: None,
                item_count: Some(item_count),
                bytes_removed: Some(bytes_removed),
                detail: REDACTED,
            },
        }
    }

    /// Serializes exactly one UTF-8 JSONL record.
    ///
    /// # Errors
    ///
    /// Returns a stable serialization or size-limit error.
    pub fn to_json_line(&self) -> Result<Vec<u8>, DiagnosticError> {
        let mut line =
            serde_json::to_vec(self).map_err(|_error| DiagnosticError::SerializeFailed)?;
        line.push(b'\n');
        let line_length =
            u64::try_from(line.len()).map_err(|_error| DiagnosticError::RecordTooLarge)?;
        if line_length > MAX_FILE_BYTES {
            return Err(DiagnosticError::RecordTooLarge);
        }
        Ok(line)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MaintenanceReport {
    pub removed_files: u64,
    pub removed_bytes: u64,
}

/// A synchronous, single-owner diagnostic JSONL writer.
pub struct DiagnosticLog {
    directory: ValidatedLogDirectory,
    rotation_sequence: u64,
}

impl DiagnosticLog {
    /// Opens the fixed diagnostic log and performs bounded startup maintenance.
    ///
    /// # Errors
    ///
    /// Returns a stable error if a managed entry is unsafe or maintenance fails.
    pub fn open(
        directory: ValidatedLogDirectory,
        now_ms: i64,
    ) -> Result<(Self, MaintenanceReport), DiagnosticError> {
        let mut log = Self {
            directory,
            rotation_sequence: 0,
        };
        log.validate_active_entry()?;
        let report = log.run_startup_maintenance(now_ms)?;
        Ok((log, report))
    }

    /// Appends one allowlisted diagnostic, rotating before the size bound.
    ///
    /// # Errors
    ///
    /// Returns a stable error if serialization, validation, rotation, or write
    /// fails. No underlying path or OS error is retained.
    pub fn write(&mut self, event: DiagnosticEvent) -> Result<(), DiagnosticError> {
        let record = RedactedDiagnostic::from_event(event).to_json_line()?;
        self.validate_active_entry()?;

        let active_path = self.directory.join(ACTIVE_FILE_NAME);
        let current_length = entry_length(&active_path)?;
        let record_length =
            u64::try_from(record.len()).map_err(|_error| DiagnosticError::RecordTooLarge)?;
        if current_length.saturating_add(record_length) > MAX_FILE_BYTES {
            self.rotate_active(event_time_ms(event))?;
        }

        let mut file = open_regular_append(&active_path)?;
        file.write_all(&record)
            .map_err(|_error| DiagnosticError::WriteFailed)?;
        file.flush().map_err(|_error| DiagnosticError::WriteFailed)
    }

    fn validate_active_entry(&self) -> Result<(), DiagnosticError> {
        validate_managed_file_if_present(&self.directory.join(ACTIVE_FILE_NAME))
    }

    fn rotate_active(&mut self, occurred_at_ms: i64) -> Result<(), DiagnosticError> {
        let active_path = self.directory.join(ACTIVE_FILE_NAME);
        if entry_length(&active_path)? == 0 {
            return Ok(());
        }

        loop {
            let file_name = rotated_file_name(occurred_at_ms, self.rotation_sequence);
            self.rotation_sequence = self.rotation_sequence.saturating_add(1);
            let rotated_path = self.directory.join(&file_name);
            if rotated_path.exists() {
                validate_managed_file_if_present(&rotated_path)?;
                continue;
            }
            fs::rename(&active_path, &rotated_path)
                .map_err(|_error| DiagnosticError::MaintenanceFailed)?;
            return Ok(());
        }
    }

    fn run_startup_maintenance(
        &mut self,
        now_ms: i64,
    ) -> Result<MaintenanceReport, DiagnosticError> {
        let active_path = self.directory.join(ACTIVE_FILE_NAME);
        if entry_length(&active_path)? > MAX_FILE_BYTES {
            self.rotate_active(now_ms)?;
        }

        let mut report = MaintenanceReport::default();
        let mut managed = collect_managed_entries(&self.directory)?;
        let cutoff = now_ms.saturating_sub(RETENTION_MS);

        for entry in managed.iter().filter(|entry| entry.timestamp_ms < cutoff) {
            remove_managed_file(entry, &mut report)?;
        }

        managed = collect_managed_entries(&self.directory)?;
        managed.sort_by_key(|entry| entry.timestamp_ms);
        let active_length = entry_length(&active_path)?;
        let mut total = managed
            .iter()
            .fold(active_length, |sum, entry| sum.saturating_add(entry.length));
        for entry in &managed {
            if total <= MAX_TOTAL_BYTES {
                break;
            }
            remove_managed_file(entry, &mut report)?;
            total = total.saturating_sub(entry.length);
        }

        Ok(report)
    }
}

#[derive(Debug)]
struct ManagedEntry {
    path: PathBuf,
    timestamp_ms: i64,
    length: u64,
}

fn event_time_ms(event: DiagnosticEvent) -> i64 {
    match event {
        DiagnosticEvent::ApplicationStarted { occurred_at_ms, .. }
        | DiagnosticEvent::OperationFailed { occurred_at_ms, .. }
        | DiagnosticEvent::OutboxDeliveryExpired { occurred_at_ms, .. }
        | DiagnosticEvent::MaintenanceCompleted { occurred_at_ms, .. } => occurred_at_ms,
    }
}

fn rotated_file_name(timestamp_ms: i64, sequence: u64) -> String {
    let safe_timestamp = timestamp_ms.max(0);
    format!("{ROTATED_FILE_PREFIX}{safe_timestamp}.{sequence}{ROTATED_FILE_SUFFIX}")
}

fn collect_managed_entries(
    directory: &ValidatedLogDirectory,
) -> Result<Vec<ManagedEntry>, DiagnosticError> {
    let entries =
        fs::read_dir(&directory.path).map_err(|_error| DiagnosticError::MaintenanceFailed)?;
    let mut managed = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_error| DiagnosticError::MaintenanceFailed)?;
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        let Some(timestamp_ms) = parse_rotated_timestamp(file_name) else {
            continue;
        };
        let path = entry.path();
        let metadata = safe_regular_metadata(&path)?;
        managed.push(ManagedEntry {
            path,
            timestamp_ms,
            length: metadata.len(),
        });
    }
    Ok(managed)
}

fn parse_rotated_timestamp(file_name: &str) -> Option<i64> {
    let body = file_name
        .strip_prefix(ROTATED_FILE_PREFIX)?
        .strip_suffix(ROTATED_FILE_SUFFIX)?;
    let (timestamp, sequence) = body.split_once('.')?;
    if timestamp.is_empty() || sequence.is_empty() || sequence.contains('.') {
        return None;
    }
    sequence.parse::<u64>().ok()?;
    timestamp.parse::<i64>().ok()
}

fn remove_managed_file(
    entry: &ManagedEntry,
    report: &mut MaintenanceReport,
) -> Result<(), DiagnosticError> {
    safe_regular_metadata(&entry.path)?;
    fs::remove_file(&entry.path).map_err(|_error| DiagnosticError::MaintenanceFailed)?;
    report.removed_files = report.removed_files.saturating_add(1);
    report.removed_bytes = report.removed_bytes.saturating_add(entry.length);
    Ok(())
}

fn entry_length(path: &Path) -> Result<u64, DiagnosticError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            ensure_regular_and_not_reparse(&metadata)?;
            Ok(metadata.len())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(_error) => Err(DiagnosticError::OpenFailed),
    }
}

fn validate_managed_file_if_present(path: &Path) -> Result<(), DiagnosticError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => ensure_regular_and_not_reparse(&metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_error) => Err(DiagnosticError::OpenFailed),
    }
}

fn safe_regular_metadata(path: &Path) -> Result<fs::Metadata, DiagnosticError> {
    let metadata = fs::symlink_metadata(path).map_err(|_error| DiagnosticError::UnsafeEntry)?;
    ensure_regular_and_not_reparse(&metadata)?;
    Ok(metadata)
}

fn ensure_regular_and_not_reparse(metadata: &fs::Metadata) -> Result<(), DiagnosticError> {
    if !metadata.is_file() || metadata.file_type().is_symlink() || is_reparse_point(metadata) {
        return Err(DiagnosticError::UnsafeEntry);
    }
    Ok(())
}

fn validate_path_chain(path: &Path) -> Result<(), DiagnosticError> {
    for ancestor in path.ancestors() {
        let metadata =
            fs::symlink_metadata(ancestor).map_err(|_error| DiagnosticError::InvalidDirectory)?;
        if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
            return Err(DiagnosticError::UnsafeDirectory);
        }
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

fn open_regular_append(path: &Path) -> Result<File, DiagnosticError> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    configure_no_follow(&mut options);
    let file = options
        .open(path)
        .map_err(|_error| DiagnosticError::OpenFailed)?;
    let metadata = file
        .metadata()
        .map_err(|_error| DiagnosticError::OpenFailed)?;
    ensure_regular_and_not_reparse(&metadata)?;
    Ok(file)
}

#[cfg(windows)]
fn configure_no_follow(options: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
}

#[cfg(not(windows))]
fn configure_no_follow(_options: &mut OpenOptions) {}
