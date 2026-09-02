# Test Strategy

| Field | Value |
|---|---|
| Status | Approved |
| Owner | QA Steward |
| Last Verified | 2026-09-02 |
| Source of Truth For | 测试层级、环境、fixture、质量门槛与证据 |
| Related Documents | `ACCEPTANCE-TESTS.md`, `TRACEABILITY.md`, `../product/NFRS.md`, `../planning/ROADMAP.md`, `../architecture/adr/ADR-0007-milestone-prototype-delivery.md`, `../contracts/` |

## Principles

- 默认验证 hermetic、可离线、可重复，不依赖真实付费 API、用户音乐或交互式账号。
- 测试行为与契约，不以大面积快照替代断言。
- 每个测试至少引用一个 `FR-*` 或 `NFR-*`；每个首发需求至少有一项自动或明确的手工验收。
- provider/source 使用真实实现的 contract suite 加 transport/session fake；外部服务只在显式 live suite 中验证。
- 时间、天气、文件系统、随机数、音频设备和系统媒体会话均通过可注入边界测试。
- `ACCEPTANCE-TESTS.md` 是最终 beta 的测试库存；M1–M6 每次只选择能证明当前 milestone 主路径与硬门槛的 checkpoint subset，未选择的场景不是失败，但必须在 known gaps 中可追踪。

## Test layers

| Layer | Planned tooling | Scope | Network |
|---|---|---|---|
| Schema | AJV 2020-12 | schema、valid/boundary/invalid examples、兼容性 | Never |
| Frontend unit/component | Vitest + React Testing Library | stores、formatters、screen states、keyboard/accessibility | Never |
| Frontend browser | Playwright + axe | React routes、responsive screenshots、keyboard and automated accessibility；不证明 Windows/Tauri integration | Never |
| Rust unit | `cargo test` | selection、state machines、validation、retention、errors | Never |
| Contract | Rust fixtures + TypeScript AJV | IPC/provider request/response/event/error parity | Never |
| Integration | Tauri mocks + temporary filesystem/SQLite + fake transports | commands、migrations、scan、queue、memory、schedules | Never |
| Desktop E2E | Repository-owned Node 24 W3C WebDriver harness + `tauri-driver` on Windows | onboarding、navigation、radio、settings、restart | Mocked by default |
| Performance | release build harness + controlled library generator | start, scan, memory, DB and audio budgets | Never |
| Security/privacy | unit/integration/static checks | secret leakage、path scope、redaction、export/delete、CSP/IPC | Never |
| Live integration | tagged manual/automation | OpenAI, MusicBrainz, Open-Meteo, Apple Music | Explicit |

## Canonical commands after M2

| Command | Purpose |
|---|---|
| `pnpm lint` | Markdown/TypeScript/React static rules |
| `pnpm typecheck` | Strict TypeScript and generated contract types |
| `pnpm test` | Frontend unit/component tests |
| `pnpm test:contracts` | JSON Schema examples and IPC/provider parity |
| `pnpm test:e2e` | Mocked Windows desktop E2E |
| `cargo fmt --all -- --check` | Rust formatting |
| `cargo clippy --all-targets --all-features -- -D warnings` | Rust lint gate |
| `cargo test --all-features` | Rust unit/integration tests |
| `pnpm verify:docs` | Links, IDs, metadata, traceability and schema examples |

Before M2, `scripts/verify-docs.ps1` is the canonical documentation gate.

`pnpm test:e2e` uses only Node 24 built-ins to speak the W3C WebDriver protocol to the pinned `tauri-driver`; it does not depend on WebdriverIO or download a browser driver during `pnpm install`. The harness builds the unpackaged desktop executable when needed, owns a loopback-only driver process/session, always attempts session deletion and process cleanup, and accepts an optional case-name filter after `--`. Windows CI must provision the Edge WebDriver that matches the installed WebView2/Edge runtime before invoking this command. Renderer-only Playwright evidence remains a separate layer and never substitutes for this desktop path.

M1 的 `TASK-002` 真实解码听感、播放/暂停/seek、默认设备切换、错误隔离与资源采样仍使用 `spikes/audio/MANUAL-TEST.md` 的 `Manual-TASK-002`，但证据与其他探针在 M1 checkpoint 集中审阅，不再形成任务级签字点。`cargo fmt`、`cargo clippy` 和 release build 是建议的快速自检；后续产品自动化仍由 `TEST-LIB-001`、`TEST-RAD-002`、`TEST-APL-004` 与 M7 release gate 覆盖。

