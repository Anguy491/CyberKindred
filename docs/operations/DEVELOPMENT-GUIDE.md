# Development Guide

| Field | Value |
|---|---|
| Status | Draft |
| Owner | Developer Experience Steward |
| Last Verified | 2026-09-01 |
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
