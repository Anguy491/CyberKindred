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
2. `docs/planning/BACKLOG.md` 中当前唯一的 `In Progress` 任务
3. 该任务引用的 `FRS`、`NFRS`、UX、架构、ADR 与契约
4. `docs/testing/TEST-STRATEGY.md` 和对应验收场景

若文档冲突、引用缺失或任务没有验收条件，停止相关实现并先修正规格。

Codex 按项目根到当前目录逐层合并 `AGENTS.md`，更深目录规则优先；本仓库只使用根、`src/`、`src-tauri/`、`tests/` 四份短指令，详细规范通过链接引用。加载语义以 [OpenAI AGENTS.md documentation](https://learn.chatgpt.com/docs/agent-configuration/agents-md) 为准。

## Working agreement

- 一次只认领一个 `TASK-*`，先确认依赖完成且工作区状态可解释。
- 里程碑内可自主实现、测试和修复；范围扩大、隐私语义变化、付费服务、外部发布、不可逆操作或未批准的高风险生产依赖必须询问用户。
- 行为或契约变更先更新权威文档，再更新实现、测试、`TRACEABILITY.md`、Backlog 和 `CHANGELOG.md`。
- 使用短分支或工作树与原子提交；不自动推送。子代理除非由主代理明确授予提交所有权，否则不得提交。
- 不覆盖用户的无关更改；不把密钥、令牌、个人对话、绝对用户路径或音乐文件纳入版本控制。
- 独立工作可以受控并行，但同一文件同时只有一个写入负责人。子代理不得单独改变 PRD、FRS、NFRS、ADR 或公共契约。

## Quality gates

- 执行当前任务在 `BACKLOG.md` 与 `TEST-STRATEGY.md` 中列出的全部验证。
- Rust/TypeScript 类型必须与 `docs/contracts/schemas/` 和 `API-CONTRACT.md` 保持一致。
- 默认测试必须 hermetic；真实 OpenAI、Apple Music、MusicBrainz 或天气测试必须显式标记并可跳过。
- 运行代码不得泄露 secret，不得通过 WebView 直接访问 secret，日志必须脱敏。
- 完成任务前不得留下未解释的 `TODO`、`TBD`、跳过测试或未追踪需求。

## Code review rules

- 标记任何绕过 provider/source capability、状态机、数据保留规则或契约校验的修改。
- 标记任何把 API Key、对话正文或文件系统权限扩大到前端的修改。
- 标记任何在用户未确认时自动播放声音、自动采集屏幕/麦克风或操控 Apple Music Web DOM 的修改。
