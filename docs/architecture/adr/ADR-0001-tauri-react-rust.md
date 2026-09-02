# ADR-0001: 采用 Tauri 2、React、TypeScript、Vite 与 Rust

| Metadata | Value |
|---|---|
| Status | Approved |
| Owner | Core Architecture |
| Last Verified | 2026-09-02 |
| Source of Truth For | Windows 桌面技术栈及 WebView/Rust 权限边界的决策理由 |
| Related Documents | [Architecture](../ARCHITECTURE.md), [Dependency Policy](../DEPENDENCY-POLICY.md), [NFRS](../../product/NFRS.md), [ADR-0006](ADR-0006-document-and-contract-driven-development.md) |

## Context

CyberKindred v1 只面向 Windows，需要现代 UI、系统托盘/通知/自启动、SQLite、本地音频、文件扫描、Credential Manager 与 GSMTC。它也必须把 API Key、文件和 Windows 能力隔离在受信边界内，并控制安装体积与后台资源占用。

## Decision

采用 Tauri 2 作为 desktop shell，React + TypeScript + Vite 作为 WebView 表现层，Rust 作为受信应用核心。Rust 唯一拥有文件、SQLite、网络、secret、audio、GSMTC 和 OS integration；React 只使用 API Contract 中 allowlisted、typed Tauri commands/events。生产 CSP 禁止 remote scripts 与 eval，Tauri capability 采用最小授权。发行物为 per-user NSIS x64 installer。

## Alternatives considered

- **Electron + Node**：生态成熟，但引入完整 Chromium/Node，资源和攻击面更大，且更容易让前端获得文件/网络/secret 权限。
- **纯 WinUI 3/.NET**：Windows 集成自然，但团队希望用 React 快速迭代 Nothing 风格 UI；跨 Rust 音频/生态桥接会增加边界。
- **纯 Rust UI（egui/iced）**：权限模型清晰，但复杂排版、可访问性与 UI 迭代成本高于当前目标。
- **PWA/localhost server**：无法可靠覆盖本地音频、Credential Manager、GSMTC 与安装态 OS 生命周期，也扩大 HTTP 攻击面。

## Consequences

- 获得较小安装体积、Rust 类型/所有权与 Windows API 能力，同时保留 React UI 开发效率。
- 必须维护严格 IPC contract、Tauri capabilities、CSP 与 DTO 同步测试；WebView 不是可信内部调用者。
- WebView2 是 Windows runtime 前提；构建/发行必须检查其可用性与安装策略。
- Rust/JS 双生态带来两个 lockfile 与供应链扫描面，按 Dependency Policy 管理。
- 任何将通用 fs/http/shell/credential capability 暴露给 WebView 的提议都与本 ADR 冲突，需要新的批准 ADR。

## M1 validation note

2026-09-02 的 `TASK-003` 在 Windows 11 x64 标准用户账户验证了 WinRT Password Vault canary 写入/匹配/删除、精确 HKCU Run 启停以及 namespaced reset，不需要管理员权限；WebView2 Runtime 也可从系统注册信息检测。默认 preflight、通知 action simulation 与自启动命令均保持静音且没有 provider/网络能力。真实 Tauri window、toast activation、tray、login cycle 与 per-user NSIS 尚未运行，明确递延至 M2/M5/M6/M7，因此本观察支持但不扩大本 ADR 的权限或部署边界。

## M2 foundation validation note

2026-09-02 的 `TASK-004` 在 Windows 11 x64 标准用户账户构建并启动了 Tauri 2.11.5 + React 19.2.8 + TypeScript 7.0.2 的静音空壳窗口。production CSP 只允许本地资源和 Tauri IPC，main window capability 只有 `core:default`，没有 command、plugin、provider、文件、数据库、Credential、音频或任意网络能力。两套 lockfile、strict TypeScript、Oxlint、Vitest coverage、Vite build、Rust fmt/clippy/test 和文档门槛均可从固定工具版本执行；这验证技术栈基础但不提前证明后续 IPC、数据、provider、installer 或产品行为。
