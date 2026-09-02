# TASK-003 Windows probe evidence

| Field | Value |
|---|---|
| Status | Approved |
| Owner | Platform Architecture / QA Steward |
| Last Verified | 2026-09-02 |
| Source of Truth For | `TASK-003` 当前标准用户 Windows 探针结果与限制 |
| Related Documents | `../README.md`, `../MANUAL-TEST.md`, `../../../docs/testing/TEST-STRATEGY.md` |

## Safety boundary

- 默认 `preflight` 只读；不会发送通知、启用自启动、启动节目/音频或调用 provider。
- 所有写操作都要求精确确认参数。缺少确认的 credential、自启动、通知模拟与 reset 命令均以退出码 1 拒绝，随后状态仍为 autostart disabled。
- Credential canary 仅使用 `CyberKindred/probe/task-003` 的 Windows Password Vault resource；运行时值不进入 stdout/stderr、文件、日志或仓库。
- 自启动只操作 HKCU Run 的 `CyberKindred.Task003Probe` 精确值；reset 只删除该值、该 credential 和精确 action log，不递归删除目录。
- 探针没有 HTTP client、音频依赖、Tauri capability、provider 或远程内容。

## Environment

| Item | Observed value |
|---|---|
| Test date/timezone | 2026-09-02 / Australia/Sydney |
| OS cohort | Windows 11 Home x64, version/build `10.0.26200` |
| Account scope | Standard user; administrator role check returned `false` |
| Rust | `rustc 1.98.0` / MSVC target |
| Node / pnpm | `24.19.0` / `11.16.0` |
| WebView2 | Runtime detected through its registered product id |
| Probe dependencies | `windows 0.62.2`, `serde 1.0.229`, `serde_json 1.0.151`, `zeroize 1.9.0` |
| Candidate commit | Recorded by the M1 milestone checkpoint containing this evidence |

## Capability matrix

| Capability | Status | Evidence (de-identified) | Failure/refusal path | Fallback / target milestone |
|---|---|---|---|---|
| Credential Manager canary cycle | Passed | write/read/match/delete/absent-after-delete all `true`; output secret fields `0` | Hermetic injected read failure performs cleanup; missing delete is idempotent | Local-only mode; M2 `TASK-007` |
| Secret/output canary boundary | Passed | Serialized success report contains only booleans and a fixed resource label; unit scan finds no canary prefix; no probe log or persistent artifact remains | Any secret field or failed absence check returns failure and blocks M2 | Stop checkpoint and repair before M2 |
| Notification action model | Passed | `start`, `snooze`, `ignore` each require explicit confirmation and record `programStarted=false`, `audioStarted=false`, `paidProviderCalls=0` | Missing confirmation exits 1 without writing | Notifications remain disabled; M5 `TASK-024` |
| Real Windows notification activation | Not Run | Visible/user action and packaged activation were not performed | Unavailable transport keeps schedule notification-only and silent | M5/M7 Windows verification |
| Per-user autostart default/enable/status/disable | Passed | Initial HKCU value absent; explicit enable produced only the expected `preflight --silent` command; status matched; disable removed the exact value | Missing confirmation exits 1; unexpected value is reported rather than accepted | Keep autostart off; M6 `TASK-028` |
| Login-cycle silent autostart | Not Run | Sign-out/sign-in was not performed | Any sound/program/provider call would be a hard failure | Keep autostart off; M6/M7 |
| Namespaced reset | Passed | Two consecutive resets reported credential/autostart/action log absent, no recursive delete, no user files touched | Hermetic tests preserve sibling and unknown in-scope files; relative scope is rejected | M2 `TASK-007`, full reset M6 `TASK-027` |
| WebView2 runtime detection | Passed | Registered runtime product detected; no version or user path retained here | Missing runtime would block Tauri WebView startup | M2 shell; M7 compatibility matrix |
| Tauri shell build/run | Not Run | M2 shell did not yet exist | Build failure remains explicit | M2 `TASK-004` |
| Tray real action | Not Run | No visible tray action performed | App remains window-only | M6/M7 |
| NSIS per-user install/uninstall | Not Run | `makensis` not detected and no installer exists | No elevation or system-wide fallback is permitted | M7 `TASK-031` |
| Windows 10 and clean-VM matrix | Not Run | Current evidence is one Windows 11 standard-user host | Unsupported cohort remains explicit | M7 beta gate |

## Automated checks

| Check | Result |
|---|---|
| `cargo fmt --manifest-path spikes/windows/Cargo.toml --all -- --check` | Passed |
| `cargo clippy --locked --manifest-path spikes/windows/Cargo.toml --all-targets --all-features -- -D warnings` | Passed |
| `cargo test --locked --manifest-path spikes/windows/Cargo.toml --all-features` | Passed — 8 tests |
| Read-only `preflight` | Passed — default side-effect counters all false; Credential Manager/WebView2/toolchain detected; autostart disabled |
| Confirmed Credential Manager cycle | Passed — value absent after deletion |
| Confirmed HKCU enable/status/disable | Passed — exact expected silent command only; final state disabled |
| Confirmed local notification action simulation | Passed — three actions; zero program/audio/provider effects |
| Confirmed reset twice | Passed — idempotent exact cleanup |
| Unconfirmed mutation commands | Passed — four cases rejected with exit code 1; final autostart state disabled |

## Known gaps

- Real toast activation, action callback, tray lifecycle and login-cycle startup require visible OS/user interaction and were not run. They remain disabled; targets are M5/M6 and the M7 VM matrix.
- No per-user NSIS was built or installed and Windows 10 was not exercised. This does not satisfy full `TEST-COMPAT-001`; target is M7.
- This probe validates Password Vault feasibility but is not the product Credential Service. Canonical provider-origin naming, application-directory reset and canary scans across SQLite/log/export remain M2/M6 work.

## Conclusion

`Passed with known gaps` for the bounded M1 `TASK-003` probe. The exercised standard-user Credential Manager, explicit HKCU autostart, local action model and exact cleanup paths passed without elevation, sound, paid calls, network access or retained probe state. Every unexecuted OS lifecycle scenario remains `Not Run`; none is relabeled as passed.
