# Roadmap

| Field | Value |
|---|---|
| Status | Approved |
| Owner | Product Owner / Lead Agent |
| Last Verified | 2026-09-03 |
| Source of Truth For | 里程碑顺序、依赖与阶段退出条件 |
| Related Documents | `../product/PRD.md`, `BACKLOG.md`, `RISK-REGISTER.md`, `../testing/TEST-STRATEGY.md`, `../architecture/adr/ADR-0007-milestone-prototype-delivery.md` |

Roadmap 描述阶段、checkpoint 结论和自动转场，不承载具体任务状态。具体执行顺序以 `BACKLOG.md` 为准。

## Delivery cadence

- 同一时刻只有一个 Active milestone；milestone 内任务是可并行的实现切片，不是独立验收点。
- M1–M2 使用 `Technical checkpoint`：证明关键路径可行、形成可运行基础，并明确剩余风险。
- M3–M6 使用 `Prototype checkpoint`：在主要开发机上演示该 milestone 的端到端主路径。M3 是首个“可用原型”，后续 checkpoint 逐步扩展能力。
- M7 使用 `Beta release gate`：执行全部 P0/P1 需求、完整默认测试、覆盖率、兼容性、无障碍、性能、恢复、安装和证据要求。
- M1–M6 checkpoint 记录为 `Passed`、`Passed with known gaps` 或 `Blocked`。前两者在文档同步完成后自动激活下一 milestone，不等待人工接受；known gap 必须包含影响、规避方式和目标 milestone。安全/隐私硬门槛、公共契约有效性、数据损坏、未经确认的出声或付费调用不可递延，失败时结论只能是 `Blocked`。

## M0 — Documentation Baseline v1

**Status:** Complete — approved 2026-09-02.

**Goal:** 在产品代码前建立可审阅、可追踪、机器可验证的事实源。

**Exit criteria:**

- 全部计划文档、schema、examples 和 AGENTS 存在且通过文档质量检查。
- 所有 FR/NFR 都映射架构、契约或明确的非契约组件、测试与 Task。
- 用户明确批准基线；本地创建 `docs-baseline-v1` 标签。

## M1 — Feasibility and platform probes

**Status:** Complete — `Passed with known gaps` on 2026-09-02; see [`M1 checkpoint`](../testing/checkpoints/M1.md).

**Goal:** 用最小可丢弃探针消除 Windows 媒体与音频的高风险未知数，不形成产品 UI。

**Scope:** packaged/unpackaged GSMTC 枚举与 Apple Music 控制、rodio 常用格式/设备切换、Credential Manager、Tauri notification/tray/NSIS/WebView2。

**Checkpoint criteria:** 在一次 M1 聚合记录中保存每个探针的环境、结果、失败回退与 ADR/风险更新；Apple Music 不支持的 capability 不进入承诺。记录为 `Passed` 或 `Passed with known gaps` 后立即进入 M2，不等待用户签字。

## M2 — Application foundation

**Status:** Complete — `Passed with known gaps` on 2026-09-03; see [`M2 checkpoint`](../testing/checkpoints/M2.md). The 2026-09-02 blocked result remains preserved in that checkpoint's historical section; the authorized semantics, `TASK-008`, `TASK-010` and final hard-gate revalidation are now complete.

**Goal:** 建立可测试的 Tauri/React/Rust 外壳与契约边界。

**Checkpoint criteria:** candidate 可启动并展示空壳导航；固定依赖、IPC 错误封装、SQLite 迁移、secret store、日志脱敏与 schema contract tests 的最小链路可运行。CI 完整矩阵和覆盖率阈值可登记后递延，但构建、secret/日志与契约硬门槛必须通过。

## M3 — Local radio vertical slice

**Status:** Complete — `Passed with known gaps` on 2026-09-03; see [`M3 checkpoint`](../testing/checkpoints/M3.md). The Product Owner lifted the post-M3 stop condition on 2026-09-03 and authorized M4 only.

**Goal:** 从导入曲库到生成并连续播放一次完整本地节目。

**Checkpoint criteria:** 交付首个可用原型：在代表性许可 fixture 上完成“扫描/标签 → 计划 → 用户点击开始 → 连续播放一次本地节目 → 控制/停止”的主路径；OpenAI/TTS 不可用时能明确降级到确定性本地队列与文字。完整格式矩阵、10,000 首压力、200 次延迟采样和 30 分钟 soak 可递延到 M7。

## M4 — Understanding and conversation

**Status:** Complete — `Passed with known gaps` on 2026-09-03; see [`M4 checkpoint`](../testing/checkpoints/M4.md). The Product Owner lifted the post-M4 stop condition on 2026-09-03 and authorized M5 only.

**Goal:** 建立可解释、可控的用户画像、文字对话、摘要与记忆提案闭环。

**Checkpoint criteria:** 在 M3 candidate 上演示一次文字对话和记忆提案的审批、编辑、停用与删除，且下一轮上下文可见变化；原文保留与删除边界不被破坏。长周期清理、20 轮回归和完整导出矩阵可递延到 M7。

## M5 — Context and proactive scheduling

**Status:** Complete — `Passed with known gaps` on 2026-09-03; see [`M5 checkpoint`](../testing/checkpoints/M5.md). The revised candidate also passes the weather/schedule Windows desktop E2E after fixing scheduler runtime startup. Per Product Owner direction, stop after M5 and do not activate M6 without further authorization.

**Goal:** 增加手动城市天气、日程与“通知后确认开播”；托盘和自启动随 M6 设置整合完成。

**Checkpoint criteria:** 演示手动选城市、天气状态、创建一次日程通知以及用户确认后才启动；无人确认时无声音、无付费调用。完整 DST、休眠、重复通知和离线恢复矩阵可递延到 M7。

## M6 — Apple Music companion mode

**Status:** Inactive — not authorized. M5 completion does not activate M6 under the Product Owner's explicit stop condition.

**Goal:** 在 capability 边界内连接 Apple Music Windows 系统媒体会话并插入安全串场。

**Checkpoint criteria:** 在一台记录环境的真实 Windows App 会话演示连接、真实字段、能力门控和基础控制；通用 TTS 不携带 GSMTC 数据，用户抢占或会话消失时安全停止/不争抢。多版本矩阵和重复竞态压力可递延到 M7。

## M7 — Installable beta hardening

**Goal:** 交付本人及少量内测者可独立安装和恢复的 Windows 版本。

**Release criteria:** 不再允许未批准的 prototype known gaps。30 分钟节目、10,000 首曲库、安装/卸载/升级、数据导出与清除、离线/限流/损坏恢复、无障碍与隐私验收全部通过；满足全部 P0/P1 与覆盖率门槛，生成版本化 NSIS 制品、证据清单和 Changelog。

## Deferred beyond v1

- 完整 MusicKit Web、Spotify、网易云或其他流媒体曲库授权。
- 语音输入、唤醒词、屏幕/前台应用感知。
- 账号后端、遥测、自动更新、Microsoft Store 与公开商业发布。
- 浅色主题、高保真设计原型、移动端和 macOS。
