# Changelog

| Field | Value |
|---|---|
| Status | Approved |
| Owner | Release Steward |
| Last Verified | 2026-09-03 |
| Source of Truth For | 用户可感知变更与破坏性契约变更历史 |
| Related Documents | `docs/operations/BUILD-RELEASE.md`, `docs/planning/BACKLOG.md` |

所有用户可感知行为、数据迁移和破坏性契约变化记录于此。版本号遵循 Semantic Versioning；内测前使用 `0.x.y`。

## [Unreleased]

### Changed

- 补齐 API-015 `TrackView` 的精确 nullable 标签、MusicBrainz `fetchedAt` 与持久化 match-status 映射，使离线缓存来源/时间验收可由 WebView 在不接收路径的情况下完成。
- 对齐 onboarding 的扫描/TTS 所有权：首次目录授权只显示名称与待扫描状态，文件计数由 M3 API-013 扫描后提供；不可预览声音在 M2 fail closed，真实 `[PLAYING]`/再次点击停止与 API-038 voice-preview cancel slice 由 M3 `TASK-017` 一并交付。
- API-002/API-003 onboarding contract 改为严格、revisioned 的七步状态与判别 step submission：每步可保存/恢复和返回编辑，profile/完成前缀原子持久化，目录/provider/城市/日程仍由各专用 API 保持唯一权威；这修复了旧版仅能最终保存 `privacyAccepted: true`、无法满足逐步重启恢复的契约缺口。
- Product Owner 已解决 M2 三项公共语义：切换 provider origin 保留各 origin credential 直到 API-005/full reset；operation 权威 terminal 唯一但 transport 至少一次，前端按 `operationId` 幂等；API-008 在 model ID 实际变化时以 60 秒 pre-save Responses capability probe 门控整份 patch 的原子保存，API-004 仍以只读 `/v1/models` 最小验证候选 credential。
- M2 technical checkpoint 如实保留 2026-09-02 的 `Blocked` 历史结果；三项语义 remediation、`TASK-010` 与完整 hard-gate 复验现已完成，M2 于 2026-09-03 以 `Passed with known gaps` 关闭并激活 M3 `TASK-011`。
- 对齐 SQLx 0.9 的实际 feature 名称：SQLite 基础只启用 bundled SQLite、Tokio runtime、migration/macro，不启用默认的多数据库、JSON 或 load-extension 能力。
- 补全 `playback-state` 与 `program-plan` 条件分支中的局部 `object`/`array` 类型声明，使不改变实例语义的 v1 schema 可由 AJV strict 模式编译。
- 为 `TASK-005` 固定 Node-only `ajv 8.20.0` 与 `ajv-formats 3.0.1`，以执行 Draft 2020-12/UUID/date-time 契约校验；两者禁止进入 WebView runtime bundle。
- M1 technical checkpoint 记录为 `Passed with known gaps` 并自动激活 M2；真实听感/双设备、通知/托盘/登录、NSIS 和完整跨版本矩阵保持 `Not Run`，未改写为通过。
- 补齐 M2 脚手架所需的 `tauri-build`、React/Node types、coverage、lint 与 Cargo CI tool exact pins；避免 TypeScript 7 与不兼容的 typescript-eslint peer range。
- Documentation Baseline v1 于 2026-09-02 获用户批准，项目进入 M1 技术探针阶段。
- 开发治理改为逐 milestone 验收：M1–M6 使用轻量 technical/prototype checkpoint，任务仅作为可并行的内部交付切片；覆盖率、完整兼容/无障碍/性能/长稳矩阵可记录后递延到 M7，secret、权限、日志、用户确认、付费调用、数据完整性、公共契约和 Critical/High 安全问题仍是即时硬门槛。
- `TASK-003` 仅依赖已批准的文档基线且与 `TASK-002` 写入范围不重叠，按新的 milestone 并行规则从 `Blocked` 调整为 `Ready`。
- `TASK-002` 的 `Manual-TASK-002` 人工清单保留为 M1 checkpoint 证据，不再单独触发任务级用户签字；产品阶段的自动兼容性、性能与恢复验收不变。
- 阻塞处理增加 token budget：自动排障最多三次有差异尝试；明确需要用户介入的步骤只请求一次并等待；达到上限后转向独立任务或暂停 Goal，禁止轮询不变状态。
- M1–M6 checkpoint 改为文档记录即转场：hard gates 通过时由 Lead Agent 记录 `Passed` 或 `Passed with known gaps` 并立即开始下一 milestone，不等待 Product Owner 在线接受；M7 beta/release 仍保留人工批准。

