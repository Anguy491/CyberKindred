# ADR-0004: 以受限 Provider 抽象隔离外部 AI 与数据服务

| Metadata | Value |
|---|---|
| Status | Draft |
| Owner | Integration Architecture |
| Last Verified | 2026-09-01 |
| Source of Truth For | `LLMProvider`、`TTSProvider`、`MetadataProvider`、`WeatherProvider` 抽象与 adapter 边界的决策理由 |
| Related Documents | [Architecture](../ARCHITECTURE.md), [AI Orchestration](../AI-ORCHESTRATION.md), [Provider Contracts](../../contracts/PROVIDER-CONTRACTS.md), [Dependency Policy](../DEPENDENCY-POLICY.md), [External Integrations](../../integrations/EXTERNAL-INTEGRATIONS.md) |

## Context

v1 采用 OpenAI Responses/Audio Speech、MusicBrainz/Cover Art Archive 和 Open-Meteo。这些服务的认证、限流、错误、可用性和数据处理不同，且未来可能替换。若业务层直接依赖 HTTP/provider SDK，测试会触网，错误和隐私规则散落，provider 变更会侵入播放与 UI。

## Decision

在 Rust domain/application 边界定义 typed `LLMProvider`、`TTSProvider`、`MetadataProvider`、`WeatherProvider`；infrastructure 提供具体 HTTP adapter 与 hermetic fake。contract 规定输入、输出、capability、timeout/cancellation 和统一错误分类。

Provider adapter 负责协议、认证注入、HTTP timeout、rate limit、Retry-After、response size/MIME/schema 初检和日志脱敏；application service 负责用例 retry policy、cache、领域校验、持久化与降级。Provider 永不直接写 SQLite、发 Tauri event、控制播放器或批准记忆。v1 OpenAI adapter 以 reqwest 实现最小 API surface，不采用通用 SDK。

## Alternatives considered

- **业务代码直接调用 HTTP**：文件少，但横切的安全、错误和测试逻辑会重复且难以审查。
- **采用单一“万能 AI Provider”接口**：表面统一，但 LLM structured output 与 TTS binary stream 的能力/失败语义不同，会形成弱类型接口。
- **直接引入官方/社区 SDK**：可减少协议代码，但不保证能统一强制 `store:false`、custom HTTP/redaction、最小 feature 与稳定测试；当前 surface 足够小。
- **微服务代理所有 provider**：能集中策略，但 v1 增加后端、部署、账号和隐私边界。

## Consequences

- 单元/集成测试可完全 hermetic，业务只依赖稳定 typed contract。
- 新 provider 必须明确 capability 与错误映射，不能用最低公分母伪装等价能力。
- 会有少量 adapter boilerplate；contract 和 schema 变更必须同步 Rust/TypeScript/testing/traceability。
- Provider-specific response/raw body 不得越过 adapter；诊断依赖 correlation ID、分类错误和用量元数据。
- 新外部数据目的地或高风险 SDK 仍需用户批准，接口可插拔并不构成自动授权。
