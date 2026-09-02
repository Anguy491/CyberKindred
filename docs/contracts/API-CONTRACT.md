# API Contract v1

| Field | Value |
|---|---|
| Status | Approved |
| Owner | Architecture Owner |
| Last Verified | 2026-09-03 |
| Source of Truth For | MVP 的 Tauri IPC command、event、版本、错误、超时与幂等语义 |
| Related Documents | `docs/contracts/PROVIDER-CONTRACTS.md`, `docs/contracts/schemas/`, `docs/architecture/ARCHITECTURE.md`, `docs/product/FRS.md` |

## 1. Boundary and transport

MVP 只有 Tauri WebView 与 Rust Core 之间的进程内 IPC；不启动 localhost server，不监听端口，也不提供外部 HTTP、WebSocket 或 deep-link API。React 只能调用本文件列出的 command，并只能订阅本文件列出的 event。Rust Core 是授权、路径、secret、数据库、网络和 Windows API 的执行边界。

所有名称均带 `v1`。请求和响应使用 UTF-8 JSON；时间使用带时区的 RFC 3339，持续时间和位置使用非负整数毫秒。字段采用 `camelCase`，枚举值采用 `snake_case`。缺失字段与显式 `null` 不等价；只有类型声明为 nullable 的字段可为 `null`。未知字段一律拒绝为 `ERR-1001`。

### 1.1 Success and failure

成功直接返回对应 response DTO。失败以 Tauri rejected invocation 返回唯一结构 `ApiError`，绝不把 provider 原文、secret、绝对路径、SQL、用户对话或 Rust backtrace 放入前端：

```ts
type ApiError = {
  schemaVersion: "1.0.0";
  errorId: ErrorId;
  safeMessage: string;          // 简体中文、1..300 字符
  retryable: boolean;
  retryAfterMs: number | null;
  correlationId: string;        // UUID；只用于关联脱敏日志
  details: {
    field: string | null;
    reason: string | null;
    currentRevision: number | null;
    capability: string | null;
    operationId: string | null;
  } | null;
};
```

`requestId`/`clientRequestId` 均为 UUID。写 command 在 Rust Core 保存最近 10 分钟的 `(command, clientRequestId, canonical request hash, result)`；相同 ID 与相同 payload 返回原结果，相同 ID 与不同 payload 返回 `ERR-1003`。媒体瞬时控制只保存 30 秒。超时表示调用方停止等待，不表示底层操作已取消；可取消操作必须显式调用 cancel command。

## 2. Shared DTOs

```ts
type EmptyRequest = Record<string, never>;
type Ack = { requestId: string; revision: number };
type OperationAccepted = { operationId: string; acceptedAt: string };

type PageRequest = { cursor: string | null; limit: number }; // limit 1..200
type Page<T> = { items: T[]; nextCursor: string | null };

type SourceCapabilities = {
  play: boolean; pause: boolean; seek: boolean;
  next: boolean; previous: boolean; setQueue: boolean;
};

type SourceSummary = {
  sourceId: string;
  kind: "local" | "system_session";
  displayName: string;
  connected: boolean;
  capabilities: SourceCapabilities;
};

type AppCapabilities = {
  protocolVersion: "1.0.0";
  appVersion: string;
  platform: "windows";
  osVersion: string;
  features: {
    localLibrary: true;
    systemMediaSession: boolean;
    musicKit: false;
    appleMusicDomControl: false;
    externalHttpApi: false;
  };
  sources: SourceSummary[];
  providers: Array<"llm" | "tts" | "metadata" | "weather">;
};
```

Canonical machine DTOs are `program-plan.schema.json`, `playback-state.schema.json`, `playback-event.schema.json`, `memory-record.schema.json`, `schedule-rule.schema.json` and `provider-error.schema.json`. Generated Rust and TypeScript types must validate against those files in contract tests.

## 3. Commands

`Timeout` is the caller deadline. “Idempotency” describes server behavior after retry.

### 3.1 App, onboarding, settings and secrets

