# CyberKindred Agent Instructions

| Field | Value |
|---|---|
| Status | Approved |
| Owner | Lead Agent |
| Last Verified | 2026-09-02 |
| Source of Truth For | Codex 根级执行约束 |
| Related Documents | `docs/INDEX.md`, `docs/planning/BACKLOG.md`, `CONTRIBUTING.md` |

## Repository mission

CyberKindred 是 Windows 优先的 AI 陪伴电台。Documentation Baseline v1 是实现前的约束基线；产品代码不得静默偏离已批准文档。

## Required reading order

1. `docs/INDEX.md`
2. `docs/planning/ROADMAP.md` 中当前 Active milestone 与 `docs/planning/BACKLOG.md` 中已认领任务
3. 已认领任务引用的 `FRS`、`NFRS`、UX、架构、ADR 与契约
4. `docs/testing/TEST-STRATEGY.md` 和该 milestone 对应的验收场景

若文档冲突、引用缺失或当前 milestone 没有 checkpoint 条件，停止相关实现并先修正规格。任务级验证是开发自检，不等同于用户验收。

Codex 按项目根到当前目录逐层合并 `AGENTS.md`，更深目录规则优先；本仓库只使用根、`src/`、`src-tauri/`、`tests/` 四份短指令，详细规范通过链接引用。加载语义以 [OpenAI AGENTS.md documentation](https://learn.chatgpt.com/docs/agent-configuration/agents-md) 为准。

## Working agreement

- 同一时刻只推进一个 Active milestone；依赖已满足、写入范围不重叠且工作区状态可解释时，可认领多个 `TASK-*` 并受控并行。
- milestone 内可自主实现、测试和修复；范围扩大、隐私语义变化、付费服务、外部发布、不可逆操作或未批准的高风险生产依赖必须询问用户。
- 行为或契约变更先更新权威文档，再更新实现与相关测试；`TRACEABILITY.md`、Backlog 和 `CHANGELOG.md` 最迟在 milestone checkpoint 前同步，公共契约与安全/隐私语义不得延后同步。
- 使用短分支或工作树与原子提交；不自动推送。子代理除非由主代理明确授予提交所有权，否则不得提交。
- 不覆盖用户的无关更改；不把密钥、令牌、个人对话、绝对用户路径或音乐文件纳入版本控制。
- 独立工作可以受控并行，但同一文件同时只有一个写入负责人。子代理不得单独改变 PRD、FRS、NFRS、ADR 或公共契约。

## Blocker budget

- 为阻塞按 `TASK + operation + stable error/cause` 建立同一 blocker fingerprint。自动排障最多三次有界尝试，并在进度中标为 `attempt 1/3`、`2/3`、`3/3`；每次必须增加新证据或采用不同的安全方案，禁止重复相同命令、轮询不变状态或靠增加 token 重试。
- 登录、凭据输入、系统授权、音频/通知设备操作、人工听感/截图或其他明确只能由用户完成的步骤，在第一次确认后立即归类为 `waiting for user`：只提出一次精确请求，不为凑满三次而继续尝试或反复提醒。
- 非硬门槛的人工验证可登记为 milestone known gap；若实现和快速自检已完成，任务可以标记 `Done` 并继续。硬门槛或实现依赖不能跳过，应把任务标记 `Blocked`，然后继续当前 milestone 中不依赖它的 `Ready` 任务。
- 第三次技术尝试仍失败，或当前只剩等待用户的阻塞时，停止该 blocker 的所有工具调用。若仍有独立工作则继续；否则暂停 `/goal`，汇总 blocker、三次尝试、所需用户动作和恢复条件。用户未响应期间不得轮询或重新启动同一工作。

## Quality gates

- 任务完成前执行 `BACKLOG.md` 中与改动相关的快速自检；任务 `Done` 表示已集成到 milestone candidate，不表示用户已逐任务验收。
- M1–M6 只在 milestone checkpoint 集中执行 `TEST-STRATEGY.md` 的 prototype gate；非关键覆盖率、完整平台矩阵、全状态截图和长稳测试可登记后递延到 M7。
- secret/路径/权限边界、日志脱敏、用户未确认时不出声且不触发付费调用、公共契约校验、数据损坏风险和 Critical/High 安全问题始终是硬门槛，不得以 prototype 为由豁免。
- M7 beta candidate 执行 `BACKLOG.md`、`TEST-STRATEGY.md` 与 `ACCEPTANCE-TESTS.md` 列出的完整发布验证。
- Rust/TypeScript 类型必须与 `docs/contracts/schemas/` 和 `API-CONTRACT.md` 保持一致。
- 默认测试必须 hermetic；真实 OpenAI、Apple Music、MusicBrainz 或天气测试必须显式标记并可跳过。
- 运行代码不得泄露 secret，不得通过 WebView 直接访问 secret，日志必须脱敏。
- milestone checkpoint 前不得留下未解释的 `TODO`、`TBD`、跳过的硬门槛测试或未追踪需求；其他递延验证必须进入 checkpoint 的 known gaps。

## Code review rules

- 标记任何绕过 provider/source capability、状态机、数据保留规则或契约校验的修改。
- 标记任何把 API Key、对话正文或文件系统权限扩大到前端的修改。
- 标记任何在用户未确认时自动播放声音、自动采集屏幕/麦克风或操控 Apple Music Web DOM 的修改。
