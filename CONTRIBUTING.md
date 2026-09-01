# Contributing to CyberKindred

| Field | Value |
|---|---|
| Status | Draft |
| Owner | Lead Agent |
| Last Verified | 2026-09-01 |
| Source of Truth For | Git、任务、审查与协作流程 |
| Related Documents | `AGENTS.md`, `docs/planning/BACKLOG.md`, `docs/testing/TEST-STRATEGY.md` |

## Roles

| Role | Responsibility |
|---|---|
| Product Owner | 批准 PRD/FR/NFR、范围、隐私语义、付费服务与发布行为。 |
| Lead Agent | 对任务分配、契约一致性、整合、全量验证与最终结论负责。 |
| Subagent | 只在明确边界内研究或修改不重叠文件，不单独改变权威规格。 |
| Reviewer | 以需求、契约、安全和测试证据审查，不以个人偏好替代规格。 |

## Git workflow

1. `main` 保持可解释、可验证；功能工作使用短分支或独立工作树。
2. 一次提交只完成一个逻辑目标；M1 起的实现提交在消息中包含 `TASK-*`，M0 文档基线提交使用明确的文档层级 scope。
3. 提交前运行该任务规定的验证；不提交生成缓存、secret、用户数据或构建产物。
4. 不自动推送、发布、创建远端资源或重写共享历史。
5. 文档基线经用户批准后创建本地 `docs-baseline-v1` 标签；产品代码与标签都不得在批准前产生。

## Task lifecycle

`Blocked -> Ready -> In Progress -> Review -> Done`。同一时刻最多一个任务由主代理标记为 `In Progress`。任务只有在验收条件、测试证据、文档联动与原子提交齐全后才能进入 `Done`。

## Specification changes

- 用户可感知行为：先修改 PRD/FRS/UX。
- 可测质量：先修改 NFRS。
- 接口载荷：先修改 schema 与 API/provider contract。
- 架构取舍：新增或 supersede ADR，并同步当前架构说明。
- 安全或隐私变化：必须同时更新 threat model、privacy lifecycle、测试与风险登记。

发现冲突时不得猜测优先级；在 `docs/INDEX.md` 的权威性矩阵中定位所有者，无法解决则请求 Product Owner。