| ID / command | Request | Response | Timeout | Idempotency |
|---|---|---|---:|---|
| API-001 `api_v1_get_capabilities` | `EmptyRequest` | `AppCapabilities` | 2 s | Read, idempotent |
| API-002 `api_v1_get_onboarding_state` | `EmptyRequest` | `OnboardingState` | 2 s | Read, idempotent |
| API-003 `api_v1_save_onboarding_step` | `{ clientRequestId; expectedRevision; submission: OnboardingStepSubmission }` | `Ack` | 5 s | 10-minute key |
| API-004 `api_v1_validate_and_set_secret` | `{ clientRequestId; kind: "openai_api_key"; origin; value: string }` | `{ requestId; configured: true; verifiedAt: string }` | 60 s | 10-minute key; validation success才写 Credential Manager |
| API-005 `api_v1_delete_secret` | `{ clientRequestId; kind: "openai_api_key"; origin }` | `{ requestId; configured: false }` | 5 s | 10-minute key；只删除该 canonical origin 的 credential |
| API-006 `api_v1_test_provider` | `{ clientRequestId; kind: "llm" | "tts" | "metadata" | "weather" }` | `{ requestId; ok: boolean; latencyMs: number; safeMessage: string }` | 60 s | 10-minute key |
| API-007 `api_v1_get_settings` | `EmptyRequest` | `SettingsView` | 2 s | Read, idempotent |
| API-008 `api_v1_update_settings` | `{ clientRequestId; expectedRevision; patch: SettingsPatch }` | `Ack` | 5 s；`llmModelId` 实际变化时 60 s | 10-minute key；model probe 成功后才原子保存整个 patch |
| API-009 `api_v1_preview_voice` | `{ clientRequestId; voiceId: string }` | `OperationAccepted` | 2 s accept / 45 s operation | 10-minute key；后端使用版本固定的中文短句，前端不能提供正文 |
| API-043 `api_v1_list_voices` | `{ provider: "tts" }` | `{ voices: Array<{ voiceId; displayName; previewAvailable: boolean }> }` | 5 s | Read, idempotent |
| API-048 `api_v1_search_weather_locations` | `{ clientRequestId; query: string; limit: number }` | `{ requestId; candidates: WeatherLocationCandidate[]; expiresAt }` | 10 s | 10-minute key；仅用户显式搜索时调用 |
| API-049 `api_v1_select_weather_location` | `{ clientRequestId; candidateId; expectedRevision }` | `{ requestId; location: WeatherLocation; revision }` | 5 s | 10-minute key；candidate 必须来自未过期搜索结果 |

`OnboardingStep` 的固定顺序和值为 `"welcome" | "music_source" | "openai_key" | "voice" | "profile" | "city_schedule" | "privacy"`。`OnboardingProfile` 是 `{ displayName: string; companionStyle: "quiet_warm"; initialPreferences: string[]; narrationDensity: "quiet" | "balanced" | "frequent" }`；`displayName` 为 0..80 字符（空字符串表示不使用称呼），`initialPreferences` 为 0..20 项且每项 1..100 字符。

`OnboardingState` 是精确 DTO `{ completed: boolean; completedSteps: OnboardingStep[]; sourceSelection: Array<"local" | "apple_music">; aiMode: "verified" | "local_only" | null; voiceMode: "selected" | "text_only" | null; cityScheduleMode: "configured" | "not_now" | null; profile: OnboardingProfile; privacyConfirmations: { explicitSound: boolean; rawConversationRetention: boolean }; revision: number }`。新安装返回中性默认画像、空 selections、两个 nullable mode、两个 false confirmation、revision 0。`completedSteps` 必须是固定顺序的无重复前缀；`completed` 只有在七步全部完成且两个 privacy confirmation 均为 true 时成立。

`OnboardingStepSubmission` 是按 `step` 判别且拒绝未知字段的联合：`{ step: "welcome" } | { step: "music_source"; sources: Array<"local" | "apple_music"> } | { step: "openai_key"; mode: "verified" | "local_only" } | { step: "voice"; mode: "selected" | "text_only" } | { step: "profile"; profile: OnboardingProfile } | { step: "city_schedule"; mode: "configured" | "not_now" } | { step: "privacy"; confirmations: { explicitSound: true; rawConversationRetention: true } }`。API-003 只允许提交当前首个未完成步骤或重新保存已完成步骤，不能跳步；保存成功原子更新 step data、`completedSteps` 和 onboarding revision。Profile submission 同事务更新 `user_profile` 与 `program.narration_density`，并单调增加 API-007/API-008 的 settings revision，使并发或仍持旧 revision 的 settings patch 以 `ERR-1003` 失败；onboarding revision 与 settings revision 仍是两个独立序列。`music_source` 至少选择一项且包含 `local` 时 API-010 必须已有 root；`verified`/`selected` 是用户已完成对应显式 provider 操作的选择记录，本身不得触发 provider 或声音；`local_only`/`text_only` 不得触发 provider。城市/日程仍只能由 API-048/API-049/API-033 保存，API-003 只记录该可选步骤为 `configured` 或 `not_now`，不能成为位置或日程的第二条 revision/validation 写入路径。窗口重启以 API-002 加各专用 read API 重建可编辑输入，不信任 WebView 缓存。

