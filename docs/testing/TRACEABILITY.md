# CyberKindred Requirements Traceability Matrix

| Metadata | Value |
|---|---|
| Status | Approved |
| Owner | Engineering Lead & Quality Engineering |
| Last Verified | 2026-09-08 |
| Source of Truth For | 每个 FR/NFR 到 UX、架构、契约、测试、任务和里程碑的逐项追踪关系 |
| Related Documents | `docs/product/FRS.md`; `docs/product/NFRS.md`; `docs/product/UX-SPEC.md`; `docs/architecture/ARCHITECTURE.md`; `docs/contracts/API-CONTRACT.md`; `docs/testing/ACCEPTANCE-TESTS.md`; `docs/planning/BACKLOG.md` |

## 1. 使用规则

本矩阵只建立引用，不重新定义需求或契约。表中 `Contract` 的 schema 文件名均位于 `docs/contracts/schemas/`；provider interface 指 `PROVIDER-CONTRACTS.md`。行为、契约或需求映射改变时必须更新受影响行，纯实现切片的状态整理可在 milestone checkpoint 集中完成。任何空单元格、引用不存在、需求无测试/任务或任务无需求都阻断开发。一个测试可覆盖多项需求，但测试失败时必须能回溯到具体需求断言。矩阵完整表示 beta 目标，不表示每个 `TEST-*` 都要在 M1–M6 的每次 checkpoint 执行。

任务状态仍以 [`BACKLOG.md`](../planning/BACKLOG.md) 为准，milestone 状态、checkpoint 结论与自动转场以 [`ROADMAP.md`](../planning/ROADMAP.md) 为准。`TASK-001` 的 M1 `TEST-APL-001/002` 证据归档于 [`TASK-001-MATRIX.md`](../../spikes/gsmtc/evidence/TASK-001-MATRIX.md)；M6 产品 UI 验收仍由 `TASK-025` 承担。`TASK-002` 的 [`Manual-TASK-002`](../../spikes/audio/MANUAL-TEST.md) 与其他探针证据在 M1 checkpoint 集中审阅；它不替代本矩阵为 FR-LIB-001、NFR-PERF-003、NFR-REL-004、NFR-COMPAT-002 映射的后续产品 `TEST-*`。

M1 已由 [`M1 checkpoint`](checkpoints/M1.md) 与其 [`evidence manifest`](../../artifacts/test-evidence/milestones/M1/manifest.json) 聚合记录。`TASK-002` 的 hermetic matrix 与 `TASK-003` 的标准用户 Credential Manager/HKCU/reset smoke 已完成；未执行的真实听感、双设备、通知/托盘/登录、NSIS 与完整产品 `TEST-*` 均保留为 `Not Run`/known gap，不因探针完成而改写本矩阵的后续验收范围。

M2 的最终实现证据与 2026-09-02 历史 hard-gate 失败由 [`M2 checkpoint`](checkpoints/M2.md) 及其 [`evidence manifest`](../../artifacts/test-evidence/milestones/M2/manifest.json) 聚合记录。`TASK-004`–`TASK-010` 已集成；Node/W3C desktop smoke 实际完成首次七步本地降级 onboarding、静音 RADIO 与 Settings 导航。Product Owner 已解决 credential 生命周期、model probe 流程和 terminal delivery 语义，`RISK-016`–`RISK-018` 已关闭；最终 hard gates 全部通过，M2 以 `Passed with known gaps` 关闭并激活 M3 `TASK-011`。未运行的完整性能、跨版本、真实 TTS 与后续专用 API 场景仍按 checkpoint 目标保留，不改写为通过。

M3 `TASK-011` 已集成六格式授权根扫描、坏文件隔离、碰撞容忍的增量身份匹配、事务批处理、取消以及唯一权威 terminal/outbox 重放。代表性许可 fixture 的 TEST-LIB-001/002 自动化路径通过；M3 checkpoint 当时记录的旧版 10,000 首性能矩阵为 `Not Run`，该发布门槛已由 2026-09-08 Product Owner 决策替换为 100 首端到端扫描，历史证据不回写为已执行。

M3 `TASK-012`–`TASK-014` 已集成 path-free API-015/曲库 UI、严格 MusicBrainz/CAA 最小披露与限流缓存边界，以及 API-016–API-023/EVT-001 串行本地播放 actor。Library 的 focused Rust/TypeScript/axe 路径、metadata hermetic provider/低置信保留原标签、本地六格式解码与未授权零设备打开均通过；真实听感、双设备切换、联网 metadata artifact 持久化与 desktop library E2E 保留到 M3 checkpoint 如实标注。

M3 `TASK-015` 已集成最多 200 个 path-free 真实候选的确定性评分、冷却降级、ProgramPlan schema/domain 双重校验、provider 非法 ID 拒绝与文字/本地队列 fallback；SQLite adapter 仅投影 adopted 标签和活动反馈，计划与 segments 原子持久化且启动恢复把未终止节目转为 interrupted。TEST-LIB-004 的固定时钟/seed、反馈、画像、批准记忆、冷却和候选外 ID 路径由 focused hermetic tests 覆盖。

