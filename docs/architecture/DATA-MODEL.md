# CyberKindred 数据模型

| Metadata | Value |
|---|---|
| Status | Approved |
| Owner | Data Architecture |
| Last Verified | 2026-09-03 |
| Source of Truth For | SQLite schema、字段、键、索引、迁移、数据目录、保留、导出与删除规则 |
| Related Documents | [FRS](../product/FRS.md), [NFRS](../product/NFRS.md), [Architecture](ARCHITECTURE.md), [AI Orchestration](AI-ORCHESTRATION.md), [Privacy Data Lifecycle](../security/PRIVACY-DATA-LIFECYCLE.md), [Memory Schema](../contracts/schemas/memory-record.schema.json) |

## 1. 存储原则

- SQLite 仅保存单个 Windows 用户的非 secret 应用数据。OpenAI API Key 及未来 provider token **禁止进入 SQLite、配置文件、日志、导出包或 WebView**；它们只存 Windows Credential Manager。
- 表名/列名使用 `snake_case`；主键使用应用生成的 UUIDv7 小写文本，事件/消息按 UUIDv7 可近似时间排序。
- 时间均为 `INTEGER` Unix epoch milliseconds UTC；日程规则额外保存 IANA timezone 与 local wall-clock 字段。duration/position 使用整数毫秒。
- 布尔值为 `INTEGER NOT NULL CHECK(value IN (0,1))`；枚举使用 `TEXT CHECK(...)`；JSON 列必须通过 `json_valid`，且只保存契约定义字段。
- 用户音乐保持原位且只读。数据库保存 library root 的 canonical absolute path 与 track 的相对路径；日志、遥测、错误 DTO 不输出 absolute path。
- 所有业务写入使用事务。外部网络调用和音频操作不得持有数据库事务。

## 2. 数据目录

路径由 Tauri path resolver 获取，不在代码中拼接用户名：

```text
AppData/                       # Tauri app_local_data_dir（Windows per-user local data）
  cyberkindred.sqlite3
  backups/
    pre-migration-v{n}-{utc}.sqlite3
  exports/                     # 仅用户显式导出时创建；成功交付后清理临时副本
Cache/                         # Tauri app_cache_dir
  tts/{sha256[0..2]}/{sha256}.mp3
  covers/{sha256[0..2]}/{sha256}.{ext}
  staging/                     # 原子落盘前临时文件；启动清理超过24小时的文件
Logs/                          # Tauri app_log_dir；结构化且脱敏
```

数据库不得放在安装目录或音乐目录。备份、cache 和 export 路径不得由 WebView 直接提供；Rust 生成后可通过系统保存对话框将 export 原子复制到用户选定位置。

## 3. SQLite 配置

每个连接强制：`PRAGMA foreign_keys=ON`、`journal_mode=WAL`、`synchronous=NORMAL`、`busy_timeout=3000`、`temp_store=MEMORY`。写入经单一 writer pool（最大 1 个写连接）串行；读取池最多 4 个连接。应用正常退出执行 passive checkpoint；打包/备份前执行 truncate checkpoint。

`PRAGMA application_id` 固定为项目分配的十进制 `1129008708`（十六进制 `0x434B4E44`，ASCII `CKND`），`user_version` 等于最新 schema version。打开其他 application ID 的数据库必须拒绝，不自动改写。

## 4. Schema 与字段

### 4.1 迁移与设置

#### `schema_migrations`

| 列 | 类型/约束 | 说明 |
|---|---|---|
| `version` | `INTEGER PRIMARY KEY` | 单调递增迁移版本 |
| `name` | `TEXT NOT NULL UNIQUE` | 不可变迁移名 |
| `checksum_sha256` | `TEXT NOT NULL CHECK(length=64)` | 编译进二进制的 SQL checksum |
| `applied_at_ms` | `INTEGER NOT NULL` | 完成时间 |
| `app_version` | `TEXT NOT NULL` | 执行迁移的应用版本 |

#### `app_settings`

| 列 | 类型/约束 | 说明 |
|---|---|---|
| `key` | `TEXT PRIMARY KEY` | allowlist key；Credential/API key/token/password/Authorization 名称均被拒绝 |
| `value_json` | `TEXT NOT NULL CHECK(json_valid(value_json))` | 非敏感 typed setting |
| `schema_version` | `INTEGER NOT NULL` | 单项值 schema |
| `updated_at_ms` | `INTEGER NOT NULL` | 更新时间 |

允许的 key namespace：`ui.*`、`audio.*`、`provider.openai.model`、`provider.openai.base_url`、`provider.openai.tts_*`、`program.*`、`privacy.*`、`os.tray`、`os.autostart`、`os.notifications`。`base_url` 保存前只允许 HTTPS（测试构建可显式 allow localhost）。

#### `user_profile`

