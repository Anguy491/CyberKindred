# Test Strategy

| Field | Value |
|---|---|
| Status | Draft |
| Owner | QA Steward |
| Last Verified | 2026-09-01 |
| Source of Truth For | 测试层级、环境、fixture、质量门槛与证据 |
| Related Documents | `ACCEPTANCE-TESTS.md`, `TRACEABILITY.md`, `../product/NFRS.md`, `../contracts/` |

## Principles

- 默认验证 hermetic、可离线、可重复，不依赖真实付费 API、用户音乐或交互式账号。
- 测试行为与契约，不以大面积快照替代断言。
- 每个测试至少引用一个 `FR-*` 或 `NFR-*`；每个首发需求至少有一项自动或明确的手工验收。
- provider/source 使用真实实现的 contract suite 加 transport/session fake；外部服务只在显式 live suite 中验证。
- 时间、天气、文件系统、随机数、音频设备和系统媒体会话均通过可注入边界测试。

## Test layers

| Layer | Planned tooling | Scope | Network |
|---|---|---|---|
| Schema | AJV 2020-12 | schema、valid/boundary/invalid examples、兼容性 | Never |
| Frontend unit/component | Vitest + React Testing Library | stores、formatters、screen states、keyboard/accessibility | Never |
| Frontend browser | Playwright + axe | React routes、responsive screenshots、keyboard and automated accessibility；不证明 Windows/Tauri integration | Never |
| Rust unit | `cargo test` | selection、state machines、validation、retention、errors | Never |
| Contract | Rust fixtures + TypeScript AJV | IPC/provider request/response/event/error parity | Never |
| Integration | Tauri mocks + temporary filesystem/SQLite + fake transports | commands、migrations、scan、queue、memory、schedules | Never |
| Desktop E2E | WebdriverIO + `tauri-driver` on Windows | onboarding、navigation、radio、settings、restart | Mocked by default |
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

## Fixtures and fakes

- Audio fixtures are programmatically generated short WAV plus explicitly licensed/generated MP3/FLAC/OGG/M4A samples; no copyrighted user track.
- Metadata fixtures include Unicode、CJK、emoji、超长标签、无标签、损坏标签、symlink/reparse point 和恶意 HTML/prompt-like 文本。
- `FakeClock` controls DST、sleep/resume、30-day cleanup 和 schedule transitions。
- `FakeMusicSource` advertises independently toggled capabilities and emits ordered/out-of-order session events。
- `FakeLLMProvider` returns valid、unknown track id、duplicate、invalid schema、timeout、rate limit 和 safety refusal。
- `FakeTTSProvider` returns short generated audio、empty/corrupt payload、timeout and cancellation。
- Temporary DB and library paths are created per test and deleted after; tests never traverse outside their explicit root。

## Quality gates

### Per task

- All task-linked tests pass; changed contract examples and traceability pass.
- Changed TypeScript/Rust lines target at least 90% coverage; critical state transitions and security validators require branch-complete tests.
- No skipped/focused tests, unexplained warnings, unbounded waits or live calls in default suites.

### Per milestone

- Repository line coverage at least 80% and branch coverage at least 70%; coverage is evidence, not a substitute for scenario completeness.
- Every milestone exit criterion has recorded evidence in `ACCEPTANCE-TESTS.md` or a linked artifact.
- Zero open Critical/High security finding; Medium findings require Product Owner acceptance and `RISK-*`.
- Contract schema/examples are 100% validated and current migrations pass from every supported schema version.

### Beta release

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
