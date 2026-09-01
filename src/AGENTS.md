# Frontend Agent Instructions

| Field | Value |
|---|---|
| Status | Draft |
| Owner | Frontend Steward |
| Last Verified | 2026-09-01 |
| Source of Truth For | `src/` 内的前端执行约束 |
| Related Documents | `../AGENTS.md`, `../docs/product/UX-SPEC.md`, `../docs/contracts/API-CONTRACT.md` |

- 修改前阅读 `../docs/product/UX-SPEC.md`、`../docs/contracts/API-CONTRACT.md` 与相关 `FR-*`/`NFR-*`。
- 前端只能通过契约化 Tauri IPC 与 Rust 通信；不得直接读取本地音乐目录、SQLite 或 Windows Credential Manager。
- 不在 WebView、localStorage、日志、错误报告或测试快照中保存 API Key。
- 遵守 OLED 深色 Nothing Design token、三层视觉层级、键盘路径和内联状态模式；不添加渐变、阴影、骨架屏、弹跳动画或应用内 Toast。
- 保持 generated/contract types 只读；契约变化必须先修改权威 schema 与 `API-CONTRACT.md`。
- UI 测试覆盖 loading、empty、error、offline、disabled 和 capability 缺失状态。
