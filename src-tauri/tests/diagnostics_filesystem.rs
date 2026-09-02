// Covers NFR-SEC-003, NFR-SEC-004, and NFR-MAINT-003.
#[path = "../src/diagnostics/mod.rs"]
mod diagnostics;

use diagnostics::{
    ACTIVE_FILE_NAME, DiagnosticError, DiagnosticEvent, DiagnosticLog, MAX_FILE_BYTES,
    MAX_TOTAL_BYTES, RETENTION_MS, ROTATED_FILE_PREFIX, ROTATED_FILE_SUFFIX, ValidatedLogDirectory,
};
use std::error::Error;
use std::fs::{self, File};
use std::path::Path;
#[cfg(windows)]
use std::process::Command;
use uuid::Uuid;

const NOW_MS: i64 = 1_788_278_400_000;

#[test]
fn rejects_relative_missing_and_non_directory_inputs_without_path_disclosure()
-> Result<(), Box<dyn Error>> {
    let relative = ValidatedLogDirectory::new("private/logs".into());
    assert!(matches!(relative, Err(DiagnosticError::InvalidDirectory)));

    let temporary = tempfile::tempdir()?;
    let missing_path = temporary.path().join("CANARY-secret-missing-directory");
    let missing = ValidatedLogDirectory::new(missing_path);
    let Err(missing_error) = missing else {
        return Err("missing directory unexpectedly accepted".into());
    };
    assert_eq!(missing_error, DiagnosticError::InvalidDirectory);
    assert!(!missing_error.to_string().contains("CANARY"));

    let regular_path = temporary.path().join("not-a-directory");
    File::create(&regular_path)?;
    let regular = ValidatedLogDirectory::new(regular_path);
    assert!(matches!(regular, Err(DiagnosticError::InvalidDirectory)));
    Ok(())
}

#[test]
fn rotates_before_an_append_would_cross_the_single_file_bound() -> Result<(), Box<dyn Error>> {
    let temporary = tempfile::tempdir()?;
    let active_path = temporary.path().join(ACTIVE_FILE_NAME);
    File::create(&active_path)?.set_len(MAX_FILE_BYTES)?;

    let directory = ValidatedLogDirectory::new(temporary.path().to_path_buf())?;
    let (mut log, _) = DiagnosticLog::open(directory, NOW_MS)?;
    log.write(DiagnosticEvent::ApplicationStarted {
        correlation_id: Uuid::nil(),
        occurred_at_ms: NOW_MS,
    })?;
    log.write(DiagnosticEvent::OperationFailed {
        correlation_id: Uuid::nil(),
        occurred_at_ms: NOW_MS + 1,
        duration_ms: 10,
        attempt: 1,
    })?;
    log.write(DiagnosticEvent::MaintenanceCompleted {
        correlation_id: Uuid::nil(),
        occurred_at_ms: NOW_MS + 2,
        item_count: 1,
        bytes_removed: MAX_FILE_BYTES,
    })?;

    let active_length = fs::metadata(&active_path)?.len();
    assert!(active_length > 0);
    assert!(active_length <= MAX_FILE_BYTES);
    let rotated_count = fs::read_dir(temporary.path())?
        .filter_map(Result::ok)
        .filter(|entry| is_rotated_name(&entry.file_name().to_string_lossy()))
        .count();
    assert_eq!(rotated_count, 1);
    Ok(())
}

#[test]
fn startup_retention_and_quota_touch_only_fixed_managed_names() -> Result<(), Box<dyn Error>> {
    let temporary = tempfile::tempdir()?;
    let old_name = rotated_name(NOW_MS - RETENTION_MS - 1, 0);
    let old_path = temporary.path().join(&old_name);
    fs::write(&old_path, b"old")?;

    let unrelated_path = temporary.path().join("user-owned-CANARY.txt");
    fs::write(&unrelated_path, b"preserve")?;
    let misleading_path = temporary
        .path()
        .join("cyberkindred-diagnostics.not-managed.jsonl");
    fs::write(&misleading_path, b"preserve")?;

    for sequence in 0..11_u64 {
        let path = temporary.path().join(rotated_name(
            NOW_MS - 1_000 + i64::try_from(sequence)?,
            sequence + 1,
        ));
        File::create(path)?.set_len(MAX_FILE_BYTES)?;
    }

    let directory = ValidatedLogDirectory::new(temporary.path().to_path_buf())?;
    let (_log, report) = DiagnosticLog::open(directory, NOW_MS)?;
    assert!(!old_path.exists());
    assert!(unrelated_path.exists());
    assert!(misleading_path.exists());
    assert!(report.removed_files >= 2);
    assert!(report.removed_bytes >= MAX_FILE_BYTES);

    let total = managed_total(temporary.path())?;
    assert!(total <= MAX_TOTAL_BYTES);
    Ok(())
}

