# ADR-0006: 文档与契约驱动开发

| Metadata | Value |
|---|---|
| Status | Approved |
| Owner | Project Governance |
| Last Verified | 2026-09-02 |
| Source of Truth For | Documentation Baseline、可追踪 ID、机器契约与任务门禁的决策理由 |
| Related Documents | [Docs Index](../../INDEX.md), [FRS](../../product/FRS.md), [NFRS](../../product/NFRS.md), [API Contract](../../contracts/API-CONTRACT.md), [Traceability](../../testing/TRACEABILITY.md), [Backlog](../../planning/BACKLOG.md), [ADR-0007](ADR-0007-milestone-prototype-delivery.md), [Repository AGENTS](../../../AGENTS.md) |

> 2026-09-02 amendment: ADR-0007 supersedes this ADR only for task concurrency, task-level sign-off and quality-gate timing. Documentation authority, contract synchronization and approval boundaries below remain in force.

## Context

CyberKindred 涉及 AI 行为、用户隐私、本地文件、音频、Windows 媒体控制和多个外部 provider。Codex 将在里程碑内自主开发；若只有口头计划或代码内隐含决策，代理容易跨越权限、实现不可测需求或让契约与测试漂移。产品代码开始前需要一套可审查、可追踪、机器可验证的共同基线。

## Decision

采用 Documentation Baseline v1：

- PRD、FRS、NFRS、UX、AI behavior、Architecture、ADR、API/provider contracts、JSON Schema、security/privacy、test strategy、traceability、roadmap/backlog 和 operations 文档各自拥有唯一事实范围。
- 使用 `FR-*`、`NFR-*`、`UX-*`、`ARCH-*`、`API/EVT/ERR-*`、`TEST-*`、`TASK-*`、`RISK-*`、`ADR-*` 建立需求 → 架构/契约 → 测试 → 任务追踪。
- 公共 DTO 以 contracts/schemas 与 API Contract 为权威；Rust/TypeScript 通过 contract tests 验证，不允许 silent drift。
- Codex 的交付节奏与验收单位遵循 ADR-0007；行为/契约改变仍先更新权威文档，再更新代码与相关测试，公共契约和安全/隐私语义不得延后同步。
- Documentation Baseline 获用户批准前不创建产品脚手架或实现代码。范围扩大、隐私变化、付费服务、外部发布、不可逆操作和高风险生产依赖必须暂停询问。

## Alternatives considered

- **代码优先、事后补文档**：前期快，但 AI/系统边界难以审查，验收与数据语义容易漂移。
- **单一大型设计文档**：入口少，但职责重叠、更新冲突和事实重复严重，无法精确追踪。
- **仅 issue/backlog 驱动**：任务明确，但缺少长期架构、隐私和公共契约权威。
- **把实现代码/类型作为全部事实源**：适合低风险内部工具，但无法在实现前批准产品行为，也不足以说明法律、AI 和用户控制。

## Consequences

- 开始编码较晚，但每个任务仍有明确依赖、范围、建议自检与停止条件；用户验收和非关键证据改在 milestone checkpoint 聚合。
- 文档变更成为实现工作的一部分；review 必须检查 ID 引用、schema 示例和 traceability。
- 同一事实只能有一个权威文件，其他文档用链接/ID 引用，避免复制粘贴规范。
- ADR 解释理由与后果，不替代 FRS/NFRS；改变用户可见行为仍先修改需求并获得相应批准。
- 必须维护文档一致性检查和 contract test，否则该决策只形成形式负担而不提供约束价值。
