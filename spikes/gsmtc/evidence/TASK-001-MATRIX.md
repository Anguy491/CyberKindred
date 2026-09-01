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
| App session present | Start playback manually, run `snapshot` | Exact App AUMID plus truthful metadata/timeline/capabilities | Pass: `playing`; title/artist/album-artist presence and thumbnail reported without retaining text; 361 s timeline; pause/stop/next/previous available; seek unavailable; zero field errors |
| Session disappears | Close App after session exists, observe with `watch` | Session disappears without binding another app | Pending user-authorized playback |
| Capability restricted | Observe a state/track with a disabled control | Corresponding boolean remains false; no synthetic capability | Pass: playing exposed pause but not play/seek; paused exposed play but not pause/seek; values were not synthesized |

## TEST-APL-002 change matrix

The acceptance target is 50 operator-triggered changes, at least 49 observed within 2 seconds and all within 5 seconds. This M1 probe records snapshot convergence at the configured polling interval; it does not claim product UI event latency. Raw track text is redacted before any evidence is retained. The 2026-09-02 assisted run did not machine-timestamp the operator's hand movement, so observed changes count toward compatibility coverage but are not assigned fabricated latency scores.

| Change type | Attempts | ≤2 s | ≤5 s | Result |
|---|---:|---:|---:|---|
| Track/media properties | ≥4 | — | — | Four unambiguous duration-boundary changes observed; media text not retained |
| Playing/paused state | 7 | — | — | Seven stable `playing↔paused` transitions observed in a 35 s focused run; final state `paused` |
| Timeline/seek | 1 | — | — | Discontinuous position change from about 61 s to 113 s observed while the same 239 s media item remained active |
| Capability availability | ≥1 | — | — | Playing and paused snapshots exposed inverse play/pause booleans; seek remained false |
| Total distinct operator changes | ≥12 / 50 | — | — | Compatibility path passes so far; latency target and remaining 38 changes pending |

## Initial capability snapshot

The App-owned idle session returned the following exact booleans without issuing any control: `play=true`, `playPauseToggle=true`; `pause`, `stop`, `next`, `previous`, `seek`, `playbackRate`, `shuffle`, and `repeat` were all `false`. The probe keeps a failed capability read as JSON `null` plus a field error rather than fabricating `false`.

The bounded one-second idle watch smoke test completed at a 100 ms interval with no player action and retained no raw output. During live playback, Apple Music advanced `LastUpdatedTime` roughly every 200–300 ms. The probe now treats that timeline heartbeat and normal position advancement as noise, while retaining media/status/capability changes and discontinuous seek jumps. Automated tests cover heartbeat suppression, seek detection and media/status detection. These observations prove repeated read access and semantic filtering in this environment; they are not evidence for the 50-change latency target.

## Completion rule

`TASK-001` cannot move to `Done` until the hermetic Rust suite passes and the operator-controlled TEST-APL-001/002 rows have reproducible evidence. The probe may be committed while the task remains `In Progress`; no missing manual evidence is converted into an assumed pass.
