# GSMTC read-only probe

| Field | Value |
|---|---|
| Status | Draft |
| Owner | Playback Architecture |
| Last Verified | 2026-09-02 |
| Source of Truth For | `TASK-001` 探针的用途、边界与运行方法 |
| Related Documents | `../../docs/integrations/EXTERNAL-INTEGRATIONS.md`, `../../docs/architecture/adr/ADR-0002-local-and-system-media-sources.md`, `evidence/TASK-001-MATRIX.md` |

该 crate 是 M1 可丢弃技术探针，不进入 CyberKindred 产品运行路径。它只读取 Windows `GlobalSystemMediaTransportControlsSessionManager` 暴露的会话、媒体属性、时间线、播放状态和 capability，不调用任何 `TryPlay*`、`TryPause*`、seek、next 或 previous 控制方法，也不访问 Apple Music Web、账号、资料库或网络。

## Requirements

- Windows 10 22H2 或 Windows 11 x64。
- Rust 1.98.0。
- Apple Music Windows App 已安装；只有 App 正在提供系统媒体会话时才能观察到该会话。

## Commands

一次性快照：

```powershell
cargo run --locked --manifest-path .\spikes\gsmtc\Cargo.toml -- snapshot
```

以 100 ms 周期观察 60 秒；只在权威快照发生变化时输出去除媒体正文的 JSON Lines 事件：

```powershell
cargo run --locked --manifest-path .\spikes\gsmtc\Cargo.toml -- watch --seconds 60 --interval-ms 100
```

验证：

```powershell
cargo fmt --manifest-path .\spikes\gsmtc\Cargo.toml -- --check
cargo clippy --locked --manifest-path .\spikes\gsmtc\Cargo.toml --all-targets -- -D warnings
cargo test --locked --manifest-path .\spikes\gsmtc\Cargo.toml
```

## Privacy and evidence

`snapshot` JSON 可能包含当前曲名、艺术家和专辑。原始快照仅用于本机诊断，不得提交、复制到日志或作为测试 fixture。`watch` JSON Lines 只保留字段是否存在，不输出媒体正文。仓库证据只记录环境、字段是否出现、精确 `SourceAppUserModelId`、capability 布尔值、计数和延迟统计；媒体正文必须写成 `[REDACTED]`。

`watch` 是探针级轮询观察，用来证明读取、变化收敛和字段映射可行；它不是产品事件架构。M6 产品实现仍须订阅 GSMTC events，并按 `NFR-PERF-004` 独立测量“收到事件到 UI 更新”的延迟。