`WeatherLocationCandidate` is `{ candidateId; city; region: string | null; country; countryCode; latitude; longitude; timezone }`；`WeatherLocation` 是选定 candidate 去除 `candidateId` 后的同构值。latitude is -90..90, longitude -180..180, countryCode 为 ISO-3166-1 alpha-2。`query` 为用户主动输入的 2..100 字符，limit 为 1..10；单字符/空字符串由 API-048 以 `ERR-1001` 拒绝。No location permission or automatic GPS/IP lookup exists. `candidateId` 是 Rust 为一次搜索签发的 opaque ID，10 分钟后失效；API-049 只接受未过期且属于当前进程搜索结果的 ID。

`UserProfileView` 与 `OnboardingProfile` 是独立 DTO：`{ displayName; companionStyle; initialPreferences; narrationDensity; weatherLocation: WeatherLocation | null }`，共有画像字段沿用上述长度、枚举与数组上限。API-044 返回 `{ profile: UserProfileView; preferenceTrends: PreferenceTrend[]; revision }`。`ProfilePatch` 为 `{ displayName?: string; companionStyle?: "quiet_warm"; initialPreferences?: string[]; narrationDensity?: "quiet" | "balanced" | "frequent" }`；它没有位置字段。Patch 中缺失字段表示保持不变；这里列出的字段均不接受 `null`，空 patch 以 `ERR-1001` 拒绝。位置只能由 API-049 设置；清除已选位置使用 `SettingsPatch.weatherLocationAction: "clear"`，不得提交任意位置对象。

`SettingsView` is the exact non-secret DTO `{ providerOrigin: string; llmModelId: string; ttsModelId: string; ttsVoiceId: string; metadataEnabled: boolean; weatherEnabled: boolean; defaultSourceId: string | null; narrationDensity: "quiet" | "balanced" | "frequent"; ttsEnabled: boolean; audioOutputDeviceId: string | null; audioOutputBehavior: "follow_system_default" | "fixed_device"; minimizeToTray: boolean; launchAtStartup: boolean; notificationsEnabled: boolean; weatherLocation: WeatherLocation | null; secretStatus: { origins: Array<{ origin; openaiApiKeyConfigured: boolean; lastVerifiedAt: string | null }> }; integrationStatuses: Array<{ integration: "openai" | "apple_music" | "musicbrainz" | "weather"; state: "connected" | "degraded" | "disabled" | "unavailable"; lastSuccessAt: string | null; safeMessage: string }>; revision: number }`. It never contains secret material. `integrationStatuses` 只聚合已绑定到当前已提交配置的 provider outcome；API-004/API-008 候选验证的 usage fact 在设置事务提交前仅供审计，验证被拒绝、设置保存失败或事务回滚不得在当前进程或重启后改变已保存配置的 integration status。

`SettingsPatch` is the strict partial DTO `{ providerOrigin?: string; llmModelId?: string; ttsModelId?: string; ttsVoiceId?: string; metadataEnabled?: boolean; weatherEnabled?: boolean; defaultSourceId?: string | null; narrationDensity?: "quiet" | "balanced" | "frequent"; ttsEnabled?: boolean; audioOutputDeviceId?: string | null; audioOutputBehavior?: "follow_system_default" | "fixed_device"; minimizeToTray?: boolean; launchAtStartup?: boolean; notificationsEnabled?: boolean; weatherLocationAction?: "clear" }`。缺失字段保持不变；只有 `defaultSourceId` 与 `audioOutputDeviceId` 明确 nullable，分别表示每次询问来源与跟随系统默认设备。其他字段不接受 `null`，空 patch 以 `ERR-1001` 拒绝。MVP raw conversation retention 固定为创建后最多 30×24 小时，不提供延长或关闭清理的 Settings 字段；用户仍可即时分类删除或全部重置。`providerOrigin` 必须是用户确认的 HTTPS origin；`audioOutputBehavior: "fixed_device"` 要求当前或既有 `audioOutputDeviceId` 非 null。API-004 先以最小 provider 请求验证候选 key；失败时不覆盖现有有效 key，也不持久化候选值；验证成功并切换 origin 时保留其他 origin 已验证 credential，只有 API-005 可删除指定 origin，API-037 删除全部 `CyberKindred/provider/*` origin credential。API-008 只有在 `llmModelId` 与当前值不同时，才以 patch 合并后的候选 `providerOrigin`/`llmModelId` 和该 origin 已验证 credential 执行固定 Responses capability probe；该调用使用 60 秒 timeout，成功后原子保存整个 patch，失败时所有 patch 字段保持不变。不含实际 model 变化的 API-008 保持 5 秒且不得调用 provider；API-006 只测试当前已保存配置。

### 3.2 Local library

