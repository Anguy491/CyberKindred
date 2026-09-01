# Roadmap

| Field | Value |
|---|---|
| Status | Approved |
| Owner | Product Owner / Lead Agent |
| Last Verified | 2026-09-02 |
| Source of Truth For | 里程碑顺序、依赖与阶段退出条件 |
| Related Documents | `../product/PRD.md`, `BACKLOG.md`, `RISK-REGISTER.md`, `../testing/TEST-STRATEGY.md` |

Roadmap 描述阶段，不承载具体需求或任务状态。具体执行顺序以 `BACKLOG.md` 为准。

## M0 — Documentation Baseline v1

**Status:** Complete — approved 2026-09-02.

**Goal:** 在产品代码前建立可审阅、可追踪、机器可验证的事实源。

**Exit criteria:**

- 全部计划文档、schema、examples 和 AGENTS 存在且通过文档质量检查。
- 所有 FR/NFR 都映射架构、契约或明确的非契约组件、测试与 Task。
- 用户明确批准基线；本地创建 `docs-baseline-v1` 标签。

## M1 — Feasibility and platform probes

**Status:** In Progress — `TASK-001` complete; `TASK-002` and `TASK-003` pending.

**Goal:** 用最小可丢弃探针消除 Windows 媒体与音频的高风险未知数，不形成产品 UI。

**Scope:** packaged/unpackaged GSMTC 枚举与 Apple Music 控制、rodio 常用格式/设备切换、Credential Manager、Tauri notification/tray/NSIS/WebView2。

**Exit criteria:** 每个探针有环境、结果、证据、失败回退与 ADR/风险更新；Apple Music 不支持的 capability 不进入承诺。

## M2 — Application foundation

**Goal:** 建立可测试的 Tauri/React/Rust 外壳与契约边界。

**Exit criteria:** 固定依赖、IPC 错误封装、SQLite 迁移、secret store、日志脱敏、schema contract tests、空壳导航与 CI 等质量门槛可运行。

## M3 — Local radio vertical slice

**Goal:** 从导入曲库到生成并连续播放一次完整本地节目。

**Exit criteria:** 扫描/标签/队列/播放、OpenAI structured ProgramPlan、TTS 开场与每 2–3 首串场、反馈和确定性离线队列全部通过验收。

## M4 — Understanding and conversation

**Goal:** 建立可解释、可控的用户画像、文字对话、摘要与记忆提案闭环。

**Exit criteria:** 30 天原文清理、摘要、记忆审批/编辑/停用/删除/导出、行为偏好权重及敏感边界测试完成。

## M5 — Context and proactive scheduling

**Goal:** 增加手动城市天气、日程与“通知后确认开播”；托盘和自启动随 M6 设置整合完成。

**Exit criteria:** 跨 DST、休眠恢复、离线天气、重复通知、未确认不出声和设置恢复场景通过。

## M6 — Apple Music companion mode

**Goal:** 在 capability 边界内连接 Apple Music Windows 系统媒体会话并插入安全串场。

**Exit criteria:** 真实订阅账号验证元数据、基础控制、会话切换、TTS pause/resume race、用户抢占和会话消失；不支持操作从 UI 隐藏。

## M7 — Installable beta hardening

**Goal:** 交付本人及少量内测者可独立安装和恢复的 Windows 版本。

**Exit criteria:** 30 分钟节目、10,000 首曲库、安装/卸载/升级、数据导出与清除、离线/限流/损坏恢复、无障碍与隐私验收全部通过；生成版本化 NSIS 制品和 Changelog。

## Deferred beyond v1

- 完整 MusicKit Web、Spotify、网易云或其他流媒体曲库授权。
- 语音输入、唤醒词、屏幕/前台应用感知。
- 账号后端、遥测、自动更新、Microsoft Store 与公开商业发布。
- 浅色主题、高保真设计原型、移动端和 macOS。
