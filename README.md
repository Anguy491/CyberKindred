# CyberKindred

| Field | Value |
|---|---|
| Status | Approved |
| Owner | Documentation Steward |
| Last Verified | 2026-09-05 |
| Source of Truth For | 项目入口与文档导航 |
| Related Documents | `docs/INDEX.md`, `docs/product/PRD.md` |

> 一个了解你、陪伴你的 Windows AI 电台朋友。

CyberKindred 以本地曲库为可精确编排的主播放源，并可连接 Apple Music Windows App 的系统媒体会话，在不接管其曲库的前提下读取当前歌曲、控制基础播放并插入 AI 主播串场。首版采用用户自备 OpenAI API Key、本地优先记忆和可见可删除的数据控制。

## Current status

**Documentation Baseline v1** 已于 2026-09-02 获用户批准。M1–M5 已以 checkpoint 完成；Product Owner 于 2026-09-05 授权开启 M6 Apple Music companion mode。当前产品代码已包含 Windows Tauri 桌面外壳、本地曲库与电台、文字对话/记忆、手动城市天气和通知后确认开播的日程。

## Target

- Windows 10 22H2 / Windows 11 x64
- Tauri 2 + React + TypeScript + Rust
- 简体中文优先
- 本地音乐 + Apple Music System Media Session
- OLED 深色 Nothing-inspired UI

## Documentation

从 [Documentation Index](docs/INDEX.md) 开始阅读。关键入口：

- [Product Requirements](docs/product/PRD.md)
- [Functional Requirements](docs/product/FRS.md)
- [Non-functional Requirements](docs/product/NFRS.md)
- [Architecture](docs/architecture/ARCHITECTURE.md)
- [API Contract](docs/contracts/API-CONTRACT.md)
- [Test Strategy](docs/testing/TEST-STRATEGY.md)
- [Roadmap](docs/planning/ROADMAP.md)
- [Backlog](docs/planning/BACKLOG.md)

## Development

当前文档阶段的快速检查：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\verify-docs.ps1
```

环境、后续启动命令和故障排除由 [Development Guide](docs/operations/DEVELOPMENT-GUIDE.md) 维护。贡献前先阅读 [AGENTS.md](AGENTS.md) 与 [CONTRIBUTING.md](CONTRIBUTING.md)。