| ID / command | Request | Response | Timeout | Idempotency |
|---|---|---|---:|---|
| API-010 `api_v1_list_library_roots` | `EmptyRequest` | `{ roots: LibraryRoot[]; revision: number }` | 2 s | Read, idempotent |
| API-011 `api_v1_pick_and_add_library_root` | `{ clientRequestId }` | `{ requestId; root: LibraryRoot | null; revision }` | User-controlled picker | 10-minute key; cancel returns `root: null` |
| API-012 `api_v1_remove_library_root` | `{ clientRequestId; rootId; expectedRevision }` | `Ack` | 5 s | 10-minute key |
| API-013 `api_v1_start_library_scan` | `{ clientRequestId; rootIds: string[] }` | `OperationAccepted` | 2 s accept | 10-minute key |
| API-014 `api_v1_cancel_library_scan` | `{ clientRequestId; operationId }` | `{ requestId; operationId; state: "cancelled" | "already_terminal" }` | 2 s | Idempotent cancel |
| API-015 `api_v1_list_tracks` | `PageRequest & { query: string | null; sort: "title" | "artist" | "album" | "recent"; filters: TrackFilters }` | `Page<TrackView>` | 5 s | Read, idempotent |

`LibraryRoot` exposes `{ rootId, displayName, available }`; absolute paths never cross into WebView. `TrackFilters` is `{ availability: "playable" | "missing" | null; matchStatus: "matched" | "unmatched" | "review" | null }`；两个字段 AND 组合，null 表示不过滤。`TrackView` exposes `{ trackId; availability: "playable" | "missing" | "corrupt" | "unsupported"; durationMs; artworkAvailable; original: { title; artist; album }; enriched: { title; artist; album; provider: "musicbrainz"; confidence } | null; matchStatus: "matched" | "unmatched" | "review" }`, but no absolute path。搜索同时匹配 title、artist、album 且保留 filters。Empty `rootIds` in API-013 means all configured roots. Scan progress is delivered by EVT-005.

### 3.3 Playback and program

| ID / command | Request | Response | Timeout | Idempotency |
|---|---|---|---:|---|
| API-016 `api_v1_list_music_sources` | `EmptyRequest` | `{ sources: SourceSummary[] }` | 2 s | Read, idempotent |
| API-017 `api_v1_select_music_source` | `{ clientRequestId; sourceId }` | `{ requestId; state: PlaybackState }` | 5 s | 10-minute key |
| API-018 `api_v1_get_playback_state` | `EmptyRequest` | `PlaybackState` | 2 s | Read, idempotent |
| API-019 `api_v1_play` | `{ clientRequestId; expectedStateRevision }` | `PlaybackState` | 5 s | 30-second key; absolute state |
| API-020 `api_v1_pause` | `{ clientRequestId; expectedStateRevision }` | `PlaybackState` | 5 s | 30-second key; absolute state |
| API-021 `api_v1_seek` | `{ clientRequestId; expectedStateRevision; positionMs }` | `PlaybackState` | 5 s | 30-second key; absolute position |
| API-022 `api_v1_next` | `{ clientRequestId; expectedStateRevision }` | `PlaybackState` | 5 s | 30-second key required |
| API-023 `api_v1_previous` | `{ clientRequestId; expectedStateRevision }` | `PlaybackState` | 5 s | 30-second key required |
| API-024 `api_v1_start_program` | `{ clientRequestId; sourceId; trigger: "manual" | "notification" }` | `{ requestId; programId; plan: ProgramPlan | null }` | 60 s | 10-minute key |
| API-025 `api_v1_stop_program` | `{ clientRequestId; programId }` | `Ack` | 5 s | Idempotent stop |
| API-026 `api_v1_submit_chat` | `{ clientRequestId; programId; text }` | `OperationAccepted` | 2 s accept / 60 s operation | 10-minute key |
| API-027 `api_v1_submit_feedback` | `{ clientRequestId; programId; trackId: string | null; kind: "like" | "skip" | "less_talk" }` | `Ack` | 5 s | 10-minute key |
| API-038 `api_v1_cancel_operation` | `{ clientRequestId; operationId; expectedKind: "chat" | "voice_preview" | "library_scan" | "data_export" }` | `{ requestId; operationId; state: "cancelled" | "already_terminal" }` | 2 s | Idempotent cancel |

All playback mutation commands enforce the currently advertised capability; unsupported actions return `ERR-1201`. A stale `expectedStateRevision` returns `ERR-1003` and the current revision. Local sources may return a full `ProgramPlan`; a `system_session` source returns `plan: null`。Apple/GSMTC metadata、timeline 与 playback event 永不进入 Responses 或 Speech：track-aware 反应只由本机 deterministic 模板产生可见文字；只有完全不含 GSMTC 派生字段的通用段才可送 TTS。The app never chooses an arbitrary Apple Music catalog track.

### 3.4 Memory, schedule and data control

