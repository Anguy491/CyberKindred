# Changelog

| Field | Value |
|---|---|
| Status | Approved |
| Owner | Release Steward |
| Last Verified | 2026-09-02 |
| Source of Truth For | 用户可感知变更与破坏性契约变更历史 |
| Related Documents | `docs/operations/BUILD-RELEASE.md`, `docs/planning/BACKLOG.md` |

所有用户可感知行为、数据迁移和破坏性契约变化记录于此。版本号遵循 Semantic Versioning；内测前使用 `0.x.y`。

## [Unreleased]

### Changed

- Documentation Baseline v1 于 2026-09-02 获用户批准，项目进入 M1 技术探针阶段。

### Added

- `TASK-001` 只读 GSMTC 技术探针：可枚举 Windows 媒体会话、元数据字段存在性、时间线与实时 capability；watch 输出自动移除媒体正文并过滤 timeline 心跳。Apple Music 空闲与播放会话实测通过，手工变化矩阵完成至少 12/50，余下延迟样本仍待执行。
- Documentation Baseline v1：产品需求、可测量非功能需求、AI 行为与 Nothing Design 文字化 UX 规范。
- Tauri/React/Rust 架构、运行时状态机、AI 编排、SQLite 数据模型、依赖政策及六项初始 ADR。
- 版本化 Tauri IPC、provider contracts、六份 JSON Schema 及合法、边界、非法示例。
- Apple Music Windows App/GSMTC、本地音乐、OpenAI、MusicBrainz、Cover Art Archive 与 Open-Meteo 集成边界。
- 威胁模型、隐私生命周期、许可证、安全与发布治理基线。
- 测试策略、验收场景、双向追踪矩阵、里程碑、原子 Backlog、风险登记和运行手册。
- 根目录与 `src`、`src-tauri`、`tests` 分层 `AGENTS.md`，以及离线文档一致性验证脚本。