#[test]
fn managed_symlink_is_rejected_without_following_it() -> Result<(), Box<dyn Error>> {
    let temporary = tempfile::tempdir()?;
    let log_directory = temporary.path().join("logs");
    fs::create_dir(&log_directory)?;
    let outside_path = temporary.path().join("outside-CANARY.txt");
    fs::write(&outside_path, b"outside must survive")?;
    let active_link = log_directory.join(ACTIVE_FILE_NAME);

    if !create_file_symlink(&outside_path, &active_link)? {
        return Ok(());
    }

    let directory = ValidatedLogDirectory::new(log_directory)?;
    let result = DiagnosticLog::open(directory, NOW_MS);
    assert!(matches!(result, Err(DiagnosticError::UnsafeEntry)));
    assert_eq!(fs::read(&outside_path)?, b"outside must survive");
    Ok(())
}

#[test]
fn symlink_directory_is_not_accepted_as_a_validated_root() -> Result<(), Box<dyn Error>> {
    let temporary = tempfile::tempdir()?;
    let target = temporary.path().join("real-logs");
    fs::create_dir(&target)?;
    let link = temporary.path().join("linked-logs");
    if !create_directory_symlink(&target, &link)? {
        return Ok(());
    }

    let result = ValidatedLogDirectory::new(link);
    assert!(matches!(result, Err(DiagnosticError::UnsafeDirectory)));
    Ok(())
}

fn rotated_name(timestamp_ms: i64, sequence: u64) -> String {
    format!("{ROTATED_FILE_PREFIX}{timestamp_ms}.{sequence}{ROTATED_FILE_SUFFIX}")
}

fn is_rotated_name(file_name: &str) -> bool {
    let Some(body) = file_name
        .strip_prefix(ROTATED_FILE_PREFIX)
        .and_then(|value| value.strip_suffix(ROTATED_FILE_SUFFIX))
    else {
        return false;
    };
    let Some((timestamp, sequence)) = body.split_once('.') else {
        return false;
    };
    !sequence.contains('.') && timestamp.parse::<i64>().is_ok() && sequence.parse::<u64>().is_ok()
}

fn managed_total(directory: &Path) -> Result<u64, Box<dyn Error>> {
    let mut total = 0_u64;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == ACTIVE_FILE_NAME || is_rotated_name(&name) {
            total = total.saturating_add(entry.metadata()?.len());
        }
    }
    Ok(total)
}

#[cfg(windows)]
fn create_file_symlink(target: &Path, link: &Path) -> Result<bool, Box<dyn Error>> {
    match std::os::windows::fs::symlink_file(target, link) {
        Ok(()) => Ok(true),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::Unsupported
            ) || error.raw_os_error() == Some(1314) =>
        {
            Ok(false)
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(not(windows))]
fn create_file_symlink(target: &Path, link: &Path) -> Result<bool, Box<dyn Error>> {
    std::os::unix::fs::symlink(target, link)?;
    Ok(true)
}

#[cfg(windows)]
fn create_directory_symlink(target: &Path, link: &Path) -> Result<bool, Box<dyn Error>> {
    match std::os::windows::fs::symlink_dir(target, link) {
        Ok(()) => Ok(true),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::Unsupported
            ) || error.raw_os_error() == Some(1314) =>
        {
            let status = Command::new("cmd")
                .args(["/d", "/c", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .status()?;
            Ok(status.success())
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(not(windows))]
fn create_directory_symlink(target: &Path, link: &Path) -> Result<bool, Box<dyn Error>> {
    std::os::unix::fs::symlink(target, link)?;
    Ok(true)
}