| ID / command | Request | Response | Timeout | Idempotency |
|---|---|---|---:|---|
| API-028 `api_v1_list_memories` | `PageRequest & { status: "proposed" | "approved" | "disabled" | null }` | `Page<MemoryRecord>` | 5 s | Read, idempotent；rejected 不返回 |
| API-029 `api_v1_approve_memory` | `{ clientRequestId; memoryId; expectedRevision }` | `MemoryRecord` | 5 s | 10-minute key |
| API-030 `api_v1_update_memory` | `{ clientRequestId; memoryId; expectedRevision; content; enabled }` | `MemoryRecord` | 5 s | 10-minute key |
| API-031 `api_v1_delete_memory` | `{ clientRequestId; memoryId; expectedRevision }` | `Ack` | 5 s | 10-minute key |
| API-039 `api_v1_reject_memory_proposal` | `{ clientRequestId; memoryId; expectedRevision }` | `{ requestId; memoryId; status: "rejected"; rejectedAt; contentDeleteAt; revision }` | 5 s | 10-minute key |
| API-044 `api_v1_get_profile_view` | `EmptyRequest` | `{ profile: UserProfileView; preferenceTrends: PreferenceTrend[]; revision }` | 5 s | Read, idempotent |
| API-045 `api_v1_update_profile` | `{ clientRequestId; expectedRevision; patch: ProfilePatch }` | `Ack` | 5 s | 10-minute key |
| API-046 `api_v1_list_session_summaries` | `PageRequest` | `Page<SessionSummaryView>` | 5 s | Read, idempotent |
| API-047 `api_v1_delete_session_summary` | `{ clientRequestId; summaryId; expectedRevision }` | `Ack` | 5 s | 10-minute key |
| API-032 `api_v1_list_schedules` | `EmptyRequest` | `{ schedules: ScheduleView[]; revision: number }` | 2 s | Read, idempotent |
| API-033 `api_v1_upsert_schedule` | `{ clientRequestId; expectedRevision; schedule: ScheduleRule }` | `{ requestId; schedule: ScheduleView; revision }` | 5 s | 10-minute key |
| API-034 `api_v1_delete_schedule` | `{ clientRequestId; scheduleId; expectedRevision }` | `Ack` | 5 s | 10-minute key |
| API-035 `api_v1_handle_notification_action` | `NotificationActionRequest` | `{ requestId; occurrenceId; status: "awaiting_user" | "snoozed" | "starting" | "dismissed"; nextNotificationAt: string | null; revision }` | 5 s | 10-minute key |
| API-036 `api_v1_export_user_data` | `{ clientRequestId }` | `OperationAccepted` | 2 s accept | 10-minute key; native save picker |
| API-037 `api_v1_delete_all_user_data` | `{ clientRequestId; confirmation: "DELETE CYBERKINDRED DATA" }` | `{ requestId; restartRequired: true }` | 30 s | 10-minute key |
| API-040 `api_v1_get_data_inventory` | `EmptyRequest` | `{ generatedAt; categories: DataCategoryInventory[] }` | 5 s | Read, idempotent |
| API-041 `api_v1_preview_data_deletion` | `{ category: DataDeletionCategory }` | `{ previewToken; expiresAt; category; itemCount; consequences: string[] }` | 5 s | Read-like；token 5 分钟有效 |
| API-042 `api_v1_delete_data_category` | `{ clientRequestId; previewToken; category: DataDeletionCategory; confirmation: "DELETE SELECTED DATA" }` | `{ requestId; category; deletedCount; restartRequired }` | 30 s | 10-minute key |

`NotificationActionRequest` is `{ clientRequestId; scheduleId; occurrenceId; action: "open" | "dismiss" | "start" | "snooze"; snoozeMinutes: 10 | 30 | 60 | null }`。`snooze` 必须携带非 null `snoozeMinutes`；其他 action 必须为 null。对同一 occurrence 的新 snooze 原子替换旧 snooze，不修改重复 rule。`ScheduleView` 是 `{ rule: ScheduleRule; nextOccurrenceAt: string | null }`。

`PreferenceTrend` 只暴露有界聚合 `{ kind; label; direction: "up" | "stable" | "down"; sampleCount; windowDays }`；`SessionSummaryView` 为 `{ summaryId; coveredFrom; coveredTo; summary; generationKind; revision }`。拒绝提案后立即从 API-028 消失、永不进入 context，正文按数据模型在 30 天内清空，仅保留 hash/status/time 审计；它不会转成 `MemoryRecord` 的长期状态。

