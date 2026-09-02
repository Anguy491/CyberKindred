# TASK-003 Windows platform probe

| Field | Value |
|---|---|
| Status | Approved |
| Owner | Platform Architecture |
| Last Verified | 2026-09-02 |
| Source of Truth For | `TASK-003` 探针用途、安全边界与运行方法 |
| Related Documents | `MANUAL-TEST.md`, `evidence/TASK-003-MATRIX.md`, `../../docs/testing/TEST-STRATEGY.md` |

Disposable M1 probe for `FR-ONB-003`, `FR-SCH-002`, `FR-SET-003`,
`FR-DAT-005`, `NFR-SEC-001`, and `NFR-COMPAT-001`.

Safety properties:

- Running with no arguments is read-only. It never emits a notification, enables
  autostart, starts audio/a program, or calls a provider.
- The Credential Manager canary uses only `CyberKindred/probe/task-003`. Its
  Rust-owned string copies are zeroized after comparison; the credential is
  deleted and the value is never returned or logged.
- Autostart owns exactly the `HKCU` Run value `CyberKindred.Task003Probe`.
  Enable/disable require an explicit confirmation flag. The stored command always
  runs `preflight --silent`.
- Notification simulation records only `start`, `snooze`, or `ignore`; even
  `start` does not start a program, audio, or a paid request.
- Reset deletes the exact canary credential, exact HKCU value, and exact
  `notification-actions.jsonl`. It does not recurse and never targets music or
  user-selected exports.

## Build and hermetic checks

```powershell
cargo fmt --manifest-path spikes/windows/Cargo.toml --all -- --check
cargo clippy --locked --manifest-path spikes/windows/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --locked --manifest-path spikes/windows/Cargo.toml --all-features
```

## Commands

```powershell
cargo run --locked --manifest-path spikes/windows/Cargo.toml -- preflight
cargo run --locked --manifest-path spikes/windows/Cargo.toml -- credential-cycle --confirm-probe-write
cargo run --locked --manifest-path spikes/windows/Cargo.toml -- autostart status
cargo run --locked --manifest-path spikes/windows/Cargo.toml -- autostart enable --confirm-user-scope
cargo run --locked --manifest-path spikes/windows/Cargo.toml -- autostart disable --confirm-user-scope
cargo run --locked --manifest-path spikes/windows/Cargo.toml -- simulate-notification-action start --confirm-local-only
cargo run --locked --manifest-path spikes/windows/Cargo.toml -- reset --confirm-probe-only
```

Do not run autostart enable or a real notification as part of default automation.
Follow [MANUAL-TEST.md](MANUAL-TEST.md) on an isolated standard-user VM.

## Interpretation

`Passed` means the read-only environment check found the facility. `NotDetected`
is not silently upgraded to success. `NotRunManual` means the probe deliberately
did not perform a visible/system lifecycle action. In particular, a detected
WebView2 runtime does not prove a packaged Tauri build, tray action, notification
activation, NSIS installation, or clean-VM compatibility.
