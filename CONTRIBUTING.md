# Contributing to CyberKindred

| Field | Value |
|---|---|
| Status | Approved |
| Owner | Lead Agent |
| Last Verified | 2026-09-02 |
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
2. 一次提交只完成一个逻辑目标；M1 起的实现提交在消息中包含 `TASK-*`，同一 milestone 可包含多个短提交，不要求每个 task 单独等待用户验收。
3. 提交前运行与改动相关的快速自检；完整场景与证据在 milestone checkpoint 集中执行。不提交生成缓存、secret、用户数据或构建产物。
4. 不自动推送、发布、创建远端资源或重写共享历史。
5. 文档基线经用户批准后创建本地 `docs-baseline-v1` 标签；产品代码与标签都不得在批准前产生。

## Task lifecycle

`Blocked -> Ready -> In Progress -> Review -> Done`。同一时刻只允许一个 Active milestone；其中依赖已满足且文件所有权不冲突的任务可以并行。`Review` 是代码/文档审查，不是用户签字等待区；`Done` 表示实现已集成、快速自检通过或例外已登记，可进入 milestone candidate。用户验收只发生在 milestone checkpoint。

milestone checkpoint 提供一个可运行或可演示的 candidate、一次聚合验收记录、已完成范围和 known gaps。Product Owner 可选择接受、带已登记债务接受或退回；接受后才启动下一 milestone。M1–M6 的非关键测试债务可递延到 M7，硬门槛不得递延。

## Blocker handling

同一技术 blocker 最多进行三次能产生新信息的尝试，并明确记录 `1/3`、`2/3`、`3/3`。第三次失败后停止重试：非硬门槛转为 known gap，硬门槛/依赖任务转为 `Blocked`，再继续不依赖它的工作。已经确认只能由用户完成的授权、登录、设备操作或人工验收只请求一次并立即等待，不执行三次无意义重试；没有其他可推进工作时暂停 Goal，用户回来后从记录的恢复条件继续。

## Specification changes

- 用户可感知行为：先修改 PRD/FRS/UX。
- 可测质量：先修改 NFRS。
- 接口载荷：先修改 schema 与 API/provider contract。
- 架构取舍：新增或 supersede ADR，并同步当前架构说明。
- 安全或隐私变化：必须同时更新 threat model、privacy lifecycle、测试与风险登记。

发现冲突时不得猜测优先级；在 `docs/INDEX.md` 的权威性矩阵中定位所有者，无法解决则请求 Product Owner。
