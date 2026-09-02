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
- 开发治理改为逐 milestone 验收：M1–M6 使用轻量 technical/prototype checkpoint，任务仅作为可并行的内部交付切片；覆盖率、完整兼容/无障碍/性能/长稳矩阵可记录后递延到 M7，secret、权限、日志、用户确认、付费调用、数据完整性、公共契约和 Critical/High 安全问题仍是即时硬门槛。
- `TASK-003` 仅依赖已批准的文档基线且与 `TASK-002` 写入范围不重叠，按新的 milestone 并行规则从 `Blocked` 调整为 `Ready`。
- `TASK-002` 的 `Manual-TASK-002` 人工清单保留为 M1 checkpoint 证据，不再单独触发任务级用户签字；产品阶段的自动兼容性、性能与恢复验收不变。
- 阻塞处理增加 token budget：自动排障最多三次有差异尝试；明确需要用户介入的步骤只请求一次并等待；达到上限后转向独立任务或暂停 Goal，禁止轮询不变状态。

### Added

- `ADR-0007` 记录 milestone-based prototype delivery、单 milestone 多任务受控并行、checkpoint evidence 与 known-gap 规则。
- `TASK-002` 本地音频探针实现已标记 `Done`：含六类 CC0 生成 fixture 与 SHA-256 清单、Symphonia 0.6.1 全量解码报告、lofty 标签/封面报告、rodio 交互播放/暂停/seek、默认设备重建后保持暂停、损坏文件隔离、脱敏事件与 CPU/内存采样手册。人工证据在 M1 checkpoint 与其他探针结果一并审阅。
- `TASK-001` 只读 GSMTC 技术探针已由用户验收完成：可枚举 Windows 媒体会话、元数据字段存在性、时间线与实时 capability；watch 输出自动移除媒体正文并过滤 timeline 心跳。Apple Music 空闲、播放、App 关闭、Web-only 隔离与会话消失场景通过，M1 输入到探针收敛矩阵 50/50 在 2 秒内；产品 UI 延迟验收仍由 M6 `TASK-025` 承担。
- Documentation Baseline v1：产品需求、可测量非功能需求、AI 行为与 Nothing Design 文字化 UX 规范。
- Tauri/React/Rust 架构、运行时状态机、AI 编排、SQLite 数据模型、依赖政策及六项初始 ADR。
- 版本化 Tauri IPC、provider contracts、六份 JSON Schema 及合法、边界、非法示例。
- Apple Music Windows App/GSMTC、本地音乐、OpenAI、MusicBrainz、Cover Art Archive 与 Open-Meteo 集成边界。
- 威胁模型、隐私生命周期、许可证、安全与发布治理基线。
- 测试策略、验收场景、双向追踪矩阵、里程碑、原子 Backlog、风险登记和运行手册。
- 根目录与 `src`、`src-tauri`、`tests` 分层 `AGENTS.md`，以及离线文档一致性验证脚本。
