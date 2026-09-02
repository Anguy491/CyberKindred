# Documentation Index

| Field | Value |
|---|---|
| Status | Approved |
| Owner | Documentation Steward |
| Last Verified | 2026-09-02 |
| Source of Truth For | 文档导航、领域权威性、阅读顺序与变更联动 |
| Related Documents | `../AGENTS.md`, `planning/ROADMAP.md`, `planning/BACKLOG.md`, `architecture/adr/ADR-0007-milestone-prototype-delivery.md` |

## Baseline rule

Documentation Baseline v1 已于 2026-09-02 获用户批准，基线文档状态为 `Approved`，并由本地标签 `docs-baseline-v1` 冻结。此后行为或契约变化必须按下方联动规则修改，不能静默偏离基线。

## Authority matrix

| Question | Authoritative source | Must not be redefined in |
|---|---|---|
| 产品为何存在、首发范围与非目标 | `product/PRD.md` | README、Roadmap、ADR |
| 用户可观察行为 | `product/FRS.md` | 架构、测试、Backlog |
| 可测质量与平台门槛 | `product/NFRS.md` | 构建指南、测试用例 |
| 人格、语气与安全表达 | `product/AI-BEHAVIOR.md` | prompt 实现、UX 文案 |
| 页面与交互状态 | `product/UX-SPEC.md` | 前端组件、README |
| 组件边界与数据流 | `architecture/ARCHITECTURE.md` | Roadmap、provider contract |
| 运行状态合法转换 | `architecture/RUNTIME-STATE-MACHINES.md` | 播放器实现、验收用例 |
| AI 编排与记忆管线 | `architecture/AI-ORCHESTRATION.md` | prompt 字符串、Backlog |
| 持久化模型与保留实现 | `architecture/DATA-MODEL.md` | Privacy 摘要、迁移代码 |
| 决策理由与替代方案 | `architecture/adr/` 中最新 `Approved` ADR；基线批准前使用对应 `Draft` | 当前架构说明、PRD |
| IPC、事件、错误与 provider 载荷 | `contracts/` 与 `contracts/schemas/` | Rust/TypeScript 手写类型 |
| 数据收集、外发、保留与删除 | `security/PRIVACY-DATA-LIFECYCLE.md` | 设置文案、日志代码 |
| 测试层级与质量门槛 | `testing/TEST-STRATEGY.md` | Backlog、CI 配置 |
| 可执行顺序与任务状态 | `planning/BACKLOG.md` | Roadmap、提交信息 |
| 里程碑范围、阶段退出与用户验收 | `planning/ROADMAP.md` | Task DoD、测试用例 |

权威文档按领域裁决，不存在一条覆盖所有领域的总优先级。若两个权威来源冲突，相关任务停止，先由 Product Owner 确认行为，再更新受影响文档与 ADR。

## Required reading paths

### Product review

`PRD -> FRS -> NFRS -> AI-BEHAVIOR -> UX-SPEC -> TRACEABILITY`

### Implementation task

`AGENTS -> ROADMAP active milestone -> BACKLOG claimed task(s) -> linked FR/NFR -> relevant architecture/ADR -> contract/schema -> milestone-relevant tests`

### Security or privacy change

`SECURITY -> THREAT-MODEL -> PRIVACY-DATA-LIFECYCLE -> DATA-MODEL -> API/PROVIDER contracts -> linked tests and risks`

### Release

`ROADMAP milestone gate -> BUILD-RELEASE -> TEST-STRATEGY -> ACCEPTANCE-TESTS -> RUNBOOK -> CHANGELOG`

## Document map

| Area | Documents |
|---|---|
| Governance | `README.md`, `AGENTS.md`, `CONTRIBUTING.md`, `SECURITY.md`, `CHANGELOG.md` |
| Product | `product/PRD.md`, `FRS.md`, `NFRS.md`, `AI-BEHAVIOR.md`, `UX-SPEC.md` |
| Architecture | `architecture/ARCHITECTURE.md`, `RUNTIME-STATE-MACHINES.md`, `AI-ORCHESTRATION.md`, `DATA-MODEL.md`, `DEPENDENCY-POLICY.md`, `architecture/adr/` |
| Contracts | `contracts/API-CONTRACT.md`, `PROVIDER-CONTRACTS.md`, `contracts/schemas/`, `contracts/examples/` |
| Integrations | `integrations/EXTERNAL-INTEGRATIONS.md` |
| Security | `security/THREAT-MODEL.md`, `PRIVACY-DATA-LIFECYCLE.md`, `LEGAL-AND-LICENSING.md` |
| Quality | `testing/TEST-STRATEGY.md`, `ACCEPTANCE-TESTS.md`, `TRACEABILITY.md` |
| Planning | `planning/ROADMAP.md`, `BACKLOG.md`, `RISK-REGISTER.md` |
| Operations | `operations/DEVELOPMENT-GUIDE.md`, `BUILD-RELEASE.md`, `RUNBOOK.md` |

## Change coupling

| Change | Required companion updates |
|---|---|
| Add/change user behavior | FRS, UX/AI behavior when relevant, acceptance tests, traceability, Backlog, Changelog |
| Add/change NFR | NFRS, test strategy/test case, traceability, risk when applicable |
| Add/change command/event/schema | API contract, schema/examples, provider contract if relevant, contract tests, traceability, Changelog if breaking |
| Add/change persistence | Data model, privacy lifecycle, threat model, migration/recovery tests, Runbook |
| Change architecture choice | New/superseding ADR, Architecture, Dependency Policy, Risk Register |
| Add external service/data flow | External Integrations, Privacy, Threat Model, Legal/Licensing, NFR cost/offline tests |

实现期间可把非契约性的 Backlog、Traceability 与 Changelog 整理批量留到当前 milestone checkpoint，但不得让代码与公共契约、安全/隐私语义或用户已批准行为发生暂时漂移。任务状态只追踪内部交付；milestone 状态和验收结论由 Roadmap 与 checkpoint 记录裁决。

## Status and ID discipline

- IDs are never reused after merge; removed items become `Superseded` with a pointer to their replacement.
- Every requirement appears exactly once as a definition; other documents reference its ID and link.
- `Draft` artifacts cannot be implementation acceptance sources. Approved artifacts may be changed only in the same task that updates their dependants.
- Time-sensitive external claims record a verification date and primary-source link.