`DataDeletionCategory` 只允许五个值：`"profile_and_memories" | "conversations_and_summaries" | "playback_history" | "metadata_cache" | "library_index"`。`DataInventoryCategory` 则覆盖本地生命周期清单的每个数据类：`"credentials" | "profile_and_preferences" | "weather_location_and_cache" | "library_roots_and_identity" | "embedded_music_tags" | "metadata_matches" | "artwork_and_tts_cache" | "system_media_runtime" | "playback_history_and_feedback" | "chat_messages" | "voice_segment_text" | "session_summaries" | "memory_proposals" | "approved_memories_and_revisions" | "schedules_and_notifications" | "provider_usage_facts" | "operation_outbox" | "diagnostic_logs" | "migration_backups"`。两套 enum 不得互换。

`DataCategoryInventory` is `{ category: DataInventoryCategory; itemCount: number; storageClasses: Array<"memory" | "credential_manager" | "sqlite" | "app_data" | "app_cache" | "windows_task" | "windows_notification">; retentionSummary: string; externalRecipients: string[]; deletionControl: "category" | "credential" | "automatic" | "reset_only"; deletionCategory: DataDeletionCategory | null }`。`storageClasses` 至少一项、去重，可表达同一数据类横跨 SQLite/cache/Windows 设施；响应不返回 absolute path、正文或 secret。只有上述五类接受 API-041/042；credential 使用 API-005 或 API-037，自动保留类按生命周期清理，`reset_only` 类只由全部重置移除。

五类删除集合固定如下：`profile_and_memories` 清画像个性化/位置并删除 preference trend、memory proposal/source、approved/disabled memory 与 revision；`conversations_and_summaries` 删除 chat session/message、active summary、deleted summary tombstone 与来源映射；`playback_history` 删除 program/segment（含 voice text）、playback event、feedback 与派生近期统计；`metadata_cache` 删除 MusicBrainz/CAA positive-negative、封面、TTS、天气与临时 provider cache；`library_index` 删除授权 root、scan job、track index、嵌入 tag 索引与派生 identity。分类删除必须先取得匹配 token；`library_index` 和 `metadata_cache` 只删应用派生数据，任何类别都不调用源音乐删除 API。每次分类删除还删除可能恢复该类旧值的 migration backup，并 scrub 关联 outbox；diagnostic log 按设计不含正文/路径/secret，不能作为恢复源。

Schedules are notification-only by schema; OS notification action `start` is an explicit user confirmation and may then call API-024. API-036 的版本化 UTF-8 JSON/JSONL 固定且仅包含 manifest/schema version、非敏感 profile、approved memory、proposal status、active session summary、非敏感 settings（含 schedule rule）以及 play/feedback records；每个 chat session 只含 covered-from/to 与 user/assistant message count。它排除 proposal 正文、deleted summary/tombstone、secret、raw chat body/hash、voice text、library root/index/tag/statistics、provider usage、原始/缓存音频、封面、TTS/cache、绝对/相对音乐路径、OS notification identifier、logs、backup、outbox 与 correlation ID。API-037 不可恢复地删除 SQLite/WAL/SHM、全部 migration backup、cache、生成音频、logs、outbox、应用设置以及 Credential Manager 中所有 `CyberKindred/provider/*` origin credentials，取消 schedule/notification/autostart，preserves installed binaries, and never deletes source music；成功后通过重启空状态、凭据枚举无匹配项、canary 文件/数据库/日志/backup 扫描证明不存在应用级恢复副本。

## 4. Events

Events use Tauri `emit` and are process-local. Subscribers must treat them as hints and re-read state after a sequence gap. Each payload includes `schemaVersion: "1.0.0"`, an increasing process-local `sequence`, and `occurredAt`; sequence resets on app restart. 每个 accepted operation 在 Rust 权威状态中只能从 accepted 原子转换为 completed、failed 或 cancelled 之一，且只能转换一次。持久化 operation terminal 通过 outbox 重放，因此 EVT-005/008/009/011 的 transport 是 at-least-once：崩溃窗口或恢复可再次投递同一 `operationId` 的同一权威 terminal。前端必须按 `operationId` 幂等去重；重复投递不是第二个权威结果。