M3 `TASK-016` 已集成 stateless OpenAI Responses ProgramPlan provider：请求固定 HTTPS `/v1/responses`、`store:false`、空 tools/`tool_choice:none`、strict `json_schema`、24k/4k 预算和 60 秒 deadline；上下文按 policy、画像、approved memory、可选天气、摘要、近期轮次、当前请求与 path-free candidates 排序并过滤 secret/路径。Hermetic tests 覆盖封闭请求、结构化失败恰好一次修复、取消/超时、provider 分类和候选外 ID 交回领域校验；真实联网调用保持 `Not Run`，不作为默认测试副作用。

M3 的最终实现证据由 [`M3 checkpoint`](checkpoints/M3.md) 及其 [`evidence manifest`](../../artifacts/test-evidence/milestones/M3/manifest.json) 聚合记录。`TASK-011`–`TASK-019` 已集成：代表性许可六格式 fixture 完成扫描/标签、确定性计划、六曲连续本地节目、显式停止和 OpenAI/TTS 不可用时的文字 fallback；Radio UI 只在用户点击“开始节目”后调用 API-024，并以 `programId`/revision 丢弃陈旧事件。Rust、TypeScript、契约、axe、视觉、E2E harness、自检和安全差异审查均通过；当时的实机出声、完整 Tauri Radio E2E、10,000 首、200 次控制采样、100×30 分钟 soak、Narrator/跨平台矩阵保持 `Not Run`。其中规模与长稳门槛已由 2026-09-08 决策替换为 100 首与 10 分钟，历史 checkpoint 不回写结果；其他缺口仍按 M7 处理。M3 以 `Passed with known gaps` 关闭。

M4 的最终实现证据由 [`M4 checkpoint`](checkpoints/M4.md) 及其 [`evidence manifest`](../../artifacts/test-evidence/milestones/M4/manifest.json) 聚合记录。Product Owner 于 2026-09-03 解除 post-M3 stop 并只授权 M4；`TASK-020`、`TASK-021`、`TASK-022` 已按依赖完成。文字请求/取消/反馈、确定性无付费降级、画像和趋势、提案审批/编辑/拒绝、记忆停用/启用/删除、摘要查看/删除、approved-memory-only Context、创建后 30 天原文清理及启动/每 24 小时维护均已集成。完整 Rust、TypeScript、契约、axe、视觉、依赖策略与精确 candidate 安全差异审查通过；完整 Tauri chat/memories E2E、live provider/实机音频、TEST-AI-001 固定评估集、删除后 20 轮语义级复活回归和 Narrator/分辨率矩阵保留为 M7 known gaps。M4 以 `Passed with known gaps` 关闭；按 Product Owner stop condition，M5 未激活。

M5 的最终实现证据由 [`M5 checkpoint`](checkpoints/M5.md) 及其 [`evidence manifest`](../../artifacts/test-evidence/milestones/M5/manifest.json) 聚合记录。`TASK-023`–`TASK-024` 已集成：API-048/049 只在显式点击后发送城市 query，Rust 签名候选与 revision 门控位置保存，Forecast 只发送经舍入坐标和固定参数，30 分钟新鲜缓存才可进入 Context；API-032–035/EVT-007、SQLite scheduler actor、IANA/DST occurrence、10/30/60 分钟单一 snooze、15 分钟 resume 边界和带开始/稍后/忽略动作的 Windows 静音通知已完成。通知、到期或恢复不会自动调用 API-024；只有 API-035 `start` 的一次性授权能通过 notification trigger。scheduler actor 改由 Tauri async runtime 托管后，同步启动回归及 weather/schedule Windows desktop E2E 通过。最终 candidate `f33f417a8d34983f4682105380967301c8122503` 的 Rust、React、严格 IPC、契约、axe、视觉、依赖策略、静态边界和 24/24 项安全差异审查均通过且零发现。live Open-Meteo、真实 Windows toast 动作/声音观察和完整 DST/休眠/重复/离线矩阵保持 `Not Run` 并递延 M7；全部 hard gates 通过，M5 以 `Passed with known gaps` 关闭，M6 按 Product Owner stop condition 保持未激活。

M6 的当前证据由 [`M6 checkpoint`](checkpoints/M6.md) 及其 [`evidence manifest`](../../artifacts/test-evidence/milestones/M6/manifest.json) 聚合。candidate `9c3add0853082c857d7de490904a94e58e69fc9` 已实现 API-016–025、Apple App GSMTC adapter、显式 discovery、真实字段/capability 映射、本地确定性反应、通用 TTS interruption、设置/托盘/自启动与 API-036/037/040–042 数据控制；自动化契约、Rust、React、构建和文档检查通过。最终 45/45 文件安全差异审查为 6 Medium、3 Low、零 Critical/High，但证明切回 Local 后 Apple observation 未撤销、切源可保留旧 TTS resume 权限、分类删除/天气 late write/export staging/Library Index 约束和 full-reset quiescence 不满足 FR-APL-005、FR-DAT-004/005 与 NFR-PRIV-003/NFR-REL-004。`TEST-APL-001/002/004` 的真实 Windows App 主路径仍 `waiting for user`；因此 checkpoint 为 `Blocked`，`TASK-027` 回退为 `Blocked`，M7 未激活。