## Fixtures and fakes

- Audio fixtures are programmatically generated short WAV plus explicitly licensed/generated MP3/FLAC/OGG/M4A samples; no copyrighted user track.
- Metadata fixtures include Unicode、CJK、emoji、超长标签、无标签、损坏标签、symlink/reparse point 和恶意 HTML/prompt-like 文本。
- `FakeClock` controls DST、sleep/resume、30-day cleanup 和 schedule transitions。
- `FakeMusicSource` advertises independently toggled capabilities and emits ordered/out-of-order session events。
- `FakeLLMProvider` returns valid、unknown track id、duplicate、invalid schema、timeout、rate limit 和 safety refusal。
- `FakeTTSProvider` returns short generated audio、empty/corrupt payload、timeout and cancellation。
- Temporary DB and library paths are created per test and deleted after; tests never traverse outside their explicit root。

## Quality gates

### Task self-checks

- 执行与实际改动直接相关的最快 lint/typecheck/build 与 focused test；具体命令由 Backlog 的“建议自检”提供，允许按改动范围裁剪。
- 改动公共 schema/IPC/provider contract 时，相关合法/边界/非法示例与两端校验必须通过；不得把契约失败递延到 milestone。
- 任务 `Done` 不要求逐任务覆盖率、完整 E2E、平台矩阵或人工签字。未执行的非硬门槛检查在 checkpoint known gaps 中登记，不得伪记为通过。
- 默认自检不访问真实付费服务，不使用用户音乐，不自动出声；focused/skipped 标记不得进入 M7 release candidate。

### M1–M6 prototype checkpoint

- candidate 在主要开发机可启动，并能完成 Roadmap 为当前 milestone 定义的主路径或技术演示。
- 对当前主路径至少保留一次成功证据和一个最重要失败/降级路径的结果；测试可以是 focused automation、人工 smoke 或两者组合。
- 相关 hard gates 全部通过：secret 不进入前端/日志，文件和网络不越权，声音与可能付费动作只由用户明确触发，改动过的公共契约有效，无已知数据损坏，无 open Critical/High security finding。
- 覆盖率只采集趋势，不设阻塞阈值；完整 Win10/Win11、分辨率/缩放、Narrator、10,000 首、200 次采样与 soak 默认递延到 M7，除非它们是当前 milestone 的明确目标。
- checkpoint 记录 candidate commit、环境、已执行检查、实际失败、known gaps、规避方式和目标 milestone。hard gates 通过时由 Lead Agent 记录 `Passed` 或 `Passed with known gaps` 并自动转场；实际失败不能改写成通过。hard gate 失败时记录 `Blocked`，不得跨 milestone 绕过。

### M7 beta release

- Repository line coverage at least 80% and branch coverage at least 70%; coverage is evidence, not a substitute for scenario completeness.
- Every P0/P1 requirement and applicable `TEST-*` has recorded passing evidence in `ACCEPTANCE-TESTS.md` or a linked artifact.
- Zero open Critical/High security finding; Medium findings require Product Owner acceptance and `RISK-*`.
- Contract schema/examples are 100% validated and current migrations pass from every supported schema version.
- Full default suite passes twice on clean checkout/build.
- Manual Windows 10 22H2 and Windows 11 x64 smoke test passes.
- Real Apple Music subscription suite and 10,000-track performance suite pass in recorded environments.
- Installer hash, install/uninstall, retained/deleted data, restart recovery and 30-minute radio session pass.

## Live-test isolation

Live tests require an explicit environment flag and never run in default CI:

- `CYBERKINDRED_LIVE_OPENAI=1`
- `CYBERKINDRED_LIVE_METADATA=1`
- `CYBERKINDRED_LIVE_WEATHER=1`
- `CYBERKINDRED_LIVE_APPLE_MUSIC=1`

Credentials come only from the OS secret store or process environment and are never echoed. Live results record provider/app version、timestamp、storefront/region when relevant and redact identifiers.

## Failure evidence and flake policy

A failure record includes requirement/test ID、environment、steps、expected、actual and redacted log. A test that fails non-deterministically twice in ten identical runs is quarantined with a blocking `RISK-*` and repair task; it is not silently retried into green.
