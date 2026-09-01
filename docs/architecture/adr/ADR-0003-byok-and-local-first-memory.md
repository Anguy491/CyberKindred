# ADR-0003: BYOK 与 local-first、审批式长期记忆

| Metadata | Value |
|---|---|
| Status | Draft |
| Owner | Privacy & Data Architecture |
| Last Verified | 2026-09-01 |
| Source of Truth For | 用户自带 OpenAI Key、Credential Manager 存储、本地数据与记忆审批模型的决策理由 |
| Related Documents | [Data Model](../DATA-MODEL.md), [AI Orchestration](../AI-ORCHESTRATION.md), [Privacy Data Lifecycle](../../security/PRIVACY-DATA-LIFECYCLE.md), [Threat Model](../../security/THREAT-MODEL.md), [FRS](../../product/FRS.md) |

## Context

CyberKindred 需要长期了解用户，同时 v1 没有账号后端或代付 AI 服务。陪伴内容、作息、曲库与反馈具有隐私敏感性；若模型自动把推断写成长期事实，用户难以理解和纠正。Windows 单用户内测允许先采用本地数据所有权和用户自带 Key。

## Decision

- OpenAI 采用 BYOK。API Key 只保存于 Windows Credential Manager，由 Rust 在请求时短暂读取；WebView、SQLite、日志与导出均不可获得 secret。
- profile、设置、曲库索引、播放事实、对话、摘要和记忆保存在本地 SQLite，不建设云端账号或同步。
- OpenAI Responses 请求强制 `store:false`；只发送当前用例需要的最小 context。
- 原始对话创建时写入 30 天固定过期时间；长期保留 session summary、反馈事实与用户批准的 memory。
- memory 必须经过 `proposal → user approval → active`；用户可以编辑、禁用、删除与导出。未经批准的 proposal 不进入模型 context。
- 不自动建立敏感属性记忆。全部重置同时删除本地数据/cache/log 和 Credential item，不影响音乐原文件。

## Alternatives considered

- **项目托管 API Key/后端代理**：可简化引导和成本控制，但需要账号、计费、服务端安全、隐私政策与持续运维。
- **把 Key 存 SQLite/.env/localStorage**：实现简单但扩大 secret 暴露、日志/备份/前端泄露风险。
- **完全不保存历史**：隐私最强，但无法实现“了解与陪伴”的核心价值。
- **模型自动写长期记忆**：体验更顺滑，但推断错误、敏感画像和不可见持久化风险不可接受。
- **保存完整永久对话**：可提供更多上下文，但与数据最小化和可控保留冲突。

## Consequences

- 用户承担 API 账号与用量；引导必须测试 Key，并清楚说明发送数据与成本。
- 设备迁移不会自动带走记忆；导出/导入如后续加入，必须有显式用户动作和 schema。
- 必须实现 30 天清理、审批 UI、来源追踪、删除传播、Credential probe 与脱敏测试。
- `store:false` 不等于“不向 OpenAI 发送数据”；Privacy 文档仍必须逐项列明所发送 context。
- 未来托管服务或同步属于产品、隐私、威胁模型和数据迁移的重大改变，需用户批准与新 ADR。