上一段保留 2026-09-05 初始 checkpoint 的历史阻塞证据，其中“当前”仅指该候选当时的状态。M6 的最终证据仍由同一 [`M6 checkpoint`](checkpoints/M6.md) 及 [`evidence manifest`](../../artifacts/test-evidence/milestones/M6/manifest.json) 聚合记录：candidate `ea2f2fcc4d5ff077291119936aa975b37e84fe75` 修复 sealed review 的 Apple 授权、分类删除、weather/export race、Library Index 完整性和 full-reset quiescence findings。Apple Music Windows App `1.1540.23042.0` 的产品服务实机会话完成连接、结构化 track/capability 观察和一次真实 generic-TTS pause/restore；Apple 数据未进入 provider。fresh bypass review 追加发现的一个 High reset-admission race 已通过四个 producer 的原子准入门及实际任务完成等待关闭，最终无 open Critical/High。全部 M6 hard gates 通过，checkpoint 为 `Passed with known gaps`，`TASK-025`–`TASK-028` 均为 `Done` 并自动激活 M7；50 次/跨版本/实机抢占与会话消失、破坏性 native reset/restart 及 `RISK-022` 对抗性证明保留为 M7 evidence。

Product Owner 于 2026-09-08 调整 M7 质量规模：`NFR-PERF-002`/`TEST-LIB-002` 的发布硬门槛为 100 首六格式文件端到端扫描，`NFR-REL-001`/`TEST-RAD-005` 为 1 次真实及 10 次加速的 10 分钟节目。10,000 条合成目录/数据库规模与 30 分钟虚拟时序仍作为非阻断工程守卫记录，不构成 beta 支持声明；最终证据由 `TASK-032` 聚合。

## 2. Functional requirements

### 2.1 Onboarding

| Requirement | UX | Architecture | Contract | Test | Task | Milestone |
|---|---|---|---|---|---|---|
| FR-ONB-001 | UX-ONB-001 | ARCH-002, ARCH-006 | API-002, API-003, API-033, API-048, API-049 | TEST-ONB-001 | TASK-010, TASK-032 | M2/M7 |
| FR-ONB-002 | UX-ONB-002 | ARCH-002, ARCH-003, ARCH-004 | API-003, API-010, API-011, API-016 | TEST-ONB-001 | TASK-010 | M2 |
| FR-ONB-003 | UX-ONB-003 | ARCH-003, ARCH-007 | API-004, API-006 | TEST-ONB-002 | TASK-003, TASK-008 | M1/M2 |
| FR-ONB-004 | UX-ONB-003, UX-STA-003 | ARCH-008, ARCH-015 | API-006, [`provider-error.schema.json`](../contracts/schemas/provider-error.schema.json) | TEST-ONB-002 | TASK-008 | M2 |
| FR-ONB-005 | UX-ONB-004 | ARCH-008, ARCH-012 | API-009, API-038, API-043, EVT-008, EVT-009, EVT-011 | TEST-ONB-003 | TASK-008, TASK-017 | M2/M3 |
| FR-ONB-006 | UX-ONB-004 | ARCH-006, ARCH-011 | API-003 | TEST-ONB-001 | TASK-010 | M2 |
| FR-ONB-007 | UX-ONB-005 | ARCH-014, ARCH-016 | API-002, API-003, API-033, API-048, API-049 | TEST-ONB-004 | TASK-010 | M2 |

### 2.2 Radio and program

| Requirement | UX | Architecture | Contract | Test | Task | Milestone |
|---|---|---|---|---|---|---|
| FR-RAD-001 | UX-RAD-001, UX-WIN-002 | ARCH-014 | API-024, EVT-002 | TEST-ONB-004, TEST-RAD-001 | TASK-018, TASK-019, TASK-032 | M3/M7 |
| FR-RAD-002 | UX-STA-005 | ARCH-009, ARCH-010 | API-024, [`program-plan.schema.json`](../contracts/schemas/program-plan.schema.json) | TEST-LIB-004 | TASK-015, TASK-016 | M3 |
| FR-RAD-003 | UX-RAD-002, UX-RAD-004 | ARCH-013 | API-024, EVT-002, EVT-003, [`program-plan.schema.json`](../contracts/schemas/program-plan.schema.json) | TEST-RAD-001, TEST-AI-001 | TASK-018, TASK-019 | M3 |
| FR-RAD-004 | UX-RAD-002, UX-RAD-003 | ARCH-004, ARCH-013, ARCH-016 | API-016–API-023, EVT-001, [`playback-state.schema.json`](../contracts/schemas/playback-state.schema.json) | TEST-RAD-002 | TASK-006, TASK-009, TASK-014, TASK-019 | M2/M3 |
| FR-RAD-005 | UX-RAD-003, UX-STA-007 | ARCH-006, ARCH-010 | API-027 | TEST-RAD-002 | TASK-020 | M4 |
| FR-RAD-006 | UX-RAD-006 | ARCH-004, ARCH-012, ARCH-013 | API-025, [`playback-state.schema.json`](../contracts/schemas/playback-state.schema.json) | TEST-RAD-003 | TASK-014, TASK-018, TASK-026 | M3/M6 |
| FR-RAD-007 | UX-STA-003, UX-STA-005 | ARCH-008, ARCH-015 | API-024, EVT-009, [`provider-error.schema.json`](../contracts/schemas/provider-error.schema.json) | TEST-RAD-004 | TASK-017, TASK-018, TASK-019, TASK-029 | M3/M7 |

