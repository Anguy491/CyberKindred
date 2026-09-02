# Development Guide

| Field | Value |
|---|---|
| Status | Approved |
| Owner | Developer Experience Steward |
| Last Verified | 2026-09-02 |
| Source of Truth For | Windows 开发环境、目录与本地命令 |
| Related Documents | `../../AGENTS.md`, `../architecture/DEPENDENCY-POLICY.md`, `BUILD-RELEASE.md` |

## Supported development host

- Windows 10 22H2 or Windows 11 x64
- PowerShell 7 preferred; Windows PowerShell is supported for bootstrap tasks
- Node.js `24.19.0`, pnpm `11.16.0` through Corepack
- Rust `1.98.0`; M2 must pin this exact version in `rust-toolchain.toml`
- Visual Studio Build Tools with Desktop development with C++ and Windows SDK
- Microsoft Edge WebView2 Runtime

Verified on the baseline host at 2026-09-01: Node `24.19.0`, npm `11.17.0`, pnpm `11.16.0`, rustc/cargo `1.98.0`, .NET `9.0.305`.

## Planned repository layout

```text
src/                    React/TypeScript UI
src-tauri/              Rust core, migrations and Tauri configuration
tests/                  cross-layer and desktop E2E
docs/                   approved product/engineering baseline
scripts/                repository verification utilities
```

Only `src/AGENTS.md`, `src-tauri/AGENTS.md` and `tests/AGENTS.md` exist before user approval; their directories are not product scaffolding.

## Setup after TASK-004

```powershell
corepack enable
pnpm install --frozen-lockfile
rustup show
pnpm verify:docs
```

Dependency installation must use the committed lockfiles. Do not install packages globally except toolchain components documented here.

## Planned commands

```powershell
pnpm dev
pnpm lint
pnpm typecheck
pnpm test
pnpm test:contracts
pnpm test:e2e
pnpm tauri build
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

## Fast iteration loop

Daily development optimizes for short feedback inside the Active milestone:

1. Claim one or more dependency-ready tasks only when their write scopes do not overlap.
2. Run the narrowest relevant lint/typecheck/build and focused tests while iterating; changed contracts and prototype hard gates are never skipped.
3. Move an integrated slice to `Review`/`Done` without waiting for Product Owner task-level acceptance. Record expensive or non-critical unrun checks in the milestone known gaps.
4. When all planned slices are integrated, build one candidate and run the milestone checkpoint subset from `TEST-STRATEGY.md`.

Do not run the full E2E, coverage, platform, accessibility, performance and soak matrix after every task. Those checks are selected when they prove the current milestone and are otherwise consolidated at M7.

Before application scaffolding, run documentation checks directly:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\verify-docs.ps1
```

## Configuration and secrets

- Committed `.env.example` may contain only names and non-secret examples.
- OpenAI Key is entered through the app and stored by Rust in Windows Credential Manager; it is never a frontend environment variable.
- Live-test environment flags enable suites but do not contain credentials.
- Developer logs default to `%LOCALAPPDATA%\CyberKindred\logs`; tests redirect to temporary directories.

## Data locations

Planned production data is under `%LOCALAPPDATA%\CyberKindred`; configuration, SQLite, artwork cache, TTS cache and logs use separate subdirectories. User-selected library files are read in place and never copied or modified. Tests must override the application data root.

## Troubleshooting

| Symptom | Check |
|---|---|
| Tauri build cannot find linker/SDK | Verify Visual Studio Build Tools C++ workload and Windows SDK |
| UI window is blank | Verify WebView2 Runtime; inspect redacted dev log and CSP errors |
| Rust command unavailable | Confirm repository root and `src-tauri` scaffolding after M2 |
| Docs verification fails | Fix the first reported missing metadata/ID/link/schema issue; do not suppress it |
| Apple Music session absent | Follow `RUNBOOK.md`; presence is not required for default development suite |
