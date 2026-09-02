# Risk Register

| Field | Value |
|---|---|
| Status | Approved |
| Owner | Lead Agent |
| Last Verified | 2026-09-02 |
| Source of Truth For | 已知项目风险、触发条件、缓解和关闭标准 |
| Related Documents | `ROADMAP.md`, `../security/THREAT-MODEL.md`, `../integrations/EXTERNAL-INTEGRATIONS.md` |

概率与影响使用 `Low / Medium / High`；风险只能在关闭标准有证据时标记 `Closed`。

| ID | Risk | P | I | Trigger | Mitigation / fallback | Owner | Status |
|---|---|---:|---:|---|---|---|---|
| RISK-001 | Apple Music Windows App 未暴露预期 GSMTC capability 或本机反应体验过弱 | M | H | 实机枚举不到会话/元数据/控制，或用户无法接受 track-aware 反应仅本机文字 | M1 同时验证 packaged/unpackaged；按 runtime capability 隐藏；反应使用本机确定性文字且 GSMTC 数据不外发；保留本地源完整 AI/TTS 主链路 | Core/Product | Open |
| RISK-002 | TTS pause/resume 与用户操作竞态造成意外恢复 | M | H | 用户在串场中手动暂停/切源后应用仍恢复 | 使用 session id、command generation 与 ownership token；只恢复由本应用暂停且状态未变化的同一会话 | Core | Open |
| RISK-003 | 音频格式、损坏文件或设备切换中断节目 | H | M | decoder error、default output 变化、睡眠唤醒 | 每曲预检、错误跳过、设备重建、队列 checkpoint；常见格式 fixtures 和实机测试 | Core/QA | Open |
| RISK-004 | LLM 生成不存在 track id 或非法 ProgramPlan | M | H | schema/semantic validation 失败 | Structured Outputs、候选 allowlist、Rust 二次校验、一次修复重试、确定性离线队列 | AI/Core | Open |
| RISK-005 | 模型/声音别名变化或兼容 Base URL 不支持 Responses | M | M | connection test 或模型请求失败 | model id 可配置、capability probe、OpenAI adapter 与兼容 adapter 分离、错误不阻断音乐 | AI | Open |
| RISK-006 | BYOK secret 泄露或自定义 origin 凭据残留 | L | H | secret 出现在 IPC/日志/SQLite/fixture，或切换 origin 后旧条目无法清除 | Credential 按 normalized origin hash 隔离、API 显式 origin 删除、全部重置枚举全部项目条目、Rust-only networking 与 canary tests | Security | Open |
| RISK-007 | MusicBrainz 错误匹配覆盖用户标签或触发封禁 | M | M | 低置信度自动覆盖或 >1 request/s | 保留原始标签、阈值/人工状态、全局 1 rps、User-Agent、永久缓存、429 backoff | Metadata | Open |
| RISK-008 | 外部天气/元数据不可用、城市查询披露或条款变化 | H | M | timeout、offline、quota/terms change，或 Geocoding query 超出用户显式动作 | 上下文可选、搜索手势门控/字段白名单/10分钟内存候选、缓存、短超时、provider kill switch；无天气/无增强正常运行 | Integrations | Open |
| RISK-009 | 长期记忆/摘要错误、越界或删除后复活 | M | H | 未批准事实进入 prompt，或删除后仍可检索/由清理任务重建 | proposal/approved 分离、来源状态、summary deletion tombstone、级联清除敏感 outbox、重启与导出验证 | AI/Data | Open |
| RISK-010 | 大曲库扫描阻塞 UI 或占用过多资源 | M | M | 10,000 首扫描无响应、内存/CPU 超阈值 | 后台有界并发、增量事务、进度/取消、mtime/hash 去重、性能预算 | Library | Open |
| RISK-011 | 未签名 NSIS 触发 SmartScreen，阻碍内测 | H | M | 测试者无法辨识或安装 | 校验和、来源说明、最小测试群；公开发布前取得代码签名，不绕过系统安全 | Release | Open |
| RISK-012 | 第三方字体、音频库或服务许可证不适合分发/商业化 | M | H | 许可证扫描或条款审查不通过 | Dependency Policy、NOTICE/SBOM、非商业内测限定；商业化前法律复核 | Legal | Open |
| RISK-013 | 文档与实现漂移导致自主开发错误 | M | H | contract/traceability 检查失败、双向映射不对称或行为无 FR | docs-first change coupling、schema tests、ID/link lint、Requirement↔Test/Task 双向检查、每任务文档联动门槛 | Lead | Open |
| RISK-014 | Windows/WebView2 版本差异导致 UI 或 IPC 行为不同 | M | M | 支持矩阵设备出现启动/渲染失败 | 固定最低 Windows、bootstrapper 检测 WebView2、至少 Win10/Win11 两环境验收 | Release/QA | Open |
| RISK-015 | WebdriverIO 9.31.5 的传递开发依赖存在未缓解的 High advisory | M | H | 完整 `pnpm audit --audit-level high` 在 `extract-zip 2.0.1`、`deepmerge-ts 7.1.6` 与 `serialize-javascript 6.0.2` 报警；其中审计建议的 `extract-zip >=2.0.2` 在 registry 尚不可安装 | 在 M2 checkpoint 前移除该 desktop E2E 依赖链或升级到已验证且无 High 的组合；未关闭时 M2 只能 `Blocked`，Playwright 仅作前端流程证据不冒充 desktop E2E | Lead/QA | Open |
| RISK-016 | authoritative credential 生命周期语义冲突 | M | H | `DATA-MODEL.md` 要求切换 origin 后删除旧 credential，`PRIVACY-DATA-LIFECYCLE.md` 要求保留各 origin credential 直到 API-005/full reset | Product Owner 选择唯一语义后先同步两份权威文档，再修改 API-004 实现和测试；决定前冻结切换提交，M2 只能 `Blocked` | Product/Security | Open |
| RISK-017 | operation terminal event 无法在当前 Tauri bus/outbox 协议下证明 delivery exactly-once | M | H | emit 成功而 delivered 标记前崩溃会跨进程重放；先标记再 emit 则可能永久丢失 | Product Owner 选择 durable consumer ack/dedup，或把公共契约改为 authoritative terminal exactly-one + transport at-least-once 并要求 UI 以 operationId 幂等；决定前不宣称 EVT-008/009 hard gate 通过 | Core/Product | Open |
| RISK-018 | model ID 保存前 capability probe 与 API-006/API-008 契约流程不相容 | M | M | API-008 可在 5 秒同步保存任意有效文本 model ID，API-006 只能测试已保存 model；无法满足 `AI-ORCHESTRATION.md` 的保存前 probe | Product Owner 明确由 API-008 在 model patch 时执行 60 秒 probe，或扩展候选 model probe/attestation 契约；先文档后实现 | AI/Product | Open |

