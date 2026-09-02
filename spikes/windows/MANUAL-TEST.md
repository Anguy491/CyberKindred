# Manual-TASK-003 Windows checklist

| Field | Value |
|---|---|
| Status | Approved |
| Owner | Quality Engineering |
| Last Verified | 2026-09-02 |
| Source of Truth For | `TASK-003` 标准用户、通知、自启动与删除的人工证据步骤 |
| Related Documents | `README.md`, `evidence/TASK-003-MATRIX.md`, `../../docs/testing/TEST-STRATEGY.md` |

Record only de-identified results in `evidence/TASK-003-MATRIX.md`. Never paste a
credential, username, machine name, absolute path, notification identifier, or
registry export.

## Standard-user VM matrix

Run separately on a fully updated Windows 10 22H2 x64 VM and Windows 11 x64 VM,
using a non-administrator account.

1. Build the locked crate and run `preflight`. Record each returned status as
   `Passed`, `NotDetected`, or `NotRunManual`; do not convert missing NSIS or an
   unbuilt Tauri shell to `Passed`.
2. Run `credential-cycle --confirm-probe-write`. Pass only if write, presence,
   in-memory match, delete, and absence-after-delete are true and output contains
   no canary value. Repeat reset once to prove missing/delete is idempotent.
3. Run `autostart status`; the clean VM must report `disabled`. Only for this
   manual step, run explicit enable, sign out/in, and verify the probe starts
   silently without a notification, program, audio, or provider call. Run explicit
   disable and verify the exact HKCU value is absent after another sign-in.
4. Build the M2 Tauri shell when available and manually verify one notification
   containing Start/Snooze/Ignore actions. No action and Ignore must remain silent;
   actions must be reported to Rust. For this M1 probe, real notification activation
   remains `NotRunManual` until that shell exists; the simulation proves only the
   no-side-effect action boundary.
5. With the Tauri shell, verify tray open/quit behavior and that tray idle is
   silent. Until performed, record tray as `NotRunManual`.
6. Build a per-user NSIS candidate, install without elevation, launch, uninstall,
   and inspect documented data handling. Until M7 packaging exists, record NSIS as
   `NotRunManual`; `makensis` detection alone is not a pass.
7. Create an unrelated sibling canary file outside the probe-owned directory, run
   `reset --confirm-probe-only`, and confirm the sibling plus any music/export
   fixture hashes remain unchanged. The exact credential, Run value, and action
   log must be absent.

If a visible toast, tray, sign-in lifecycle, or installer step needs user action,
request it once and do not poll. Missing manual results are checkpoint known gaps,
unless they reveal a hard-gate violation such as unsolicited sound, credential
leakage, elevation, or deletion outside the probe scope.