单例行 `id='current'`：`display_name TEXT`、`locale TEXT NOT NULL DEFAULT 'zh-CN'`、`city TEXT`、`region TEXT`、`country TEXT`、`country_code TEXT CHECK(country_code IS NULL OR (length(country_code)=2 AND country_code=upper(country_code)))`、`city_lat REAL CHECK(city_lat BETWEEN -90 AND 90)`、`city_lon REAL CHECK(city_lon BETWEEN -180 AND 180)`、`timezone TEXT NOT NULL`、`routine_json TEXT NOT NULL CHECK(json_valid(routine_json))`、`program_preferences_json TEXT NOT NULL CHECK(json_valid(program_preferences_json))`、`profile_revision INTEGER NOT NULL`、`created_at_ms INTEGER NOT NULL`、`updated_at_ms INTEGER NOT NULL`。city/region/country/country_code/latitude/longitude/timezone 与选中 `WeatherLocation` 逐字段对应；精确 GPS 不采集，未选择的 search query/candidate 不入库。

### 4.2 曲库与元数据

#### `library_roots`

`id TEXT PRIMARY KEY`、`canonical_path TEXT NOT NULL`、`path_key TEXT NOT NULL UNIQUE`（Windows case-fold + separator normalization）、`display_name TEXT NOT NULL`、`enabled INTEGER NOT NULL CHECK(enabled IN (0,1))`、`created_at_ms INTEGER NOT NULL`、`last_scan_at_ms INTEGER`。删除 root 默认软禁用；用户确认“移除索引”才级联删除 track 索引，永不删除原音乐。

#### `scan_jobs`

`id TEXT PRIMARY KEY`、`root_id TEXT NOT NULL REFERENCES library_roots(id)`、`status TEXT NOT NULL CHECK(status IN ('queued','running','completed','cancelled','failed','interrupted'))`、`files_seen INTEGER NOT NULL DEFAULT 0`、`tracks_indexed INTEGER NOT NULL DEFAULT 0`、`errors_count INTEGER NOT NULL DEFAULT 0`、`started_at_ms INTEGER`、`finished_at_ms INTEGER`、`error_code TEXT`、`app_version TEXT NOT NULL`。partial unique index `ux_scan_one_active_root(root_id) WHERE status IN ('queued','running')`。

#### `tracks`

| 列组 | 字段 |
|---|---|
| 身份/路径 | `id TEXT PRIMARY KEY`; `root_id TEXT NOT NULL REFERENCES library_roots(id) ON DELETE CASCADE`; `relative_path TEXT NOT NULL`; `relative_path_key TEXT NOT NULL`; `file_identity TEXT`; unique `(root_id, relative_path_key)` |
| 文件状态 | `availability TEXT NOT NULL CHECK(availability IN ('available','missing','corrupt','unsupported'))`; `format TEXT NOT NULL`; `file_size_bytes INTEGER NOT NULL CHECK(file_size_bytes>=0)`; `modified_at_ms INTEGER NOT NULL`; `duration_ms INTEGER NOT NULL CHECK(duration_ms>=0)`; `last_seen_scan_id TEXT REFERENCES scan_jobs(id)` |
| 原始标签 | `title TEXT`; `artist TEXT`; `album TEXT`; `album_artist TEXT`; `track_number INTEGER`; `disc_number INTEGER`; `year INTEGER`; `genre_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(genre_json))` |
| 派生数据 | `metadata_confidence REAL NOT NULL CHECK(metadata_confidence BETWEEN 0 AND 1)`; `embedded_cover_hash TEXT` |
| 时间 | `created_at_ms INTEGER NOT NULL`; `updated_at_ms INTEGER NOT NULL`; `last_played_at_ms INTEGER` |

索引：`ix_tracks_root_availability(root_id,availability)`、`ix_tracks_artist(artist)`、`ix_tracks_album(album)`、`ix_tracks_last_played(last_played_at_ms)`、`ix_tracks_seen_scan(last_seen_scan_id)`。`relative_path` 必须经 Rust containment 校验后才能与 root 合成路径。

#### `track_external_metadata`

`id TEXT PRIMARY KEY`、`track_id TEXT NOT NULL REFERENCES tracks(id) ON DELETE CASCADE`、`provider TEXT NOT NULL CHECK(provider='musicbrainz')`、`external_id TEXT`（MBID）、`normalized_title TEXT`、`normalized_artist TEXT`、`normalized_album TEXT`、`tags_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(tags_json))`、`confidence REAL NOT NULL CHECK(confidence BETWEEN 0 AND 1)`、`match_status TEXT NOT NULL CHECK(match_status IN ('adopted','suggested','no_match'))`、`response_etag TEXT`、`fetched_at_ms INTEGER NOT NULL`、`expires_at_ms INTEGER`、unique `(track_id,provider)`。原始 provider body 不保存。

#### `cover_art_cache`

`cache_key TEXT PRIMARY KEY`（SHA-256）、`track_id TEXT REFERENCES tracks(id) ON DELETE SET NULL`、`source TEXT NOT NULL CHECK(source IN ('embedded','cover_art_archive'))`、`external_url TEXT`、`mime_type TEXT NOT NULL`、`width INTEGER`、`height INTEGER`、`size_bytes INTEGER NOT NULL CHECK(size_bytes>=0)`、`etag TEXT`、`attribution_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(attribution_json))`、`last_accessed_at_ms INTEGER NOT NULL`、`created_at_ms INTEGER NOT NULL`。文件丢失时删除该行并可再生，不影响 track。

