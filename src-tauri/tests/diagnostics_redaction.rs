// Covers NFR-SEC-004 and NFR-MAINT-003.
#[path = "../src/diagnostics/mod.rs"]
mod diagnostics;

use diagnostics::{DiagnosticEvent, DiagnosticLog, RedactedDiagnostic, ValidatedLogDirectory};
use serde_json::Value;
use std::collections::BTreeSet;
use std::error::Error;
use std::fs;
use uuid::Uuid;

const CORRELATION_ID: &str = "0198d34e-4a74-7000-8000-000000000001";

#[test]
fn redacted_record_has_only_allowlisted_fields_and_fixed_detail() -> Result<(), Box<dyn Error>> {
    let sensitive_canaries = [
        "sk-proj-CANARY-SECRET-DO-NOT-LOG",
        r"C:\Users\Alice\Music\private-track.flac",
        "CANARY private chat body",
        "CANARY provider response body",
    ];
    let event = DiagnosticEvent::OperationFailed {
        correlation_id: Uuid::parse_str(CORRELATION_ID)?,
        occurred_at_ms: 1_788_278_400_000,
        duration_ms: 320,
        attempt: 1,
    };

    let bytes = RedactedDiagnostic::from_event(event).to_json_line()?;
    let line = String::from_utf8(bytes)?;
    assert!(line.ends_with('\n'));
    for canary in sensitive_canaries {
        assert!(!line.contains(canary));
    }

    let value: Value = serde_json::from_str(line.trim_end())?;
    let object = value.as_object().ok_or("record must be a JSON object")?;
    let keys = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = [
        "attempt",
        "bytesRemoved",
        "code",
        "correlationId",
        "detail",
        "durationMs",
        "itemCount",
        "level",
        "occurredAtMs",
        "reason",
        "schemaVersion",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    assert_eq!(keys, expected);
    assert_eq!(
        object.get("detail"),
        Some(&Value::String("[redacted]".into()))
    );
    assert_eq!(
        object.get("correlationId"),
        Some(&Value::String(CORRELATION_ID.into()))
    );
    assert_eq!(
        object.get("code"),
        Some(&Value::String("operation_outcome".into()))
    );
    assert_eq!(object.get("level"), Some(&Value::String("error".into())));
    assert_eq!(
        object.get("reason"),
        Some(&Value::String("operation_failed".into()))
    );
    assert!(object.get("durationMs").is_some_and(Value::is_number));
    assert!(object.get("attempt").is_some_and(Value::is_number));
    Ok(())
}

#[test]
fn writer_emits_utf8_jsonl_for_each_allowlisted_event() -> Result<(), Box<dyn Error>> {
    let temporary = tempfile::tempdir()?;
    let directory = ValidatedLogDirectory::new(temporary.path().to_path_buf())?;
    let (mut log, report) = DiagnosticLog::open(directory, 1_788_278_400_000)?;
    assert_eq!(report.removed_files, 0);
    assert_eq!(report.removed_bytes, 0);

    let correlation_id = Uuid::parse_str(CORRELATION_ID)?;
    log.write(DiagnosticEvent::ApplicationStarted {
        correlation_id,
        occurred_at_ms: 1_788_278_400_000,
    })?;
    log.write(DiagnosticEvent::MaintenanceCompleted {
        correlation_id,
        occurred_at_ms: 1_788_278_400_001,
        item_count: 2,
        bytes_removed: 128,
    })?;

    let output = fs::read_to_string(temporary.path().join(diagnostics::ACTIVE_FILE_NAME))?;
    let lines = output.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 2);
    for line in lines {
        let value: Value = serde_json::from_str(line)?;
        assert_eq!(
            value.get("detail"),
            Some(&Value::String("[redacted]".into()))
        );
    }
    Ok(())
}
