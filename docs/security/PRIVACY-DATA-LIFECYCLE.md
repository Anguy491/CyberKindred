# Privacy and Data Lifecycle

| Field | Value |
|---|---|
| Status | Approved |
| Owner | Privacy Steward |
| Last Verified | 2026-09-02 |
| Source of Truth For | 用户数据的来源、用途、本地存储、外发、保留、导出与删除语义 |
| Related Documents | `docs/architecture/DATA-MODEL.md`, `docs/contracts/API-CONTRACT.md`, `docs/integrations/EXTERNAL-INTEGRATIONS.md`, `docs/security/THREAT-MODEL.md` |

## 1. Privacy posture

CyberKindred 是单 Windows 用户、本地优先、BYOK 的 AI 陪伴电台。它不建立 CyberKindred 云账户，不运行产品遥测，不采集屏幕、前台应用、麦克风、通讯录、精确设备位置或浏览器 cookie。计划时段只显示通知；没有用户确认就不播放声音。

本地优先不等于完全离线：用户启用 LLM/TTS、metadata 或天气后，Rust Core 会向对应第三方发送本文件列出的最小数据。首次引导展示分项披露；Settings 可独立关闭 metadata、weather、LLM/TTS。关闭只阻止后续外发，已有本地数据按下表保留，用户可立即删除。

## 2. Data inventory