### Added

- `TASK-016` 交付 OpenAI Responses ProgramPlan provider、按当前 origin 精确读取的 BYOK adapter、strict structured output、无 tools/不存储请求、24k/4k 预算、有序脱敏 context、60 秒取消/deadline 与结构化无效恰好一次修复；无 credential 或 provider 不可用时由 `TASK-015` 确定性计划降级。
- `TASK-015` 交付 path-free 本地候选评分、重复冷却与明确 scarcity degradation、最多 200 个真实 UUIDv7 候选、双层 ProgramPlan 校验和确定性文字/六首队列 fallback；节目计划及 segment 以事务写入 SQLite，崩溃恢复不会留下可误恢复的活跃节目。
- `TASK-012`–`TASK-014` 交付 path-free 分页曲库与扫描 UI、严格 MusicBrainz/CAA 最小披露/限流/缓存 provider，以及 API-016–API-023/EVT-001 本地播放 actor；生产解码保持 direct Symphonia 0.6 → rodio output 边界，队列预置与启动恢复默认静音，只有明确用户动作才打开设备并播放。
- `TASK-011` 交付授权目录内的六格式本地扫描、坏文件隔离、增量/移动识别、碰撞容忍的文件身份匹配、可取消进度、事务提交与可重放 terminal outbox；扫描不复制音频，事件与错误不携带本地路径。
- `TASK-010` 交付 API-002/API-003 七步可恢复 onboarding、API-010–API-012 原生目录授权边界、路径与 revision 完整性校验、静音/本地文字降级、双隐私确认 gate，以及 Windows 桌面首次启动到 RADIO 的自动化路径；Key、目录 picker 与声音预览都只在用户显式点击后触发。
- `TASK-008` 交付 Rust-only provider registry/config、严格 API-004..009/API-043 边界、Windows Credential Manager 内部 unsafe 隔离、显式 Responses capability probe、候选 usage 与设置事务原子绑定、唯一权威 accepted→terminal outbox、at-least-once transport/frontend dedupe、启动恢复、24 小时/7 天 outbox retention 和脱敏诊断；默认测试不联网、不出声、不触碰真实凭据。
- 移除含三项 High advisory 的 WebdriverIO 9.31.5 开发依赖链，改用 Node 24 built-in W3C client 直连固定 `tauri-driver`；9 条 hermetic harness 测试与匹配 Microsoft EdgeDriver 的隔离 desktop shell/navigation smoke 通过。
- `TASK-009` 交付 RADIO/LIBRARY/YOU/SETTINGS 四页静音应用壳、OLED 三层视觉 token、本地字体/fallback 状态、全局键盘导航与可聚焦的 capability 降级原因；四页 1100×720 截图证据已归档。
- M2 新增 Rust-only 强类型诊断日志：只写入 allowlist 字段与固定 `[redacted]` JSONL，以 5 MiB 单文件、14 天和 50 MiB 总量上限轮换清理，并拒绝 symlink/Windows reparse 路径。
- `TASK-007` 建立 29 表 SQLite 初始迁移、单写者事务仓储、迁移前校验备份、数据库身份/完整性 fail-closed、30 天原文/voice 清理、路径 containment 与按 canonical origin 隔离的 Windows Credential Vault；默认测试不触碰真实凭据。
- `TASK-006` 建立 API-001 Tauri 注册、统一安全 `ApiError`、deadline/幂等/revision/全局事件重同步原语，以及唯一可注入的严格 TypeScript IPC transport；WebView 静态审计拒绝直接文件、数据库、凭据和 provider 网络访问。
- 补齐 M2 已批准的 Tauri dialog/notification/autostart 前端绑定及 Playwright、axe 测试依赖；安装期只允许 `esbuild` 本机构建检查，desktop driver 不通过 npm 安装脚本下载。
- 为 M2 应用壳本地打包 Doto、Space Grotesk 与 Space Mono，固定 Google Fonts source commit、逐文件 SHA-256 和完整 OFL-1.1 文本；运行时不访问字体 CDN。
- M2 固定并校验 `cargo-deny 0.20.2` 与 `cargo-audit 0.22.2` 本机工具，新增 Windows 目标的 registry/license/advisory/wildcard gate；无漏洞，Tauri 传递链的停止维护告警保留为可见风险。
- `TASK-005` 交付六份 schema 的可复现 Rust/TypeScript 类型生成、canonical digest/baseline 漂移门槛、Node AJV 与 Rust offline registry 的同源 18 fixture parity，以及验证错误不回显提交内容的 canary 测试。
- `TASK-004` 建立可启动的静音 Tauri/React/Rust 空壳、exact tool/dependency pins、双 lockfile、严格 CSP/最小 capability、基础测试/coverage、locked Rust 门槛与 Windows CI；本机 debug executable 启动并显示 `CyberKindred` 窗口。
- `TASK-003` Windows 标准用户探针完成：Windows Password Vault canary 写/读/删、显式 HKCU 自启动启停、静音通知 action 模型、精确 reset、WebView2/toolchain preflight 与去标识 evidence 均通过自动/本机 smoke；真实 toast、tray、登录周期、NSIS 与 Windows 10 VM 留作已记录 known gaps。
- `ADR-0007` 记录 milestone-based prototype delivery、单 milestone 多任务受控并行、checkpoint evidence 与 known-gap 规则。
- `TASK-002` 本地音频探针实现已标记 `Done`：含六类 CC0 生成 fixture 与 SHA-256 清单、Symphonia 0.6.1 全量解码报告、lofty 标签/封面报告、rodio 交互播放/暂停/seek、默认设备重建后保持暂停、损坏文件隔离、脱敏事件与 CPU/内存采样手册。人工证据在 M1 checkpoint 与其他探针结果一并审阅。
- `TASK-001` 只读 GSMTC 技术探针已由用户验收完成：可枚举 Windows 媒体会话、元数据字段存在性、时间线与实时 capability；watch 输出自动移除媒体正文并过滤 timeline 心跳。Apple Music 空闲、播放、App 关闭、Web-only 隔离与会话消失场景通过，M1 输入到探针收敛矩阵 50/50 在 2 秒内；产品 UI 延迟验收仍由 M6 `TASK-025` 承担。
- Documentation Baseline v1：产品需求、可测量非功能需求、AI 行为与 Nothing Design 文字化 UX 规范。
- Tauri/React/Rust 架构、运行时状态机、AI 编排、SQLite 数据模型、依赖政策及六项初始 ADR。
- 版本化 Tauri IPC、provider contracts、六份 JSON Schema 及合法、边界、非法示例。
- Apple Music Windows App/GSMTC、本地音乐、OpenAI、MusicBrainz、Cover Art Archive 与 Open-Meteo 集成边界。
- 威胁模型、隐私生命周期、许可证、安全与发布治理基线。
- 测试策略、验收场景、双向追踪矩阵、里程碑、原子 Backlog、风险登记和运行手册。
- 根目录与 `src`、`src-tauri`、`tests` 分层 `AGENTS.md`，以及离线文档一致性验证脚本。