### 2.3 Local library

| Requirement | UX | Architecture | Contract | Test | Task | Milestone |
|---|---|---|---|---|---|---|
| FR-LIB-001 | UX-LIB-001, UX-LIB-002 | ARCH-002, ARCH-005 | API-011, API-013, EVT-005 | TEST-LIB-001 | TASK-002, TASK-011, TASK-032 | M1/M3/M7 |
| FR-LIB-002 | UX-LIB-002 | ARCH-006, ARCH-012 | API-013, API-014, EVT-005 | TEST-LIB-002 | TASK-011 | M3 |
| FR-LIB-003 | UX-LIB-003 | ARCH-005, ARCH-006 | API-013, API-015 | TEST-LIB-001 | TASK-011 | M3 |
| FR-LIB-004 | UX-LIB-004 | ARCH-008, ARCH-015 | API-015, [`PROVIDER-CONTRACTS.md` §5](../contracts/PROVIDER-CONTRACTS.md#5-metadataprovider) | TEST-LIB-003 | TASK-013 | M3 |
| FR-LIB-005 | UX-LIB-003, UX-LIB-005 | ARCH-006, ARCH-016 | API-015 | TEST-LIB-003 | TASK-009, TASK-012 | M2/M3 |
| FR-LIB-006 | UX-LIB-003 | ARCH-009, ARCH-010 | [`program-plan.schema.json`](../contracts/schemas/program-plan.schema.json), [`PROVIDER-CONTRACTS.md` §3](../contracts/PROVIDER-CONTRACTS.md#3-llmprovider) | TEST-LIB-004 | TASK-015 | M3 |

### 2.4 Apple Music companion mode

| Requirement | UX | Architecture | Contract | Test | Task | Milestone |
|---|---|---|---|---|---|---|
| FR-APL-001 | UX-SET-003, UX-RAD-006 | ARCH-004, ARCH-019 | API-001, API-016, [`PROVIDER-CONTRACTS.md` §2](../contracts/PROVIDER-CONTRACTS.md#2-musicsourceadapter) | TEST-APL-001 | TASK-001, TASK-025, TASK-032 | M1/M6/M7 |
| FR-APL-002 | UX-RAD-002 | ARCH-004, ARCH-016 | API-018, EVT-001, [`playback-state.schema.json`](../contracts/schemas/playback-state.schema.json) | TEST-APL-002 | TASK-001, TASK-025 | M1/M6 |
| FR-APL-003 | UX-RAD-003, UX-STA-006 | ARCH-004, ARCH-013 | API-019–API-023, EVT-001 | TEST-APL-002 | TASK-001, TASK-025 | M1/M6 |
| FR-APL-004 | UX-RAD-006 | ARCH-009, ARCH-010 | API-024, EVT-001, [`program-plan.schema.json`](../contracts/schemas/program-plan.schema.json), [`PROVIDER-CONTRACTS.md` §2.1](../contracts/PROVIDER-CONTRACTS.md#21-invariants), [`PROVIDER-CONTRACTS.md` §3.1](../contracts/PROVIDER-CONTRACTS.md#31-inputs) | TEST-APL-003 | TASK-026 | M6 |
| FR-APL-005 | UX-RAD-004, UX-RAD-006 | ARCH-013 | EVT-001, [`playback-event.schema.json`](../contracts/schemas/playback-event.schema.json), [`playback-state.schema.json`](../contracts/schemas/playback-state.schema.json), [`PROVIDER-CONTRACTS.md` §2.2](../contracts/PROVIDER-CONTRACTS.md#22-tts-interruption-protocol), [`PROVIDER-CONTRACTS.md` §4](../contracts/PROVIDER-CONTRACTS.md#4-ttsprovider) | TEST-APL-004 | TASK-026 | M6 |

### 2.5 Chat and memory

| Requirement | UX | Architecture | Contract | Test | Task | Milestone |
|---|---|---|---|---|---|---|
| FR-CHAT-001 | UX-RAD-005 | ARCH-009, ARCH-011 | API-026, EVT-004 | TEST-CHAT-001 | TASK-016, TASK-020, TASK-032 | M3/M4/M7 |
| FR-CHAT-002 | UX-RAD-005, UX-A11Y-003 | ARCH-003, ARCH-019 | API-026, EVT-004 | TEST-CHAT-002, TEST-AI-001 | TASK-020 | M4 |
| FR-CHAT-003 | UX-RAD-005 | ARCH-012 | API-038, EVT-011 | TEST-CHAT-003 | TASK-020 | M4 |
| FR-MEM-001 | UX-YOU-001 | ARCH-011, ARCH-016 | API-028, API-044, API-046, [`memory-record.schema.json`](../contracts/schemas/memory-record.schema.json) | TEST-MEM-001 | TASK-009, TASK-021, TASK-022, TASK-032 | M2/M4/M7 |
| FR-MEM-002 | UX-YOU-002 | ARCH-009, ARCH-011 | API-028, EVT-006, [`memory-record.schema.json`](../contracts/schemas/memory-record.schema.json) | TEST-MEM-001, TEST-AI-001 | TASK-021 | M4 |
| FR-MEM-003 | UX-YOU-002 | ARCH-006, ARCH-011 | API-029, API-030, API-039 | TEST-MEM-002 | TASK-021 | M4 |
| FR-MEM-004 | UX-YOU-003 | ARCH-006, ARCH-011 | API-030, API-031, API-045, [`memory-record.schema.json`](../contracts/schemas/memory-record.schema.json) | TEST-MEM-003 | TASK-007, TASK-021 | M2/M4 |
| FR-MEM-005 | UX-YOU-001, UX-YOU-004 | ARCH-006, ARCH-011 | API-046, API-047 | TEST-MEM-004 | TASK-022 | M4 |

### 2.6 Schedule and weather

| Requirement | UX | Architecture | Contract | Test | Task | Milestone |
|---|---|---|---|---|---|---|
| FR-SCH-001 | UX-SET-004 | ARCH-006, ARCH-014 | API-032, API-033, API-034, [`schedule-rule.schema.json`](../contracts/schemas/schedule-rule.schema.json) | TEST-SCH-001 | TASK-024, TASK-032 | M5/M7 |
| FR-SCH-002 | UX-WIN-001, UX-WIN-002 | ARCH-014 | API-035, EVT-007 | TEST-SCH-002 | TASK-003, TASK-024 | M1/M5 |
| FR-SCH-003 | UX-WIN-001 | ARCH-013, ARCH-014 | API-035, [`schedule-rule.schema.json`](../contracts/schemas/schedule-rule.schema.json) | TEST-SCH-003 | TASK-024 | M5 |
| FR-SCH-004 | UX-SET-004 | ARCH-013 | API-033, [`schedule-rule.schema.json`](../contracts/schemas/schedule-rule.schema.json) | TEST-SCH-001 | TASK-024 | M5 |
| FR-WEA-001 | UX-ONB-004, UX-SET-001, UX-SET-006 | ARCH-008 | API-048, API-049, [`PROVIDER-CONTRACTS.md` §6](../contracts/PROVIDER-CONTRACTS.md#6-weatherprovider) | TEST-WEA-001 | TASK-023, TASK-032 | M5/M7 |
| FR-WEA-002 | UX-RAD-001, UX-SET-006, UX-STA-005 | ARCH-008, ARCH-015 | API-007, [`PROVIDER-CONTRACTS.md` §6](../contracts/PROVIDER-CONTRACTS.md#6-weatherprovider) | TEST-WEA-002 | TASK-023, TASK-029 | M5/M7 |

### 2.7 Settings and data control

| Requirement | UX | Architecture | Contract | Test | Task | Milestone |
|---|---|---|---|---|---|---|
| FR-SET-001 | UX-SET-001, UX-SET-002 | ARCH-007, ARCH-008 | API-004–API-009 | TEST-SET-001 | TASK-008, TASK-028, TASK-032 | M2/M6/M7 |
| FR-SET-002 | UX-SET-001 | ARCH-004, ARCH-015 | API-007, API-008 | TEST-SET-002 | TASK-017, TASK-028 | M3/M6 |
| FR-SET-003 | UX-SET-004, UX-WIN-003 | ARCH-014, ARCH-018 | API-007, API-008 | TEST-SET-003 | TASK-003, TASK-028 | M1/M6 |
| FR-SET-004 | UX-SET-002, UX-SET-003 | ARCH-008, ARCH-015 | API-006, API-007 | TEST-SET-001 | TASK-006, TASK-008, TASK-009, TASK-023, TASK-028 | M2/M5/M6 |
| FR-DAT-001 | UX-SET-005 | ARCH-006, ARCH-011 | API-040, [`API-CONTRACT.md` §3.4](../contracts/API-CONTRACT.md#34-memory-schedule-and-data-control), [`DATA-MODEL.md` §5](../architecture/DATA-MODEL.md#5-数据保留与清理) | TEST-MEM-004 | TASK-007, TASK-022, TASK-032 | M2/M4/M7 |
| FR-DAT-002 | UX-SET-005 | ARCH-006, ARCH-007 | API-040 | TEST-DAT-001 | TASK-007, TASK-027 | M2/M6 |
| FR-DAT-003 | UX-SET-005, UX-STA-007 | ARCH-007, ARCH-017 | API-036, EVT-008 | TEST-DAT-002 | TASK-027 | M6 |
| FR-DAT-004 | UX-LIB-005, UX-SET-005, UX-STA-008 | ARCH-003, ARCH-006 | API-041, API-042 | TEST-DAT-003 | TASK-027 | M6 |
| FR-DAT-005 | UX-SET-005, UX-STA-008 | ARCH-007, ARCH-014, ARCH-018 | API-037 | TEST-DAT-004 | TASK-003, TASK-007, TASK-027 | M1/M2/M6 |

## 3. Non-functional requirements

### 3.1 Performance and reliability

| Requirement | UX | Architecture | Contract | Test | Task | Milestone |
|---|---|---|---|---|---|---|
| NFR-PERF-001 | UX-STA-001 | ARCH-012, ARCH-016 | API-001, [`API-CONTRACT.md` §3.1](../contracts/API-CONTRACT.md#31-app-onboarding-settings-and-secrets) | TEST-ONB-004 | TASK-010 | M2 |
| NFR-PERF-002 | UX-LIB-002 | ARCH-005, ARCH-012 | API-013, API-014, EVT-005 | TEST-LIB-002 | TASK-011, TASK-032 | M3/M7 |
| NFR-PERF-003 | UX-RAD-003, UX-STA-007 | ARCH-013, ARCH-016 | API-019–API-023, API-027, EVT-001 | TEST-RAD-002 | TASK-002, TASK-012, TASK-014, TASK-019 | M1/M3 |
| NFR-PERF-004 | UX-RAD-002, UX-STA-006 | ARCH-004, ARCH-013 | EVT-001, [`playback-event.schema.json`](../contracts/schemas/playback-event.schema.json) | TEST-APL-002 | TASK-001, TASK-025 | M1/M6 |
| NFR-PERF-005 | UX-WIN-003 | ARCH-012, ARCH-018 | [`API-CONTRACT.md` §4](../contracts/API-CONTRACT.md#4-events) | TEST-RESOURCE-001 | TASK-032 | M7 |
| NFR-REL-001 | UX-RAD-002, UX-STA-003 | ARCH-005, ARCH-013 | EVT-001–EVT-003, [`playback-event.schema.json`](../contracts/schemas/playback-event.schema.json) | TEST-RAD-005 | TASK-014, TASK-018, TASK-032 | M3/M7 |
| NFR-REL-002 | UX-STA-003 | ARCH-006, ARCH-013, ARCH-016 | API-018, ERR-1402, [`API-CONTRACT.md` §5](../contracts/API-CONTRACT.md#5-error-registry) | TEST-REL-001 | TASK-007, TASK-029 | M2/M7 |
| NFR-REL-003 | UX-STA-003, UX-STA-005 | ARCH-008, ARCH-015 | ERR-1302–ERR-1305, [`provider-error.schema.json`](../contracts/schemas/provider-error.schema.json) | TEST-RAD-004 | TASK-015, TASK-016, TASK-018 | M3 |
| NFR-REL-004 | UX-STA-005, UX-STA-006 | ARCH-013 | EVT-001, EVT-010, [`playback-state.schema.json`](../contracts/schemas/playback-state.schema.json), [`playback-event.schema.json`](../contracts/schemas/playback-event.schema.json) | TEST-APL-004 | TASK-002, TASK-014, TASK-024, TASK-025, TASK-026, TASK-029 | M1/M3/M5/M6/M7 |

### 3.2 Security and privacy

| Requirement | UX | Architecture | Contract | Test | Task | Milestone |
|---|---|---|---|---|---|---|
| NFR-SEC-001 | UX-ONB-003, UX-SET-002 | ARCH-007 | API-004, API-005, ERR-1101, ERR-1102, [`API-CONTRACT.md` §1.1](../contracts/API-CONTRACT.md#11-success-and-failure) | TEST-ONB-002 | TASK-003, TASK-007, TASK-028 | M1/M2/M6 |
| NFR-SEC-002 | UX-STA-003 | ARCH-001, ARCH-002, ARCH-008 | API-001–API-049, [`API-CONTRACT.md` §1](../contracts/API-CONTRACT.md#1-boundary-and-transport), [`API-CONTRACT.md` §6](../contracts/API-CONTRACT.md#6-capability-and-compatibility-rules) | TEST-SEC-001 | TASK-006, TASK-031 | M2/M7 |
| NFR-SEC-003 | UX-LIB-001, UX-STA-008 | ARCH-002, ARCH-003 | API-011, API-036, API-041, API-042, ERR-1501 | TEST-SEC-002 | TASK-007, TASK-027 | M2/M6 |
| NFR-SEC-004 | UX-STA-003 | ARCH-017 | [`API-CONTRACT.md` §1.1](../contracts/API-CONTRACT.md#11-success-and-failure), [`provider-error.schema.json`](../contracts/schemas/provider-error.schema.json), [`PROVIDER-CONTRACTS.md` §1](../contracts/PROVIDER-CONTRACTS.md#1-common-rules) | TEST-SEC-001 | TASK-006, TASK-032 | M2/M7 |
| NFR-SEC-005 | UX-SET-002 | ARCH-019 | [`DEPENDENCY-POLICY.md`](../architecture/DEPENDENCY-POLICY.md), [`API-CONTRACT.md` §6](../contracts/API-CONTRACT.md#6-capability-and-compatibility-rules) | TEST-SEC-003 | TASK-031 | M7 |
| NFR-PRIV-001 | UX-A11Y-003, UX-ONB-005 | ARCH-001, ARCH-019 | API-001, [`API-CONTRACT.md` §6](../contracts/API-CONTRACT.md#6-capability-and-compatibility-rules) | TEST-PRIV-001 | TASK-023 | M5 |
| NFR-PRIV-002 | UX-SET-005 | ARCH-006, ARCH-011 | API-040, [`API-CONTRACT.md` §3.4](../contracts/API-CONTRACT.md#34-memory-schedule-and-data-control), [`DATA-MODEL.md` §5](../architecture/DATA-MODEL.md#5-数据保留与清理) | TEST-MEM-004 | TASK-007, TASK-022 | M2/M4 |
| NFR-PRIV-003 | UX-SET-005, UX-STA-008 | ARCH-006, ARCH-007 | API-036, API-037, API-040, API-041, API-042 | TEST-DAT-002, TEST-DAT-003, TEST-DAT-004 | TASK-027 | M6 |
| NFR-PRIV-004 | UX-ONB-005, UX-SET-002, UX-SET-006 | ARCH-008, ARCH-009 | API-048, API-049, [`PROVIDER-CONTRACTS.md` §2.1](../contracts/PROVIDER-CONTRACTS.md#21-invariants), [`PROVIDER-CONTRACTS.md` §3](../contracts/PROVIDER-CONTRACTS.md#3-llmprovider), [`PROVIDER-CONTRACTS.md` §5](../contracts/PROVIDER-CONTRACTS.md#5-metadataprovider), [`PROVIDER-CONTRACTS.md` §6](../contracts/PROVIDER-CONTRACTS.md#6-weatherprovider) | TEST-LIB-003, TEST-APL-003, TEST-WEA-001, TEST-WEA-002, TEST-PRIV-001 | TASK-013, TASK-016, TASK-023 | M3/M5 |
| NFR-PRIV-005 | UX-YOU-002, UX-YOU-003 | ARCH-009, ARCH-011 | API-028–API-031, API-039, [`memory-record.schema.json`](../contracts/schemas/memory-record.schema.json) | TEST-MEM-001, TEST-MEM-003 | TASK-021 | M4 |

### 3.3 Accessibility, compatibility and cost

| Requirement | UX | Architecture | Contract | Test | Task | Milestone |
|---|---|---|---|---|---|---|
| NFR-A11Y-001 | UX-NAV-003, UX-A11Y-001 | ARCH-001 | [`API-CONTRACT.md` §2](../contracts/API-CONTRACT.md#2-shared-dtos), [`API-CONTRACT.md` §4](../contracts/API-CONTRACT.md#4-events) | TEST-A11Y-001 | TASK-010, TASK-012, TASK-019, TASK-022, TASK-030 | M2/M3/M4/M7 |
| NFR-A11Y-002 | UX-SYS-002, UX-SYS-004 | ARCH-001 | [`UX-SPEC.md` §3](../product/UX-SPEC.md#3-design-tokens), [`API-CONTRACT.md` §7](../contracts/API-CONTRACT.md#7-contract-enforcement) | TEST-A11Y-002 | TASK-009, TASK-030 | M2/M7 |
| NFR-A11Y-003 | UX-A11Y-002, UX-STA-003 | ARCH-016 | EVT-001–EVT-011, [`API-CONTRACT.md` §1.1](../contracts/API-CONTRACT.md#11-success-and-failure) | TEST-A11Y-001 | TASK-030 | M7 |
| NFR-A11Y-004 | UX-SYS-005, UX-A11Y-001 | ARCH-001 | [`UX-SPEC.md` §3.4](../product/UX-SPEC.md#34-组件图标与动效), [`API-CONTRACT.md` §7](../contracts/API-CONTRACT.md#7-contract-enforcement) | TEST-A11Y-002 | TASK-009, TASK-030 | M2/M7 |
| NFR-COMPAT-001 | UX-NAV-001, UX-WIN-003 | ARCH-018 | API-001, [`API-CONTRACT.md` §6](../contracts/API-CONTRACT.md#6-capability-and-compatibility-rules) | TEST-COMPAT-001 | TASK-003, TASK-004, TASK-028, TASK-031 | M1/M2/M6/M7 |
| NFR-COMPAT-002 | UX-LIB-002 | ARCH-005 | API-013, EVT-005 | TEST-LIB-001 | TASK-002, TASK-011 | M1/M3 |
| NFR-COMPAT-003 | UX-A11Y-001, UX-SET-003 | ARCH-004, ARCH-018 | API-001, EVT-001, [`playback-state.schema.json`](../contracts/schemas/playback-state.schema.json), [`playback-event.schema.json`](../contracts/schemas/playback-event.schema.json) | TEST-COMPAT-002 | TASK-001, TASK-009, TASK-025, TASK-030 | M1/M2/M6/M7 |
| NFR-COST-001 | UX-WIN-001, UX-ONB-004 | ARCH-008, ARCH-014 | API-006, API-009, API-024, API-026 | TEST-ONB-003, TEST-SCH-002 | TASK-008, TASK-017, TASK-020, TASK-024 | M2/M3/M4/M5 |
| NFR-COST-002 | UX-RAD-007, UX-SET-002 | ARCH-009, ARCH-010 | [`PROVIDER-CONTRACTS.md` §3](../contracts/PROVIDER-CONTRACTS.md#3-llmprovider), [`PROVIDER-CONTRACTS.md` §4](../contracts/PROVIDER-CONTRACTS.md#4-ttsprovider), [`program-plan.schema.json`](../contracts/schemas/program-plan.schema.json) | TEST-COST-001 | TASK-015, TASK-016, TASK-017 | M3 |

### 3.4 Maintainability and offline degradation

| Requirement | UX | Architecture | Contract | Test | Task | Milestone |
|---|---|---|---|---|---|---|
| NFR-MAINT-001 | UX-STA-003 | ARCH-003, ARCH-016 | [`API-CONTRACT.md` §7](../contracts/API-CONTRACT.md#7-contract-enforcement), [`schemas/`](../contracts/schemas/), [`examples/`](../contracts/examples/) | TEST-MAINT-001 | TASK-004, TASK-005, TASK-032 | M2/M7 |
| NFR-MAINT-002 | UX-STA-003 | ARCH-010, ARCH-011, ARCH-013 | [`API-CONTRACT.md` §7](../contracts/API-CONTRACT.md#7-contract-enforcement), [`program-plan.schema.json`](../contracts/schemas/program-plan.schema.json), [`memory-record.schema.json`](../contracts/schemas/memory-record.schema.json), [`playback-state.schema.json`](../contracts/schemas/playback-state.schema.json) | TEST-MAINT-002 | TASK-004, TASK-032 | M2/M7 |
| NFR-MAINT-003 | UX-STA-003 | ARCH-003, ARCH-012, ARCH-019 | [`API-CONTRACT.md` §3](../contracts/API-CONTRACT.md#3-commands), [`API-CONTRACT.md` §4](../contracts/API-CONTRACT.md#4-events), [`API-CONTRACT.md` §5](../contracts/API-CONTRACT.md#5-error-registry), [`schemas/`](../contracts/schemas/) | TEST-MAINT-001 | TASK-004, TASK-005, TASK-006, TASK-032 | M2/M7 |
| NFR-OFF-001 | UX-STA-004 | ARCH-006, ARCH-015 | local APIs API-007, API-015, API-018–API-023, API-028–API-047 | TEST-OFF-001 | TASK-029 | M7 |
| NFR-OFF-002 | UX-STA-004, UX-STA-005 | ARCH-008, ARCH-015 | [`provider-error.schema.json`](../contracts/schemas/provider-error.schema.json), ERR-1302–ERR-1305 | TEST-LIB-003, TEST-RAD-004, TEST-WEA-002 | TASK-013, TASK-023, TASK-029 | M3/M5/M7 |
| NFR-OFF-003 | UX-STA-004, UX-STA-007 | ARCH-008, ARCH-012, ARCH-015 | API-006, [`PROVIDER-CONTRACTS.md` §1](../contracts/PROVIDER-CONTRACTS.md#1-common-rules), [`API-CONTRACT.md` §1.1](../contracts/API-CONTRACT.md#11-success-and-failure) | TEST-OFF-002 | TASK-029 | M7 |

## 4. Coverage summary and audit method

本基线追踪 48 个 `FR-*` 与 34 个 `NFR-*`，每行均具有 UX、架构、契约、验收测试、交付任务和里程碑。`scripts/verify-docs.ps1` 检查 ID 唯一性、引用和需求/任务覆盖；语义审查还必须确认表中引用确实实现该断言。若新增需求，先增加权威需求 ID，再在同一变更中补齐本矩阵、验收场景和 Backlog；禁止先创建无需求任务。M1–M6 可只执行当前 checkpoint subset，但 M7 前所有适用边必须有 passing evidence。