| Data class | Source and purpose | Local storage | External disclosure | Default retention | Delete behavior | Export behavior |
|---|---|---|---|---|---|---|
| OpenAI API Key | 用户输入；认证 Responses/Speech | Windows Credential Manager，按 canonical provider origin 隔离；SQLite/WebView 只看 origin 与 configured boolean | 仅作为 Authorization header 发往用户确认的 HTTPS origin | 直到用户通过 API-005 删除指定 origin 或全部重置 | API-005 立即删除指定 Credential item；全部重置枚举并删除所有 `CyberKindred/provider/*` origin items；已发请求无法由本地撤回 | 永不导出 |
| Profile and preferences | 用户引导/Settings；称呼、节目风格、作息、初始偏好 | SQLite `user_profile`/`app_settings` | 生成内容时，必要摘要发往 LLM；不发 display name，除非它是用户明确要求的主播称呼 | 直到用户编辑、分类删除或全部重置 | 覆盖旧值；分类删除清画像/偏好，全部重置立即删除数据库而不保留 quarantine | 导出非敏感值与 schema version |
| City search and selected location | 用户主动输入城市/邮编查询并从结果选择；天气上下文与日程 | 未选 search query/candidate 只在内存 10 分钟；选中的 city/region/country/坐标/timezone 存 SQLite；不是 GPS 轨迹 | 搜索时 Open-Meteo 收到 query/language/count/IP；Forecast 收到四位小数坐标、timezone、变量/IP。LLM 只收到天气摘要，不收到 query/坐标 | Query/candidate 10 分钟；选中值直到编辑/删除；weather cache 最多 7 天，30 分钟后不再作为新鲜上下文 | 清 search 内存、selected location 与 weather cache；日程 timezone 不自动删除 | 导出选中的 city/region/country/坐标/timezone，不导出 search query；提示坐标属于位置数据 |
| Library roots and file identity | 用户通过 native picker 授权；定位音乐 | SQLite 保存 canonical root 与相对路径；音频保持原位只读 | 不外发路径、文件名、音频或 fingerprint | 直到移除 root/索引或重置 | 只删索引与 cache reference，永不删音乐文件 | 不导出 root、path、identity 或曲库统计；仅在 API-040 本机清单显示数量 |
| Embedded music tags | 本地文件 title/artist/album/duration/genre；检索和候选生成 | SQLite track index；不改原标签 | 启用匹配时只向 MusicBrainz 发 title/artist/album/duration；候选文本可发 LLM，无音频/路径 | 跟随曲库索引；移除 root 时删除 | 删除 root 索引；原文件不变 | 不导出 track ID/tag/index |
| MusicBrainz match | MusicBrainz 返回 MBID、标准化标签、tag、置信度 | SQLite 只存规范化结果，不存原始 body | release MBID 可发 Cover Art Archive | 成功结果保留至手动刷新/移除 track；no-match 24 小时 | 删除 track 或 metadata cache 时删除 | 不导出 |
| Artwork and TTS cache | 本地内嵌封面、Cover Art Archive 图片、OpenAI TTS bytes；减少重复请求 | Rust-owned cache；staging 24 小时 | CAA 只收 release MBID；OpenAI 收合规的最终串场文本与声音参数。Apple track-aware 文字不进入 Speech，只有无任何 GSMTC 派生数据的通用段可 TTS | CAA/TTS 成功 cache 最多 30 天，并受 quota/LRU；staging 24 小时 | `metadata_cache` 分类清 cache 或全部重置；不影响音乐/文本记录 | 不导出二进制 cache |
| Apple/system media now playing | 用户选择的 GSMTC session；显示和反应式串场 | 当前状态仅内存；节目事实可存 opaque media identity 与显示标签 | 不发送 Responses、Speech、metadata、weather 或 Apple API；无 Apple 凭据。曲目变化只触发本地 deterministic 可见文字 | 当前状态随进程；节目事实直到用户清除历史/重置 | 断开清内存；`playback_history` 清节目事实 | 导出历史时含当时的安全显示标签/opaque ID，不含 account/session token |
| Playback history and feedback | 播放状态与用户 like/skip/less-talk；推荐与偏好 | SQLite `program_runs`、`playback_events`、`feedback` | 只有 local-source 的聚合近期信号/实际播放事实可发 LLM；system-session/GSMTC facts 不外发；不发完整时间线给 metadata/weather | 直到用户清除收听历史或全部重置 | 删除事件/反馈/节目事实并重算 `last_played`; 不删曲库 | 导出事件时间、opaque track/source ID 与反馈 |
| Raw chat messages | 用户文字与 AI 文字；当前会话和摘要 | SQLite message body 与 hash | 发送必要近期 turns 至 OpenAI Responses；不发 metadata/weather | 创建后固定 30 天，不因访问延期 | 用户可立即删单条/session；定时清理到期正文 | 只导出 conversation covered-from/to 与 user/assistant message count，不导出 message ID/hash/正文；summary 由其独立行导出 |
| Voice segment text | LLM 生成串场；显示与 TTS | SQLite 30 天，之后正文置 null、保留 hash/状态 | 发 OpenAI Speech（若启用） | segment 结束后 30 天 | 可随 program/session 删除；cache 分开删除 | 不导出已过期正文；未过期正文也不在 MVP export 中 |
| Session summaries | LLM 或 deterministic 摘要；跨会话连续性 | SQLite，可见摘要最多 1000 Unicode 字符，另存本机 source message IDs/time range/prompt/model version | 可作为后续 LLM context；不发其他 provider；Apple/GSMTC facts 不进入摘要输入 | 直到删除对应 session/summary 或全部重置 | 删除后不再进入 context，相关 source mapping 同步删除 | 导出摘要、时间范围、生成方式，不导出 source hash/message ID |
| Memory proposals | AI 根据对话提出；等待用户审查 | SQLite proposal、confidence、source hash/status | 在审批前不得进入后续 LLM profile context | pending 90 天；rejected 正文 30 天后清空 | 用户可拒绝/删除；source message 删除后只留不可逆 hash | 只导出 proposal status，不导出 proposal 正文/source hash |
| Approved memories and revisions | 用户明确批准/编辑；长期个性化 | SQLite；只有 active memory 进入 context | active 内容可发 LLM；不发 metadata/weather | 直到用户禁用/删除/重置 | 删除立即清空所有 revision 正文，仅留最小 hash/change audit | 导出 active/disabled 内容和可见 revision 元数据 |
| Schedule and notification state | 用户设置时区、星期、时间；主动提醒 | SQLite schedule/occurrence；Windows notification state | 不发第三方 | 直到删除 rule/重置；历史 occurrence 随重置或规则删除 | 删除 rule 级联 occurrence；撤销未触发 notification | 作为 non-sensitive settings 导出 schedule rule；不导出 occurrence 或 OS notification identifier |
| Provider usage facts | Rust 生成；预算、可靠性诊断 | SQLite 仅 provider/kind/model/units/latency/status/correlation ID，无 body | 不外发 | 13 个月 | 定时或重置删除 | 不导出；只在 API-040 本机清单与会话用量 UI 显示聚合值 |
| Operation outbox | Rust 生成；跨线程投递 command/event 的最小状态 | SQLite 只存 event kind、opaque entity/operation ID、sequence、attempt/status/time 与数值计数；永不存 chat/voice/model 正文、GSMTC metadata、secret 或路径 | 不外发；只供本机 IPC 投递 | delivered 24 小时；undelivered 最多 7 天 | 生命周期自动清理；相关实体分类删除同步删除引用；全部重置删除 | 不导出 |
| Diagnostic logs | Rust 生成；本机故障诊断 | app log dir，结构化脱敏，最多 50 MiB | 不自动上传；用户自行决定是否提供 | 14 天 rolling | 分类清除或全部重置 | 默认不进入数据 export；支持单独生成脱敏诊断包时需再次预览确认 |
| Migration backups | 数据库迁移失败时的本机可靠恢复 | app data backups | 不外发 | 最近 3 个成功 pre-migration backup 最多 30 天 | 到期删除；任一用户数据分类删除与全部重置都同时删除旧 backup，防止已删数据从副本恢复 | 不导出 |