## M1 observations

- `RISK-001` remains `Open`: the recorded Apple Music Windows App probe established truthful GSMTC discovery/capability convergence on one Windows 11 environment, but packaged/unpackaged and multi-version coverage remains for M6/M7.
- `RISK-003` remains `Open`: six fixed-format decode fixtures and corrupt-file isolation passed, while real audible controls, dual-device switching and product recovery remain `Not Run` for M3/M7.
- `RISK-006` remains `Open`: a standard-user Password Vault canary write/read/delete cycle passed with zero secret output fields and exact cleanup; canonical origin isolation plus SQLite/log/export scans remain M2/M6 work.
- `RISK-014` remains `Open`: WebView2 was detected on the M1 Windows 11 host, but Windows 10, packaged Tauri and NSIS installation matrices remain `Not Run` for M2/M7.

## M2 observations

- The M2 blocked checkpoint is recorded in [`M2.md`](../testing/checkpoints/M2.md). Hermetic frontend/Rust suites, strict clippy, Cargo license/source/advisory policy and RustSec vulnerability checks passed, but hard gates remain failed by `RISK-015`, `RISK-016` and `RISK-017`.
- The provider worktree now persists API-009 acceptance before spawning, atomically converts accepted operations to one authoritative terminal, recovers bounded batches without a 100-row tail, expires delivered/undelivered outbox records at 24-hour/7-day boundaries, and serializes credential mutations. These reduce the implementation risk but do not resolve `RISK-017` transport semantics.
- `RISK-018` blocks completion of the model-setting flow, not the fixed hermetic Responses probe itself. The probe uses `store:false`, no tools, strict JSON Schema and explicit `reasoning.effort=none`; no real provider request was made.
