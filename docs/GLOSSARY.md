# CyberKindred 术语表

| Metadata | Value |
|---|---|
| Status | Draft |
| Owner | Product & Architecture |
| Last Verified | 2026-09-01 |
| Source of Truth For | 项目领域术语及其规范含义 |
| Related Documents | `docs/product/PRD.md`; `docs/product/FRS.md`; `docs/architecture/ARCHITECTURE.md`; `docs/contracts/API-CONTRACT.md` |

本文档只定义术语，不定义需求优先级、实现细节或接口字段。英文术语用于代码、契约与标识；中文释义用于产品沟通。

| Term | 中文名 | 规范含义 |
|---|---|---|
| CyberKindred | CyberKindred | Windows 优先的 AI 陪伴电台产品名称。 |
| Program | 节目 | 一次由开场、音乐段与串场组成的连续收听会话；开始与结束都有持久化记录。 |
| Program Plan | 节目计划 | 经过结构化校验的有序 Segment 列表；本地源可含精确曲目，Apple Music 陪伴模式不含远程精确队列。 |
| Segment | 节目段 | `Track Segment` 或 `Voice Segment` 的统称。 |
| Track Segment | 音乐段 | 播放一首真实且可解析曲目的节目段。 |
| Voice Segment | 语音段 | AI 主播文本及其可选 TTS 音频构成的节目段，包括开场、串场和结束语。 |
| Local Source | 本地音乐源 | 对用户授权目录中的本地音频进行扫描、精确选曲、排序和播放的来源。 |
| System Media Session | 系统媒体会话 | Windows Global System Media Transport Controls 暴露的媒体状态、元数据和控制能力。 |
| GSMTC | Windows 全局媒体传输控制 | CyberKindred 用来发现并陪伴控制 Apple Music Windows App 的 Windows API；不等同于 Apple Music API。 |
| Apple Music Companion Mode | Apple Music 陪伴模式 | 监听 Apple Music 的系统媒体会话，按实际能力控制播放并根据真实歌曲生成串场的模式；不承诺指定任意远程曲目。 |
| MusicKit | Apple Music 开发接口 | Apple 官方的目录、播放与用户授权接口；不属于首发版本。 |
| Playback Capability | 播放能力 | 某一 Music Source 在当前运行时明确声明可执行的动作，如播放、暂停、跳转或设置队列。 |
| Capability Negotiation | 能力协商 | 客户端依据来源实时声明的能力显示或禁用控制，不假定所有来源能力相同。 |
| Music Source | 音乐源 | 实现统一播放来源契约的组件；首发包含 Local Source 与 System Media Session Source。 |
| Candidate Set | 候选曲目集 | 根据真实曲库、时间、偏好、历史和重复约束筛出的本地曲目集合，LLM 只能在其中重排或选择。 |
| Now Playing | 正在播放 | 当前媒体项、时间线、来源、播放状态与可用能力的统一快照。 |
| Interlude | 串场 | AI 主播在开场或若干歌曲之间给出的简短文字，可选由 TTS 播放。 |
| TTS | 文字转语音 | 将已批准的主播文本合成为音频；首发通过用户自备 OpenAI Key 调用。 |
| BYOK | 用户自备密钥 | 用户提供自己的服务 API Key，密钥由 Windows Credential Manager 保存，且不存入数据库或 WebView。 |
| Provider | 外部能力提供者 | LLM、TTS、元数据或天气能力的可替换适配器。 |
| LLM Provider | 大语言模型提供者 | 根据受约束上下文产生结构化节目计划、主播文本、摘要与记忆提案的适配器。 |
| Metadata Provider | 元数据提供者 | 使用文本标签查询并补全曲目元数据的适配器；首发为 MusicBrainz，并可结合 Cover Art Archive。 |
| Weather Provider | 天气提供者 | 仅在用户主动搜索时查询城市候选，并以用户明确选定的城市坐标读取天气上下文的适配器；首发为 Open-Meteo Geocoding 与 Forecast。 |
| User Profile | 用户画像 | 用户显式填写且可编辑的作息、偏好、称呼、陪伴边界等稳定信息。 |
| Memory Proposal | 记忆提案 | AI 从对话或反馈中推断、等待用户查看或批准的长期记忆候选；未批准不得作为长期事实使用。 |
| Approved Memory | 已批准记忆 | 用户批准或直接创建、可查看、修改、停用和删除的长期记忆。 |
| Session Summary | 会话摘要 | 对一次或多次交互的压缩长期表示；不替代原始对话的可见与删除控制。 |
| Raw Conversation | 原始对话 | 用户文字输入与 AI 原始回复；默认从产生之日起保留 30 天。 |
| Feedback Event | 反馈事件 | 喜欢、跳过、少说一点或文字反馈等显式/行为反馈，用于更新偏好。 |
| Schedule Rule | 日程规则 | 用户配置的星期、当地时间、时区和启用状态，用于触发节目通知。 |
| Notification-first Playback | 通知后确认开播 | 到达日程时先显示 Windows 通知，只有用户明确确认才允许开始出声。 |
| Manual City | 手动城市 | 用户主动选择的天气位置；首发不读取设备定位。 |
| Offline Degradation | 离线降级 | 网络能力不可用时继续提供本地播放、设置与数据管理，并明确标示不可用的联网功能。 |
| Hermetic Test | 封闭测试 | 不依赖真实付费 API、用户账号、互联网或机器外部状态，且可重复执行的测试。 |
| Documentation Baseline v1 | 文档基线 v1 | 用户批准后约束首发开发的需求、架构、契约、测试与任务集合。 |