API-040 必须为上表每一行返回一项，而不是只返回五个可删集合；按表顺序对应的稳定 `DataInventoryCategory` 为 `credentials`、`profile_and_preferences`、`weather_location_and_cache`、`library_roots_and_identity`、`embedded_music_tags`、`metadata_matches`、`artwork_and_tts_cache`、`system_media_runtime`、`playback_history_and_feedback`、`chat_messages`、`voice_segment_text`、`session_summaries`、`memory_proposals`、`approved_memories_and_revisions`、`schedules_and_notifications`、`provider_usage_facts`、`operation_outbox`、`diagnostic_logs`、`migration_backups`。同一项跨多个介质时 `storageClasses` 返回去重数组；仅能由 API-041/042 删除的五类才带非 null `deletionCategory`。

## 3. External recipients and purposes

| Recipient | Purpose | Data categories | Controller outside CyberKindred |
|---|---|---|---|
| User-confirmed OpenAI-compatible origin | 生成节目、对话、摘要、TTS | 有界 context、批准记忆、近期对话、local-source 候选/事实、合规 TTS text、API Key header；不含 GSMTC/Apple 数据 | 由该 origin 的条款与用户账户控制；官方默认是 OpenAI |
| MusicBrainz | 文本 metadata 匹配 | title/artist/album/duration、request IP、User-Agent | MetaBrainz Foundation |
| Cover Art Archive / Internet Archive | 获取 release cover | release MBID、request IP | MetaBrainz / Internet Archive |
| Open-Meteo / GeoNames-backed geocoder | 用户主动城市搜索与当前天气 | Search query/language/count；选定城市的四位小数坐标、timezone、变量；request IP | OpenMeteo GmbH；location dataset 归因 GeoNames |
| Windows GSMTC | 读取/控制用户选择的本地系统媒体会话 | 本机 session metadata 与控制 action | 本机 Windows 与 source app；CyberKindred 不传给 Apple API |

“`store:false`”只表示 CyberKindred 不要求 Responses API 为后续 retrieve 存储 response；第三方的 abuse monitoring、计费、安全日志、账号设置和法定义务仍遵循其条款。隐私 UI 链接到实际 origin 的政策，不以本文件代替第三方政策。

## 4. Context assembly and minimization

发送 LLM 前按固定顺序构建：AI policy → 当次节目规则 → 本地时间/可选天气摘要 → 精简 profile → active approved memories → local-source 近期聚合反馈 → 本地模式的当次候选文本 → 当前会话必要 turns。每类设置硬上限，超出先本地选择或摘要；不把完整数据库交给模型。Apple Music/GSMTC metadata、session/media identity、capability、timeline 与 playback event 永不进入 Responses 或 Speech。Track-aware Apple 反应在本机由 deterministic 模板生成且仅显示；只有完全无 GSMTC 派生数据的通用段可 TTS。

