# ADR-0007: Milestone-based prototype delivery

| Metadata | Value |
|---|---|
| Status | Approved |
| Owner | Product Owner / Engineering Lead |
| Last Verified | 2026-09-02 |
| Source of Truth For | milestone 验收、原型门槛、任务并行与质量债务递延的决策理由 |
| Related Documents | [Roadmap](../../planning/ROADMAP.md), [Backlog](../../planning/BACKLOG.md), [Test Strategy](../../testing/TEST-STRATEGY.md), [Acceptance Tests](../../testing/ACCEPTANCE-TESTS.md), [ADR-0006](ADR-0006-document-and-contract-driven-development.md), [Repository AGENTS](../../../AGENTS.md) |

## Context

Documentation Baseline v1 把任务拆得足够细，但逐任务等待完整自动化、人工证据和用户签字会让探针与早期产品切片频繁停顿。当前目标是尽快获得一个可运行、可体验、可持续迭代的 Windows 原型；完整 beta 质量仍然重要，但不需要在每个内部实现切片结束时重复证明。

同时，CyberKindred 接触 API Key、本地音乐目录、对话与记忆、外部 provider、Windows 媒体会话和声音播放。原型阶段不能用速度为理由弱化会造成隐私泄露、越权访问、意外计费、未经确认出声、数据损坏或公共契约漂移的边界。

## Decision

- M1–M6 以 milestone checkpoint 为转场单位，而不要求 Product Owner 在线验收。任务是内部计划与交付切片；`Done` 只表示已集成并完成适当自检。
- 同一时刻只有一个 Active milestone。milestone 内依赖满足、文件写入不重叠的任务可受控并行；checkpoint 文档记录为 `Passed` 或 `Passed with known gaps` 后自动激活下一 milestone，`Blocked` 才停止转场。
- M1–M2 使用 technical checkpoint，M3–M6 使用 prototype checkpoint，M7 使用 beta release gate。M3 是首个面向使用的本地电台原型。
- M1–M6 每个 checkpoint 只证明 Roadmap 主路径、一个关键失败或降级路径和始终适用的 hard gates。覆盖率、完整平台/缩放矩阵、全状态截图、Narrator 全流程、大曲库压力、重复延迟采样和长 soak 可记录为 known gaps 并递延到 M7。
- hard gates 不可递延：secret 不进入 WebView、日志或制品；文件、数据库、凭据和网络权限不越界；声音与可能计费动作必须由用户明确触发；公共 schema/契约改动必须验证；不得存在已知数据损坏路径或 open Critical/High security finding。
- checkpoint 必须如实列出 candidate commit、环境、完成范围、执行结果、失败与未执行检查、影响、规避方式、目标 milestone 和结论。Lead Agent 可在 hard gates 通过时记录 `Passed with known gaps` 并继续，但失败或未执行不能改写成通过。
- 同一技术 blocker 最多进行三次有差异、有界且能增加证据的尝试；第三次失败后停止重试并转向独立工作。已确认只能由用户完成的授权、登录、设备操作或人工判断只请求一次，不为达到次数继续消耗资源。无独立工作可做时暂停 Goal，等待用户后再从明确恢复条件继续。
- 行为、公共契约与安全/隐私语义继续遵循 ADR-0006 的文档先行和同步更新要求。Backlog 状态、非契约 Traceability 整理与 Changelog 可在同一 milestone 内批量维护，但 checkpoint 前必须一致。
- M7 beta candidate 恢复完整 P0/P1、覆盖率、兼容性、无障碍、性能、恢复、安装与证据门槛。外部发布、签名、上传或扩大分发仍需单独授权。

## Alternatives considered

- **继续逐 task 完整验收**：局部证据最细，但等待与重复成本过高，延迟可用原型。
- **取消全部门禁直到功能完成**：短期最快，却会让 secret、路径、计费、声音和契约问题在集成后才暴露，返工和用户风险不可接受。
- **只在 M7 做一次测试**：减少中间成本，但无法及时确认 milestone 主路径是否真实可用，也难以定位回归来自哪个阶段。

## Consequences

- M1–M6 不再因用户离线停在 checkpoint；完成记录成为可审计的异步验收材料，开发可跨 milestone 连续推进到目标停止条件。
- 早期 checkpoint 代表“原型可用/技术路径成立”，不代表满足 beta 发布质量；状态展示和沟通必须保留这一区别。
- 自动化和质量工作不会消失，而是按风险前置、按完整性后置。M7 可能集中暴露技术债，因此每个 checkpoint 必须维护可执行的 known gaps，而不能只写模糊备注。
- 重复失败不会形成无限重试循环；代价是某些任务会更早暴露为 `Blocked`，需要用户回来后根据精确的恢复条件继续。
- 安全、隐私、契约与数据完整性仍即时验证，降低快速迭代对用户设备和账户造成不可逆影响的概率。