#### `cover_art_negative_cache`

`release_mbid TEXT PRIMARY KEY`、`result TEXT NOT NULL CHECK(result='not_found')`、`fetched_at_ms INTEGER NOT NULL`、`expires_at_ms INTEGER NOT NULL`、`provider_status INTEGER NOT NULL CHECK(provider_status=404)`。仅表示经验证 release MBID 的 CAA 404；不存 HTML/body，24 小时后删除。网络、5xx、重定向或格式错误不得写入负缓存。

#### `tts_cache_entries`

`cache_key TEXT PRIMARY KEY`（model/voice/speed/format/locale/normalized text 的 SHA-256）、`relative_path TEXT NOT NULL UNIQUE`、`model_id TEXT NOT NULL`、`voice_id TEXT NOT NULL`、`speed REAL NOT NULL CHECK(speed BETWEEN 0.75 AND 1.25)`、`format TEXT NOT NULL CHECK(format='mp3')`、`locale TEXT NOT NULL`、`text_hash TEXT NOT NULL`、`size_bytes INTEGER NOT NULL CHECK(size_bytes BETWEEN 1 AND 20971520)`、`duration_ms INTEGER NOT NULL CHECK(duration_ms BETWEEN 1 AND 120000)`、`created_at_ms INTEGER NOT NULL`、`last_accessed_at_ms INTEGER NOT NULL`、`expires_at_ms INTEGER NOT NULL`。`relative_path` 只能由 Rust 根据 cache key 推导；entry 在 30 天、512 MiB quota 或 LRU 先到者清理，但 active lease 阻止删除。

#### `tts_cache_references`

`cache_key TEXT NOT NULL REFERENCES tts_cache_entries(cache_key) ON DELETE CASCADE`、`owner_kind TEXT NOT NULL CHECK(owner_kind IN ('program_segment','voice_preview'))`、`owner_id TEXT NOT NULL`、`created_at_ms INTEGER NOT NULL`、primary key `(cache_key,owner_kind,owner_id)`。program segment 引用随 segment 删除；voice preview 使用 API operation ID，operation terminal 后立即删引用。entry 可在无 reference 时继续作为可再生 LRU cache。

#### `tts_cache_leases`

`lease_id TEXT PRIMARY KEY`、`cache_key TEXT NOT NULL REFERENCES tts_cache_entries(cache_key) ON DELETE CASCADE`、`owner_kind TEXT NOT NULL CHECK(owner_kind IN ('program_segment','voice_preview'))`、`owner_id TEXT NOT NULL`、`process_instance_id TEXT NOT NULL`、`acquired_at_ms INTEGER NOT NULL`、`expires_at_ms INTEGER NOT NULL`。播放/preview 打开 artifact 前取得 lease，完成/取消后释放；启动时删除其他 process instance 或已过期 lease。清理 job 只删除没有有效 lease 的 entry。

### 4.3 节目与播放事实

#### `program_runs`