用户输入、音乐 tag、provider metadata 与 LLM 历史输出均标为 data，不得进入 developer instruction。模型不能获得网络、文件、shell、screen 或 Windows media tools；其输出只是一份待 Rust schema/allowlist 验证的数据建议。

## 5. Retention and cleanup guarantees

- 过期时间在创建时写入 UTC epoch，不因读取、重新播放或时钟回拨而延期。启动与每日 cleanup 每批最多 500 行并释放 runtime。
- 对话到期前如无摘要，生成不含原文的 deterministic 最小摘要；删除正文后不保留可逆副本。被批准为 memory 的独立正文按 memory 规则管理。
- provider response body、Authorization、secret、绝对路径、屏幕/麦克风和系统全部会话清单历史从不持久化；operation outbox 也不得保存 chat/voice/model 正文或 GSMTC metadata。
- 删除从 active 数据路径立即生效。执行任一分类删除或全部重置时同步删除可能包含该类别旧值的 migration backup；删除完成后不保留可恢复应用副本。

## 6. User controls

用户可以查看并完成：修改 profile/location；关闭每个外部 provider；查看/批准/编辑/禁用/删除 memory；删除对话或摘要；清除收听历史；移除曲库索引；清 TTS/cover/weather cache；按 origin 删除 API Key；导出 MVP 数据包；全部重置。

API-041/042 的 `DataDeletionCategory` 仅有五项且集合固定：`profile_and_memories` 清画像个性化/位置并删除 preference trend、memory proposal/source、memory/revision；`conversations_and_summaries` 删除 chat session/message、active summary、deleted summary tombstone 与 source mapping；`playback_history` 删除 program/segment（含 voice text）、playback event/feedback 与派生统计；`metadata_cache` 删除 MusicBrainz/CAA positive-negative、cover、TTS、weather 和临时 provider cache；`library_index` 删除 root、scan job、track/tag/index/identity。每次分类删除同步删除可恢复该类的 migration backup 与关联 outbox，不删除 source music。Credentials、settings、schedule、usage、diagnostic logs 等不被冒充为第六个分类删除项，按各自行级控制或全部重置处理。

全部重置先停止节目、scan 与网络任务，撤销日程/notification/autostart，关闭 SQLite，再枚举并删除全部 `CyberKindred/provider/*` origin Credential items、SQLite/WAL/SHM、migration backup、cache、生成音频、outbox、logs 和应用设置，然后以空状态重启。操作立即且不可恢复，不使用 quarantine；失败时不报告成功。源音乐和用户主动保存到应用数据目录之外的 export 从不在删除目标内。

## 7. Export contract

MVP 导出是版本化 UTF-8 JSON/JSONL，且只含：manifest/schema version、非敏感 profile、approved memory、proposal status、active session summary、非敏感 settings（含 schedule rule）及 play/feedback records；每个 chat session 只含 covered-from/to 与 user/assistant message count。它不含 proposal 正文、deleted summary/tombstone、API Key/Authorization、raw chat body/hash、voice text、library root/index/tag/statistics、provider usage、绝对或相对音乐路径、原始/缓存音频、封面、TTS/cache、outbox、日志、backup、OS notification identifier、source message ID/hash 或 correlation ID。

导出通过 Rust-owned temporary path 生成，并由 native save picker 复制到用户选定位置；WebView 不接收临时绝对路径。复制成功或取消后立即删除临时文件。导出文件离开应用后由用户负责保管。

## 8. Verification

隐私测试使用 canary secret、canary absolute path、canary chat、音频标签与 GSMTC metadata，证明它们不会进入错误 DTO、日志、非目标 provider、outbox、event 或 export；Responses/Speech capture 中 GSMTC canary 匹配数必须为 0。冻结时钟覆盖 30 天 message/voice、90 天 proposal、7 天 weather、14 天 logs、24 小时 delivered outbox、7 天 undelivered outbox、30 天 backup/cache 与 13 个月 usage 的边界。分类删除验证 active state、outbox 引用与 backup 无法恢复对应 canary；全部重置还枚举所有 origin credentials，并扫描 SQLite/WAL/SHM、backup、cache、audio、logs/outbox/settings 路径，任何 canary 或可恢复副本匹配数必须为 0。真实 provider 测试只用合成数据，不使用真实个人对话或音乐路径。