| ID / event name | Payload | Delivery semantics |
|---|---|---|
| EVT-001 `cyberkindred://v1/playback/event` | `PlaybackEvent` | At-most-once; coalescing allowed only for position-only state changes |
| EVT-002 `cyberkindred://v1/program/state` | `{ schemaVersion; sequence; occurredAt; programId; state: "planning" | "running" | "paused" | "stopping" | "completed" | "failed"; safeMessage: string | null }` | Coarse user-visible projection；相邻内部状态可合并 |
| EVT-003 `cyberkindred://v1/program/segment` | `{ schemaVersion; sequence; occurredAt; programId; segmentId; state: "queued" | "playing" | "completed" | "skipped" | "failed" }` | Coarse user-visible projection；相邻内部状态可合并 |
| EVT-004 `cyberkindred://v1/chat/message` | `{ schemaVersion; sequence; occurredAt; operationId; programId; role: "user" | "assistant"; text; final: boolean }` | Ordered per operation; final exactly once on success |
| EVT-005 `cyberkindred://v1/library/scan` | `{ schemaVersion; sequence; occurredAt; operationId; state: "running" | "completed" | "cancelled" | "failed"; scanned; discovered; failed; safeMessage: string | null }` | Progress may coalesce；one authoritative terminal；terminal transport at-least-once；consumer dedupes `operationId` |
| EVT-006 `cyberkindred://v1/memory/proposed` | `{ schemaVersion; sequence; occurredAt; memory: MemoryRecord }` | At-most-once; UI re-queries on focus |
| EVT-007 `cyberkindred://v1/schedule/due` | `{ schemaVersion; sequence; occurredAt; scheduleId; occurrenceId; notificationShown: boolean }` | At-most-once per occurrence |
| EVT-008 `cyberkindred://v1/operation/completed` | `{ schemaVersion; sequence; occurredAt; operationId; kind: "voice_preview" | "data_export"; outputLabel: string | null }` | One authoritative success；at-least-once transport；consumer dedupes `operationId` |
| EVT-009 `cyberkindred://v1/operation/failed` | `{ schemaVersion; sequence; occurredAt; operationId; error: ApiError }` | One authoritative failure；at-least-once transport；consumer dedupes `operationId` |
| EVT-010 `cyberkindred://v1/app/resumed` | `{ schemaVersion; sequence; occurredAt; sleptAt: string | null }` | Emitted after resume reconciliation |
| EVT-011 `cyberkindred://v1/operation/cancelled` | `{ schemaVersion; sequence; occurredAt; operationId; kind: "chat" | "voice_preview" | "library_scan" | "data_export" }` | One authoritative cancellation；at-least-once transport；consumer dedupes `operationId` |

EVT-002/003 只是内部状态机面向 UI 的 coarse projection，不是逐转换审计流；订阅者不得以缺少中间 event 推断非法转换，必须在 sequence gap、resume 或 terminal 之后通过 read command 重取权威状态。对 EVT-005/008/009/011，订阅者先按 `operationId` 去重再应用 terminal UI side effect；同一 operation 的相同 terminal 重复投递必须成为 no-op，若观察到不同 terminal kind 则停止应用增量并重取权威状态。No event contains secret material or absolute local paths. EVT-004 assistant text is user-visible conversation content and must not be written to diagnostic logs. Apple/GSMTC track metadata、timeline、capability payload 与 playback event 不进入 EVT-004、EVT-006、EVT-008/009 的 provider-derived content；Apple track-aware 文本只能是本机 deterministic 产物。

API-038 成功后该 operation 只可产生 EVT-011（library scan 另产生 EVT-005 `cancelled`）；已在 UI/SQLite 接受的用户 chat 原文保留，但 late provider output 被丢弃，不产生 assistant final message、Memory Proposal、summary、TTS 或 playback side effect。

## 5. Error registry

| ID | Meaning | Retry policy |
|---|---|---|
| ERR-1001 | Request/schema validation failed | Fix request; never automatic retry |
| ERR-1002 | Unsupported protocol version | Upgrade client/app |
| ERR-1003 | Revision or idempotency conflict | Re-read state; retry with a new request ID only after user intent is preserved |
| ERR-1004 | Entity not found | Re-read collection |
| ERR-1005 | Resource busy | Retry with bounded backoff |
| ERR-1006 | Operation cancelled | No automatic retry |
| ERR-1101 | Required secret not configured | Ask user to configure |
| ERR-1102 | Secret rejected by provider | Ask user to replace; never echo it |
| ERR-1201 | Capability not supported | Hide/disable action until capabilities change |
| ERR-1202 | Music source unavailable | Reconnect/reselect source |
| ERR-1203 | System media session changed during operation | Re-read and require fresh intent |
| ERR-1204 | Media file or metadata invalid | Skip item; surface safe message |
| ERR-1301 | Provider authentication failed | Non-retryable until credentials/config changes |
| ERR-1302 | Provider rate limited | Retry after `retryAfterMs` with jitter |
| ERR-1303 | Provider timed out | At most two bounded retries for read/generation; never duplicate side effects |
| ERR-1304 | Provider/network unavailable | Use documented degradation; bounded retry |
| ERR-1305 | Provider response failed validation | Reject output; one repair attempt for LLM only |
| ERR-1401 | SQLite/storage operation failed | Stop affected write; preserve source data |
| ERR-1402 | Database migration failed | Do not open writable app state; run recovery flow |
| ERR-1501 | Path access denied or outside approved root | Ask user to select through native picker |
| ERR-1502 | Destructive confirmation missing | Require exact confirmation phrase |
| ERR-1601 | Unexpected internal error | No blind retry; use correlation ID |