`id TEXT PRIMARY KEY`、`source_kind TEXT NOT NULL CHECK(source_kind IN ('local','system_session'))`、`source_instance_id TEXT`、`status TEXT NOT NULL CHECK(status IN ('planning','ready','music','voice_preparing','voice','paused','degraded','completing','stopping','completed','interrupted','failed'))`、`target_duration_ms INTEGER CHECK(target_duration_ms>0)`、`started_at_ms INTEGER`、`ended_at_ms INTEGER`、`plan_schema_version INTEGER NOT NULL`、`prompt_version TEXT`、`provider TEXT`、`model TEXT`、`degraded_features_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(degraded_features_json))`、`failure_code TEXT`、`revision INTEGER NOT NULL DEFAULT 0`、`created_at_ms INTEGER NOT NULL`。status 持久化 [Runtime State Machines](RUNTIME-STATE-MACHINES.md#41-public-event-coarse-projection) 的内部 Program state（数据库使用 snake_case）并按唯一表投影为 EVT-002；索引 `ix_program_runs_started(started_at_ms DESC)`、`ix_program_runs_status(status)`。

#### `program_segments`

`id TEXT PRIMARY KEY`、`program_run_id TEXT NOT NULL REFERENCES program_runs(id) ON DELETE CASCADE`、`ordinal INTEGER NOT NULL CHECK(ordinal>=0)`、`kind TEXT NOT NULL CHECK(kind IN ('track','voice'))`、`track_id TEXT REFERENCES tracks(id) ON DELETE SET NULL`、`system_media_identity TEXT`、`voice_text TEXT`、`voice_text_hash TEXT`、`tts_cache_key TEXT REFERENCES tts_cache_entries(cache_key) ON DELETE SET NULL`、`status TEXT NOT NULL CHECK(status IN ('planned','preparing','active','completed','skipped','failed','cancelled'))`、`started_at_ms INTEGER`、`ended_at_ms INTEGER`、`failure_code TEXT`、unique `(program_run_id,ordinal)`。另设 table CHECK：`kind='track'` 时 `track_id`/`system_media_identity` 恰有一个非 NULL；`kind='voice'` 时创建时 `voice_text` 非 NULL，保留清理后允许为 NULL。`voice_text` 在 30 天后置 NULL，hash 保留用于诊断去重；创建/替换 `tts_cache_key` 时同步维护 `tts_cache_references(owner_kind='program_segment')`，避免孤立 ownership。

#### `playback_events`

`id TEXT PRIMARY KEY`、`program_run_id TEXT REFERENCES program_runs(id) ON DELETE SET NULL`、`source_kind TEXT NOT NULL CHECK(source_kind IN ('local','system_session'))`、`source_instance_id TEXT`、`track_id TEXT REFERENCES tracks(id) ON DELETE SET NULL`、`system_media_identity TEXT`、`display_title TEXT CHECK(display_title IS NULL OR length(display_title) BETWEEN 1 AND 300)`、`display_artist TEXT CHECK(display_artist IS NULL OR length(display_artist) BETWEEN 1 AND 300)`、`display_album TEXT CHECK(display_album IS NULL OR length(display_album) BETWEEN 1 AND 300)`、`event_type TEXT NOT NULL CHECK(event_type IN ('started','paused','resumed','seeked','completed','skipped','failed','source_changed'))`、`position_ms INTEGER CHECK(position_ms>=0)`、`reason_code TEXT`、`occurred_at_ms INTEGER NOT NULL`、`playback_revision INTEGER NOT NULL CHECK(playback_revision>=0)`。system-session event 可保存事件发生时本机 GSMTC 提供的三个 display 字段 snapshot；它们只用于本机历史/UI/导出，禁止进入 Responses、Speech、metadata 或 weather provider。索引 `ix_playback_events_run_time(program_run_id,occurred_at_ms)`、`ix_playback_events_track_time(track_id,occurred_at_ms DESC)`、`ix_playback_events_media_time(system_media_identity,occurred_at_ms DESC)`。

#### `feedback`

`id TEXT PRIMARY KEY`、`program_run_id TEXT REFERENCES program_runs(id) ON DELETE SET NULL`、`target_kind TEXT NOT NULL CHECK(target_kind IN ('track','voice','program'))`、`track_id TEXT REFERENCES tracks(id) ON DELETE SET NULL`、`system_media_identity TEXT`、`feedback_type TEXT NOT NULL CHECK(feedback_type IN ('like','skip','less_talk','more_like_this'))`、`value_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(value_json))`、`created_at_ms INTEGER NOT NULL`、`revoked_at_ms INTEGER`。索引 `ix_feedback_track_active(track_id,feedback_type,revoked_at_ms)` 和 `ix_feedback_time(created_at_ms DESC)`。

### 4.4 对话、摘要与记忆

#### `chat_sessions`

`id TEXT PRIMARY KEY`、`program_run_id TEXT REFERENCES program_runs(id) ON DELETE SET NULL`、`started_at_ms INTEGER NOT NULL`、`ended_at_ms INTEGER`、`covered_from_ms INTEGER`、`covered_to_ms INTEGER`、`user_message_count INTEGER NOT NULL DEFAULT 0 CHECK(user_message_count>=0)`、`assistant_message_count INTEGER NOT NULL DEFAULT 0 CHECK(assistant_message_count>=0)`、`status TEXT NOT NULL CHECK(status IN ('active','completed','interrupted'))`。table CHECK 要求 covered range 同为 NULL 或同为非 NULL 且 from≤to；零消息 session 使用 NULL range。插入 message 时在同一事务中扩大 covered range并增加对应 role count；message 的 30 天到期清理不回退这些非内容 aggregate，因此导出无需保留正文即可提供范围/count。一个 run 至多一个 chat session：partial unique index on `program_run_id WHERE program_run_id IS NOT NULL`。

#### `messages`

`id TEXT PRIMARY KEY`、`chat_session_id TEXT NOT NULL REFERENCES chat_sessions(id) ON DELETE CASCADE`、`role TEXT NOT NULL CHECK(role IN ('user','assistant'))`、`content_text TEXT NOT NULL`、`content_hash TEXT NOT NULL`、`provider TEXT`、`model TEXT`、`prompt_version TEXT`、`created_at_ms INTEGER NOT NULL`、`expires_at_ms INTEGER NOT NULL`、`deletion_reason TEXT`。索引 `ix_messages_session_time(chat_session_id,created_at_ms)`、`ix_messages_expiry(expires_at_ms)`。`expires_at_ms` 创建时固定为 `created_at_ms + 30 days`；不得以访问行为延期。

#### `session_summaries`

`id TEXT PRIMARY KEY`、`chat_session_id TEXT NOT NULL UNIQUE REFERENCES chat_sessions(id) ON DELETE CASCADE`、`status TEXT NOT NULL CHECK(status IN ('active','deleted'))`、`summary_text TEXT`、`source_from_ms INTEGER`、`source_to_ms INTEGER`、`source_hash TEXT`、`generation_kind TEXT CHECK(generation_kind IN ('llm','deterministic'))`、`preference_signals_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(preference_signals_json))`、`provider TEXT`、`model TEXT`、`prompt_version TEXT`、`revision INTEGER NOT NULL DEFAULT 1 CHECK(revision>0)`、`created_at_ms INTEGER NOT NULL`、`updated_at_ms INTEGER NOT NULL`、`deleted_at_ms INTEGER`。table CHECK 要求 active 行具备 summary/source range/hash/generation/prompt 且 `length(summary_text) BETWEEN 1 AND 1000`；deleted 行必须清空 summary、source_hash、preference signals、provider/model/prompt，保留 id/session/status/revision/time 作为 deletion tombstone。API-047 在同一事务中清空正文、置 deleted 并递增 revision；存在 tombstone 时清理任务不得重新生成 summary。

#### `memory_proposals`

`id TEXT PRIMARY KEY`、`category TEXT NOT NULL CHECK(category IN ('preference','routine','boundary','biographical'))`、`statement_text TEXT`、`statement_hash TEXT NOT NULL`、`reason_text TEXT`、`confidence REAL NOT NULL CHECK(confidence BETWEEN 0 AND 1)`、`status TEXT NOT NULL CHECK(status IN ('proposed','approved','rejected','superseded'))`、`created_at_ms INTEGER NOT NULL`、`decided_at_ms INTEGER`、`prompt_version TEXT NOT NULL`、`model TEXT`。proposed/approved 状态必须有 statement；rejected 后 30 天将 statement/reason 置 NULL，只保留 hash/status 防止重复。

#### `memory_proposal_sources`

`proposal_id TEXT NOT NULL REFERENCES memory_proposals(id) ON DELETE CASCADE`、`message_id TEXT REFERENCES messages(id) ON DELETE SET NULL`、`source_content_hash TEXT NOT NULL`、primary key `(proposal_id,source_content_hash)`。消息过期后 hash 保留，正文不可恢复。

#### `memories`

`id TEXT PRIMARY KEY`、`category TEXT NOT NULL CHECK(category IN ('preference','routine','boundary','biographical'))`、`current_revision INTEGER NOT NULL CHECK(current_revision>0)`、`status TEXT NOT NULL CHECK(status IN ('approved','disabled','deleted'))`、`pinned INTEGER NOT NULL DEFAULT 0 CHECK(pinned IN (0,1))`、`created_from_proposal_id TEXT REFERENCES memory_proposals(id) ON DELETE SET NULL`、`created_at_ms INTEGER NOT NULL`、`updated_at_ms INTEGER NOT NULL`、`deleted_at_ms INTEGER`。只有 `status='approved'` 可进入 AI context。

#### `memory_revisions`

`memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE`、`revision INTEGER NOT NULL CHECK(revision>0)`、`statement_text TEXT`、`statement_hash TEXT NOT NULL`、`change_kind TEXT NOT NULL CHECK(change_kind IN ('approved','edited','disabled','reenabled','deleted'))`、`changed_at_ms INTEGER NOT NULL`、primary key `(memory_id,revision)`。删除 memory 时同一事务将所有 revision 的 `statement_text` 置 NULL，保留 hash/时间/change_kind 作为最小审计。

### 4.5 日程与 OS 交互

#### `schedule_rules`

`id TEXT PRIMARY KEY`、`name TEXT NOT NULL`、`enabled INTEGER NOT NULL CHECK(enabled IN (0,1))`、`timezone TEXT NOT NULL`、`days_of_week_json TEXT NOT NULL CHECK(json_valid(days_of_week_json))`、`local_time TEXT NOT NULL`（`HH:mm`）、`notification_only INTEGER NOT NULL DEFAULT 1 CHECK(notification_only=1)`、`next_occurrence_at_ms INTEGER`、`revision INTEGER NOT NULL DEFAULT 1 CHECK(revision>0)`、`created_at_ms INTEGER NOT NULL`、`updated_at_ms INTEGER NOT NULL`。数据映射必须逐字段满足 `schedule-rule.schema.json`；节目 source 由用户点击通知后显式选择的当前 source 决定，不属于 Schedule Rule。

#### `schedule_occurrences`

`id TEXT PRIMARY KEY`、`rule_id TEXT NOT NULL REFERENCES schedule_rules(id) ON DELETE CASCADE`、`occurrence_key TEXT NOT NULL`（rule + intended local datetime + UTC offset）、`due_at_ms INTEGER NOT NULL`、`status TEXT NOT NULL CHECK(status IN ('due','awaiting_user','snoozed','starting','consumed','dismissed','missed'))`、`snooze_minutes INTEGER CHECK(snooze_minutes IN (10,30,60))`、`snoozed_until_ms INTEGER`、`notification_id TEXT`、`program_run_id TEXT REFERENCES program_runs(id) ON DELETE SET NULL`、`updated_at_ms INTEGER NOT NULL`、unique `(rule_id,occurrence_key)`。同一 occurrence 最多一条记录，修改延后时原子替换 snooze 字段；索引 `ix_occurrence_status_due(status,due_at_ms)`。

#### `os_integration_state`

单例 `id='current'`：`notifications_permission TEXT NOT NULL CHECK(notifications_permission IN ('unknown','granted','denied','unavailable'))`、`autostart_enabled INTEGER NOT NULL CHECK(autostart_enabled IN (0,1))`、`tray_enabled INTEGER NOT NULL CHECK(tray_enabled IN (0,1))`、`last_suspend_at_ms INTEGER`、`last_resume_at_ms INTEGER`、`scheduler_checked_at_ms INTEGER`、`updated_at_ms INTEGER NOT NULL`。这里只记录观察状态；真正 OS 配置仍由系统 API 管理。

### 4.6 Provider cache、用量与可靠交付

#### `weather_cache`

`cache_key TEXT PRIMARY KEY`（四位小数坐标 + IANA timezone + variables/units hash）、`city TEXT NOT NULL`、`region TEXT`、`country TEXT NOT NULL`、`country_code TEXT NOT NULL CHECK(length(country_code)=2 AND country_code=upper(country_code))`、`latitude REAL NOT NULL CHECK(latitude BETWEEN -90 AND 90)`、`longitude REAL NOT NULL CHECK(longitude BETWEEN -180 AND 180)`、`timezone TEXT NOT NULL`、`request_shape_hash TEXT NOT NULL`、`weather_json TEXT NOT NULL CHECK(json_valid(weather_json))`、`fetched_at_ms INTEGER NOT NULL`、`expires_at_ms INTEGER NOT NULL`、`delete_after_ms INTEGER NOT NULL`。索引 `ix_weather_expiry(expires_at_ms)`；30 分钟后不得进入 context，存储最多保留 7 天。

#### `provider_usage`

`id TEXT PRIMARY KEY`、`provider TEXT NOT NULL`、`request_kind TEXT NOT NULL`、`model TEXT`、`input_units INTEGER`、`output_units INTEGER`、`audio_seconds REAL`、`latency_ms INTEGER NOT NULL`、`status_class TEXT NOT NULL`、`correlation_id TEXT NOT NULL`、`created_at_ms INTEGER NOT NULL`。不保存 request/response body，不估算价格。API-004/API-008 先分别写 `secret_validation` / `model_validation` candidate outcome；这些行保留调用事实但不参与当前 integration status 聚合。只有验证成功且设置保存成功时，设置事务才把同一行原子提升为 `secret_validation_applied` / `model_validation_applied`；验证失败、设置失败或崩溃留下的 candidate 行不能污染重启后的当前状态。其他已提交配置上的 provider request kind 直接参与状态聚合。索引 `ix_usage_provider_time(provider,created_at_ms)`；按 13 个月保留用于本机会话用量显示与可靠性诊断。

#### `outbox_events`

`id TEXT PRIMARY KEY`、`aggregate_type TEXT NOT NULL`、`aggregate_id TEXT NOT NULL`、`aggregate_revision INTEGER NOT NULL CHECK(aggregate_revision>=0)`、`event_type TEXT NOT NULL CHECK(event_type!='cyberkindred://v1/chat/message')`、`payload_json TEXT NOT NULL CHECK(json_valid(payload_json))`、`created_at_ms INTEGER NOT NULL`、`delivered_at_ms INTEGER`、unique `(aggregate_type,aggregate_id,aggregate_revision,event_type)`。仅用于不含用户/AI正文的状态事件；payload 禁止 `text`、`content`、`summary`、profile/memory statement、secret 或绝对路径。EVT-004 只从当前进程内存直接 emit，绝不写 outbox。任何实体/分类删除先按 aggregate/type 删除关联 outbox 行，再报告成功；已 delivered 非内容行最多保留 24 小时。

## 5. 不存储的数据

以下值禁止作为数据库列或 JSON 值：OpenAI API Key、Authorization header、Credential Manager secret/value/handle、Apple Music 账号 cookie/token、音频文件 bytes、完整 provider request/response body、屏幕/麦克风内容、系统所有媒体会话清单历史、用户未选择目录的路径。

Credential Manager target 固定使用 `CyberKindred/provider/{provider}/{origin_sha256}/api-key`，其中 hash 来自规范化 HTTPS origin，使 custom origin 之间不会共用 Key；数据库只可保存 `credential_present: true/false` 的瞬时 view，不持久化该镜像，避免不同步。MVP 可为同一 provider 的每个已成功验证 canonical origin 保留一个 Key：API-004 验证新 origin 成功并切换设置后不得删除其他 origin Credential；失败则保留旧设置/Key。API-005 只删除请求中指定 canonical origin 的条目（无论它是否为当前 configured origin）；只有 API-005 或全部重置可删除这些条目。全部重置枚举并删除该 provider prefix 下全部条目。

## 6. 数据清单与五个分类删除组

Inventory 必须列出所有本地/内存数据类，而 `DataCategory` 只表示 FR-DAT-004 的五个批量删除组；两者不是同一枚举。每个 inventory row 返回 storage class、item count、retention、external recipients、`delete_route`，但只有映射到下表五组的 row 提供 category-delete action。

| `DataInventoryCategory` | Storage/data class | Delete route / category |
|---|---|---|
| `credentials` | Credential Manager 中各 canonical origin 的 API Key | API-005 删除指定 origin；full reset 枚举并删除 `CyberKindred/provider/*`；不是 category |
| `profile_and_preferences` | `user_profile` 的称呼、作息与节目偏好 | `profile_and_memories` |
| `weather_location_and_cache` | selected `WeatherLocation`、`weather_cache`；未选 query/candidate 只在内存 | `profile_and_memories` 清位置及其 weather cache；`metadata_cache` 也可单独清全部 weather cache；临时搜索在 10 分钟、替换搜索或退出时清除 |
| `library_roots_and_identity` | `library_roots`、授权路径、file identity | `library_index` |
| `embedded_music_tags` | `tracks` 内原始 tag/index | `library_index` |
| `metadata_matches` | `track_external_metadata`、CAA negative match | `metadata_cache` |
| `artwork_and_tts_cache` | cover/TTS cache rows、files、references、leases | `metadata_cache` |
| `system_media_runtime` | 当前 GSMTC session/capability/timeline（内存） | disconnect/process exit；持久 display snapshot 归下一项 |
| `playback_history_and_feedback` | run/segment state、playback events、Apple display snapshot、feedback | `playback_history` |
| `chat_messages` | `chat_sessions` aggregate 与 `messages` | `conversations_and_summaries` |
| `voice_segment_text` | `program_segments.voice_text`/hash | `playback_history` 删除 segment 及其正文/hash；30 天自动清理仅置正文 NULL |
| `session_summaries` | active summary、revision 与 deletion tombstone | `conversations_and_summaries` |
| `memory_proposals` | proposal、source hash、status | `profile_and_memories` |
| `approved_memories_and_revisions` | memory 与 revision/audit | `profile_and_memories` |
| `schedules_and_notifications` | schedule/occurrence、Windows notification/autostart观察状态 | individual Schedule/Settings API 或 full reset；不是 category |
| `provider_usage_facts` | `provider_usage` | retention cleanup 或 full reset；不是 category |
| `operation_outbox` | 不含正文的 `outbox_events` | aggregate/category scrub、retention cleanup 或 full reset；不是独立 category |
| `diagnostic_logs` | 脱敏 rolling log | retention cleanup 或 full reset；不是 category |
| `migration_backups` | 最近的 pre-migration SQLite backup | retention cleanup；任一 category delete 与 full reset 同步删除；不是 category |

五个 category 的事务目标固定为：

- `profile_and_memories`：清空 `user_profile` 个性化/WeatherLocation 字段并恢复非识别默认值，删除与旧 location 关联的 weather cache，并删除 memory proposal/source/memory/revision。不得删除 schedule timezone。
- `conversations_and_summaries`：删除 chat session、message、active summary 和 summary tombstone；原文已同时删除，因此不会触发重新生成。
- `playback_history`：删除 program run/segment/playback event/feedback，清空 track `last_played_at_ms`，并 scrub 相关 outbox；不删除 track/index。
- `metadata_cache`：删除 MusicBrainz/CAA positive-negative/weather/TTS entry/reference/lease 与对应 cache 文件；保留原始 local tag/index。
- `library_index`：删除 roots/scan jobs/tracks，并通过 FK/显式清理删除关联 metadata/cover reference；永不调用源音乐删除 API。

## 7. 保留与清理

| 数据 | 默认保留 | 清理行为 |
|---|---|---|
| `messages.content_text` | 创建后 30 天 | 每次启动及每日一次批量删除 message 行；若 summary 尚未生成，先生成 deterministic summary，再删除；用户可立即删除。 |
| `program_segments.voice_text` | segment 结束后 30 天 | 置 NULL，保留 hash、状态和时间。 |
| active session summaries、播放事件、feedback、program stats | 直到用户删除/分类删除/全部重置 | 导出后不会自动延长；summary 删除立即清空正文并保留 tombstone，其他数据按 category 级联。 |
| proposed memory proposal | 90 天未处理 | 删除 proposal 与 source links；approved memory 不受影响。 |
| rejected proposal 正文 | 决策后 30 天 | 清空 statement/reason，保留 hash/status。 |
| deleted memory 正文 | 立即 | 清空所有 revision statement，保留最小审计。 |
| weather cache | 最多 7 天 | `delete_after_ms` 后删除。 |
| provider usage | 13 个月 | 按月批量删除。 |
| delivered outbox | 24 小时 | 删除；未 delivered 在成功或 7 天后转诊断失败并删除 payload。 |
| TTS/cover cache | 成功 artifact 30 天或 quota/LRU 先到者；CAA 404 24 小时；staging 24 小时 | 删除可再生文件及对应 cache 行；有效 TTS lease 暂缓 entry 删除，lease 结束后立即重评。 |
| logs | 14 天且总量 50 MiB | rolling delete，保持脱敏。 |

清理 job 单次事务最多处理 500 行并循环让出 runtime，避免启动阻塞。时钟回拨不延长已经写入的 `expires_at_ms`。message 到期时：若 summary row 不存在则先生成 deterministic summary；若存在 `status='deleted'` tombstone 则直接删除 message，绝不重新生成。

## 8. 删除、导出与重置

- **删除单条对话**：删除 message 并 scrub 该 message/operation 关联 outbox；相关 source FK 置 NULL，memory 仍保留但 UI 显示来源已删除。若用户同时选择删除衍生记忆，则走 memory 删除事务。
- **删除 summary**：校验 expected revision 后，将 row 置 `status='deleted'`、清空正文/source/preference/provider 字段、记录 `deleted_at_ms` 并递增 revision；tombstone 阻止 message 到期或下次启动重新生成。
- **删除 session**：级联 messages/summary/tombstone 并 scrub 关联 outbox；program/playback facts 默认保留并解除 chat link，除非用户选择 `playback_history` category。
- **删除记忆**：状态改 deleted、清空全部 revision 正文、从 context cache 立即失效；该事务完成后才向 UI 回成功。
- **清除收听历史**：删除 feedback/playback events/program segments/runs，重新计算 track `last_played_at_ms=NULL`；不删除曲库索引和原文件。
- **移除曲库**：删除 root 与 track 索引/cache references；不得调用文件删除 API。
- **分类删除的 outbox/备份传播**：任一 category 事务先删相关 outbox 行，成功后立即删除全部 pre-migration backup 并 checkpoint WAL，防止从旧副本恢复已删数据；该动作不创建新的包含删除前数据的 backup。
- **全部重置**：仅在用户完成二次确认后执行。先停止并取消节目、扫描、provider、scheduler 与通知任务，关闭 SQLite，再删除 `CyberKindred/provider/` 下全部 Credential Manager 条目、数据库及 WAL/SHM、全部 migration backup、cache、生成音频、日志、应用设置与未交付的临时 export；同时关闭自启动并撤销待处理通知。不得创建 quarantine、回收站副本或应用可恢复备份。重启后从空目录创建新库并进入首次引导；用户原始音乐与用户此前主动保存到应用目录之外的导出不受影响。
- **导出**：生成版本化 JSON manifest + UTF-8 JSONL，只包含 FR-DAT-003 要求的非敏感 profile/settings（含 schedule rule）、approved memory、proposal status、active session summary 及 play/feedback records。每个 chat session 只导出 covered-from/to 和 user/assistant message count，不导出原始正文或 content hash。MVP 固定排除 proposal 正文、deleted summary/tombstone、secret、voice text、absolute/relative 音乐路径、曲库 index/tag/statistics、音频、封面/TTS/cache、OS notification identifier、日志、backup、outbox、provider usage 明细与 correlation ID，不提供扩大范围的勾选项。临时 export 成功复制或取消后立即删除。

## 9. 迁移策略

1. 迁移文件命名 `VNNNN__description.sql`，编译进 Rust binary，不在运行时下载。
2. 启动先验证 `application_id`、`user_version`、migration checksum 与 `PRAGMA integrity_check(1)`；失败进入 recovery mode，不启动 scheduler/program。
3. 对每个未应用版本，在 migration 前使用 SQLite backup API 创建 `backups/pre-migration-v{current}-{utc}.sqlite3`，确认 backup integrity 后执行。
4. 每个版本在 `BEGIN IMMEDIATE` 中完成 schema/data change、插入 `schema_migrations`、更新 `user_version`；任何失败完整回滚。
5. 只支持前向迁移；旧应用遇到更高 `user_version` 必须拒绝打开。发行回滚依赖 pre-migration backup，不运行 down migration。
6. 大表变更使用 create-copy-verify-swap：创建新表、分批复制、核对 row count/关键 checksum、同事务 rename；禁止直接丢列造成静默数据丢失。
7. 最近 3 个成功 pre-migration backup 保留 30 天；清理前确认当前数据库至少成功启动一次且 integrity check 通过。

## 10. 数据模型测试门槛

1. 空库可迁移到 latest；每个历史 schema fixture 可逐级迁移；重复启动幂等。
2. checksum 改变、外部数据库、未来版本、corrupt DB 均拒绝写入并进入 recovery mode。
3. foreign key、CHECK、partial unique index 和 root containment 有正反例测试。
4. 30 天消息/voice/cache、24 小时 CAA negative、90 天 proposal、7 天 weather、13 个月 usage 和 outbox 清理使用冻结时钟验证边界毫秒；有效/过期 TTS lease 与 preview reference 均有测试。
5. Summary 删除测试验证正文立即清空、revision 增加、原文到期和重启均不重新生成；分类删除测试验证 outbox 与 migration backup 无旧值。
6. 导出 snapshot 只含 FR-DAT-003 集合，chat 只有范围/count，并证明不存在 Credential/key、Authorization、原始 chat/voice、absolute/relative library path、cache、provider usage 明细或 deleted tombstone。
7. 全部重置验证所有 configured-origin Credential、SQLite/WAL/SHM、migration backup、cache、生成音频、日志、设置、日程/通知与自启动均移除且应用无法恢复，音乐原文件及用户主动保存的外部导出未改变。
