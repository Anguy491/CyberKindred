# TASK-001 GSMTC probe evidence

| Field | Value |
|---|---|
| Status | Draft |
| Owner | Playback Architecture / QA Steward |
| Last Verified | 2026-09-02 |
| Source of Truth For | `TASK-001` 当前 Windows/Apple Music 实机探针环境、步骤、结果与限制 |
| Related Documents | `../README.md`, `../../../docs/testing/ACCEPTANCE-TESTS.md`, `../../../docs/integrations/EXTERNAL-INTEGRATIONS.md`, `../../../docs/architecture/adr/ADR-0002-local-and-system-media-sources.md` |

## Safety boundary

- Probe mode is read-only; source inspection confirms there are no GSMTC `Try*` control calls.
- No WebView, browser automation, MusicKit, Apple credential, network or source audio access is used.
- Raw media title/artist/album output remains local and is not committed; the evidence below records only redacted presence and boolean capabilities.

## Environment

| Item | Observed value |
|---|---|
| Test date/timezone | 2026-09-02 / Australia/Sydney |
| OS | Microsoft Windows 11 Home, version `10.0.26200`, build `26200` |
| Rust | `rustc 1.98.0` |
| Probe dependency | `windows 0.62.2` |
| Apple Music Windows App | Package `AppleInc.AppleMusicWin` `1.1540.23042.0`; process running during the initial snapshot |
| Account/subscription | Not inspected by CyberKindred |

## TEST-APL-001 matrix

| Scenario | Procedure | Expected | Evidence status |
|---|---|---|---|
| App not running | Close Apple Music, run `snapshot` | No Apple Music GSMTC session; no browser/DOM activity | Pending operator-controlled app state |
| Web only | With App closed, open `music.apple.com`, run `snapshot` | Browser may expose its own AUMID; probe must not classify it as Apple Music App | Pending operator-controlled browser state |
| App installed, no active media session | Open App without starting playback, run `snapshot` | Either no session or an App-owned session with only fields actually exposed | Pass: exact AUMID `AppleInc.AppleMusicWin_nzyj5cx40ttqa!App`; status `opened`; title/artist/album/thumbnail absent; timeline present; zero field read errors |
| App session present | Start playback manually, run `snapshot` | Exact App AUMID plus truthful metadata/timeline/capabilities | Pending user-authorized playback |
| Session disappears | Close App after session exists, observe with `watch` | Session disappears without binding another app | Pending user-authorized playback |
| Capability restricted | Observe a state/track with a disabled control | Corresponding boolean remains false; no synthetic capability | Pending suitable media state |

## TEST-APL-002 change matrix

The acceptance target is 50 operator-triggered changes, at least 49 observed within 2 seconds and all within 5 seconds. This M1 probe records snapshot convergence at the configured polling interval; it does not claim product UI event latency. Raw track text is redacted before any evidence is retained.

| Change type | Attempts | ≤2 s | ≤5 s | Result |
|---|---:|---:|---:|---|
| Track/media properties | 0 | 0 | 0 | Pending user-authorized playback |
| Playing/paused state | 0 | 0 | 0 | Pending user-authorized playback/control |
| Timeline/seek | 0 | 0 | 0 | Pending user-authorized playback/control |
| Capability availability | 0 | 0 | 0 | Pending suitable media states |
| Total | 0 | 0 | 0 | Pending |

## Initial capability snapshot

The App-owned idle session returned the following exact booleans without issuing any control: `play=true`, `playPauseToggle=true`; `pause`, `stop`, `next`, `previous`, `seek`, `playbackRate`, `shuffle`, and `repeat` were all `false`. The probe keeps a failed capability read as JSON `null` plus a field error rather than fabricating `false`.

The bounded one-second watch smoke test completed at a 100 ms interval with no player action and retained no raw output. This proves repeated read access is stable in the observed idle state; it is not evidence for the 50-change latency target.

## Completion rule

`TASK-001` cannot move to `Done` until the hermetic Rust suite passes and the operator-controlled TEST-APL-001/002 rows have reproducible evidence. The probe may be committed while the task remains `In Progress`; no missing manual evidence is converted into an assumed pass.