Provider errors crossing IPC must also validate against `provider-error.schema.json`. The provider-specific ID is mapped to the public `ApiError` without leaking upstream bodies.

### 5.1 Internal reason mapping

Rust 内部只能把以下稳定、安全的 `reason` code 写入 `ApiError.details.reason`；provider 原文、OS HRESULT、SQL 与 parser message 只进入已脱敏本机诊断字段，不跨 IPC。映射按首个命中的最具体原因执行，不允许调用点自行选择另一个 public error：

| Internal reason | Public error |
|---|---|
| `request_invalid`, `unknown_field`, `invalid_patch`, `invalid_candidate` | `ERR-1001` |
| `protocol_unsupported` | `ERR-1002` |
| `revision_conflict`, `idempotency_payload_conflict`, `preview_token_stale` | `ERR-1003` |
| `entity_not_found`, `operation_not_found` | `ERR-1004` |
| `resource_busy`, `operation_already_running` | `ERR-1005` |
| `operation_cancelled` | `ERR-1006` |
| `secret_missing` | `ERR-1101` |
| `secret_validation_rejected` | `ERR-1102` |
| `capability_absent` | `ERR-1201` |
| `source_unavailable` | `ERR-1202` |
| `session_identity_changed`, `state_revision_changed`, `user_media_override` | `ERR-1203` |
| `media_unreadable`, `media_metadata_invalid` | `ERR-1204` |
| `provider_authentication` | `ERR-1301` |
| `provider_rate_limit` | `ERR-1302` |
| `provider_timeout` | `ERR-1303` |
| `provider_unavailable`, `network_unavailable` | `ERR-1304` |
| `provider_invalid_response`, `provider_output_policy_violation` | `ERR-1305` |
| `storage_read_failed`, `storage_write_failed`, `storage_integrity_failed` | `ERR-1401` |
| `migration_failed`, `database_version_unsupported` | `ERR-1402` |
| `path_denied`, `path_outside_root`, `unsafe_reparse_point` | `ERR-1501` |
| `destructive_confirmation_missing`, `destructive_confirmation_mismatch` | `ERR-1502` |
| `unexpected_internal` | `ERR-1601` |

没有表项的内部错误在 release build 统一映射 `unexpected_internal` / `ERR-1601`；测试必须枚举所有内部 error variant，证明不存在隐式 fall-through。Provider `ProviderError.category` 的五类映射固定为 `authentication`→`ERR-1301`、`rate_limit`→`ERR-1302`、`timeout`→`ERR-1303`、`unavailable`→`ERR-1304`、`invalid_response`→`ERR-1305`；只有 API-004 验证候选 credential 时将 `authentication` 特化为 `secret_validation_rejected` / `ERR-1102`。

## 6. Capability and compatibility rules

- `protocolVersion` changes major only for a breaking field/name/semantic change. Additive optional behavior retains major `1`; machine schemas remain `1.0.0` until a coordinated schema revision.
- The frontend checks API-001 before rendering controls. A control is enabled only when both app feature and selected source capability are true.
- GSMTC capabilities are runtime facts, not assumptions about Apple Music. Capability changes emit EVT-001 with type `capabilities_changed`.
- `setQueue` is true only for local playback in MVP. `musicKit`, `appleMusicDomControl` and `externalHttpApi` are always false.
- Unknown event types or newer major versions are ignored and trigger a capabilities refresh; unknown fields within v1 DTOs fail contract tests and are not emitted.

## 7. Contract enforcement

Rust owns validation before side effects and validates provider output again before persistence or playback. TypeScript types are generated from the same schema artifacts but are not a security boundary. Contract tests must parse all six schemas, accept every `.valid.json` and `.boundary.json`, reject every `.invalid.json`, and assert command serialization plus error redaction.

`TASK-005` 固定由 `scripts/generate-contracts.mjs` 从六份 schema 生成 `src/contracts/generated.ts` 与 `src-tauri/src/contracts/generated.rs`；`tests/contracts/schema-baseline.json` 保存逐 schema version、`$id`、canonical SHA-256 和 bundle SHA-256。`pnpm contracts:check` 在内存重生成并逐字检查双语言输出，同版本 schema 内容漂移或未经审阅的 baseline 变化立即失败。Rust `ContractRegistry` 关闭 `jsonschema` 的 HTTP/file resolver，只使用内存注册的六个 `$id`，按 Draft 2020-12、format assertion 与 unknown-format fail-closed 在 typed deserialization 和副作用前校验；错误不格式化提交内容。AJV 只存在于 Node contract suite，不进入 `src/` runtime import 或 WebView bundle。command serialization、统一 `ApiError` 与 IPC redaction 由依赖任务 `TASK-006` 补齐，不以 schema 子集通过代替整个传输契约门槛。
