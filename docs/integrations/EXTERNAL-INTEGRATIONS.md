# External Integrations

| Field | Value |
|---|---|
| Status | Approved |
| Owner | Integration Owner |
| Last Verified | 2026-09-02 |
| Source of Truth For | 外部服务的认证、数据披露、能力边界、限流、缓存、降级与测试方法 |
| Related Documents | `docs/contracts/PROVIDER-CONTRACTS.md`, `docs/security/PRIVACY-DATA-LIFECYCLE.md`, `docs/security/LEGAL-AND-LICENSING.md`, `docs/planning/RISK-REGISTER.md` |

## 1. Integration policy

- 所有网络请求由 Rust Core 发起，使用 HTTPS、固定允许的 scheme/host、有限重定向、响应大小上限、deadline 和脱敏日志。WebView 不直接请求这些服务。
- 用户内容与本地标签均视为不可信数据；它们不能改变 system/developer policy、启用工具、选择请求目标或取得文件/secret。
- 默认测试使用本地 fake server，并阻断公共 DNS。真实服务测试必须单独标记、默认跳过、由操作者显式提供凭据或同意流量。
- 外部条款、模型与限额在每次公开或商业发布前复核；本文件记录的是 2026-09-01 基线。

## 2. OpenAI Responses and Audio Speech

| Item | Decision |
|---|---|
| Authentication | 用户自带 OpenAI API Key（BYOK），只保存在 Windows Credential Manager；按 canonical API origin 分隔凭据。API-005 删除指定 origin；API-037 枚举并删除所有 `CyberKindred/provider/*` origin credential。WebView、SQLite、导出与日志只看 origin 与“已配置”布尔值。 |
| Endpoints | 文本与结构化输出使用 `POST /v1/responses`；语音使用 `POST /v1/audio/speech`。官方默认 origin 为 `https://api.openai.com`。 |
| Data sent | Responses：人格/安全规则、时间、可选天气摘要、用户画像摘要、已批准记忆、近期对话片段、local-source 聚合反馈/实际播放事实、本地候选曲目的文本标签；Speech：合规的最终串场文字、模型/声音/格式参数。Apple Music/GSMTC metadata、session/media identity、capability、timeline 与 playback event 永不发送至 Responses 或 Speech。 |
| Data never sent | API Key 以外的 credential、本地绝对路径、音频文件/字节、未批准记忆、完整 SQLite、诊断日志、屏幕、麦克风、Apple Music cookie/token。 |
| Retention control | 每个 Responses 请求显式 `store: false`，不使用 provider conversation ID，也不启用 background mode。`store: false` 不是对所有服务端日志/法定义务的零保留承诺；用户仍须参考其 OpenAI 账户的数据控制。 |
| Limits | 每个 Responses 请求 input ≤24,000 tokens、requested output ≤4,000 tokens、候选曲目≤200，总 deadline 60 秒；Speech 45 秒、输入最多 500 Unicode 字符（低于 API 的 4096 字符上限）、输出最多 20 MiB。请求不启用 hosted web/file/computer tools。 |
| Cache | LLM 输出不跨程序缓存；语音按规范化文本、voice/model/format 哈希缓存 30 天，或在用户清除数据时立即删除。哈希不含 key。 |
| Failures | 401/403 → ERR-1301；429 → ERR-1302；deadline → ERR-1303；网络/5xx → ERR-1304；无效 structured output/audio → ERR-1305。LLM 可进行一次无工具 schema repair；TTS 失败回退为屏幕文字。 |
| Custom origin | 高级设置只接受 HTTPS origin。保存前显示完整 hostname 与“此主机将收到 API Key 和所列上下文”的确认；禁止 userinfo、fragment、IP literal、明文 HTTP、跨 origin redirect。每个 origin 单独存 key；切换 origin 不复制 credential，删除/测试均绑定明确 origin。全部重置删除所有 CyberKindred origin 的 key，而非仅当前 origin。 |
| Test | Fake 断言 Authorization 不进入日志、Responses 必有 `store:false`、无 hosted tools、取消后不落库；真实 smoke test 只验证最短中文输出和短语音，显式标签 `real_openai`。 |

OpenAI 官方 Responses 接口说明 `store` 控制是否保存生成响应以供后续 API 获取；CyberKindred 将其固定为 false。[Create a model response](https://developers.openai.com/api/reference/cli/resources/responses/methods/create) Speech 接口接受文本并返回或流式返回音频，当前官方参数列出了 `gpt-4o-mini-tts` 等模型与 mp3/opus/aac/flac/wav/pcm 格式。[Create speech](https://developers.openai.com/api/reference/resources/audio/subresources/speech/methods/create) 服务端实际保留还受账户数据控制与 OpenAI 政策约束，因此 UI 不宣称“OpenAI 零保留”。[OpenAI data controls](https://platform.openai.com/docs/models/default-usage-policies-by-endpoint)

## 3. Apple Music on Windows via GSMTC

| Item | Decision |
|---|---|
| Authentication | CyberKindred 不接收 Apple ID、Apple Music cookie、developer token 或 user token。用户在 Apple Music Windows App 内自行登录。 |
| Interface | Rust 通过 Windows `GlobalSystemMediaTransportControlsSessionManager` 枚举系统媒体会话，用户明确选择后保存非秘密的 `SourceAppUserModelId` 绑定。 |
| Data read | 当前媒体标题、艺术家、专辑、封面（若会话提供）、播放状态、时间轴和每项 playback capability。只在本机进程内使用，不进入 Responses、Speech、metadata 或 weather 请求；实际曲目变化仅触发本地 deterministic 可见文字模板。 |
| Actions | 只在运行时 capability 为 true 时尝试 play、pause、seek、next、previous；调用后重新读取状态。不能搜索 Apple Music catalog、选择任意歌曲、设置精确队列、修改资料库或创建 playlist。 |
| Session safety | 不按窗口标题或“Apple Music”文本盲选；绑定会话身份。会话替换、用户媒体键操作或状态 revision 不符时停止恢复，绝不把命令重放到新会话。 |
| TTS behavior | Track-aware Apple 反应永远只显示本机 deterministic 文字，不合成语音。只有文字及其生成输入完全不含 GSMTC/Apple metadata、event、timeline、capability、session/media identity 的通用段才可调用 Speech；此时只在同一会话可 pause 且正在播放时暂时暂停，TTS 结束后仅在无用户 override 时恢复。无法确认就保持现状。 |
| Failures | 未安装/未播放/未暴露会话 → ERR-1202；控制未启用 → ERR-1201；会话漂移 → ERR-1203。UI 显示连接指引并允许改用本地曲库。 |
| Test | Fake GSMTC 覆盖 capability、竞态与 data-boundary capture（Responses/Speech 中 GSMTC canary 匹配数为 0）；真实 Apple Music Windows App + 有效订阅的 `real_apple_music` 手工测试验证元数据、控制、休眠恢复与用户 override。 |

Microsoft 将 GSMTC session 定义为“来自另一个应用、提供播放信息并可能允许控制”的会话；具体控制必须读取当前 `IsPlayEnabled`、`IsPauseEnabled`、`IsNextEnabled` 等属性，而不能假设存在。[GSMTC session](https://learn.microsoft.com/en-us/uwp/api/windows.media.control.globalsystemmediatransportcontrolssession?view=winrt-28000) [Playback controls](https://learn.microsoft.com/en-us/uwp/api/windows.media.control.globalsystemmediatransportcontrolssessionplaybackcontrols?view=winrt-28000)

本能力不是 MusicKit。MusicKit 能访问 catalog、用户资料库并支持 app/web 播放，但需要相应标识、developer token 和用户授权；这些能力不进入 MVP。[Apple MusicKit](https://developer.apple.com/musickit/) CyberKindred 也不注入或自动化 `music.apple.com` DOM，不读取浏览器会话。

### 3.1 M1 probe evidence

`TASK-001` 在 2026-09-02 的 Windows 11 Home build 26200 上，以 `windows 0.62.2` 只读探针成功枚举 Apple Music Windows App package `1.1540.23042.0` 的会话。观察到的精确 `SourceAppUserModelId` 为 `AppleInc.AppleMusicWin_nzyj5cx40ttqa!App`；App 运行但未播放时仍可能暴露 `opened` 会话，媒体正文为空、timeline 可读、capability 仅按实时值出现且字段读取错误为零。该值是本机实测身份，不升级为跨版本硬编码常量；产品连接仍须枚举、让用户明确选择并保存所选身份。完整矩阵与红线见 [`TASK-001-MATRIX.md`](../../spikes/gsmtc/evidence/TASK-001-MATRIX.md)。

## 4. MusicBrainz Web Service

| Item | Decision |
|---|---|
| Authentication | 公开只读 Web Service，无用户登录。 |
| Endpoint | 只允许 `https://musicbrainz.org/ws/2/` 的 GET 查询；不使用编辑、OAuth、submission 或 fingerprint 服务。 |
| Data sent | 本地标签中的 title、artist、album、duration 文本。字段在请求前裁剪、规范化与长度限制。 |
| Data never sent | 音频、AcoustID fingerprint、文件名/路径、用户身份、对话、播放历史、城市或 API Key。 |
| User-Agent | `CyberKindred/<app-version> (<maintainer-contact-url>)`；Release 构建缺少可联系的 HTTPS URL 或 mailto 地址时集成质量门失败。 |
| Rate limit | 全进程 token bucket：最多平均 1 request/second，不并发突发；503/429 使用带 jitter 的退避并尊重 Retry-After。 |
| Cache | 成功的 MBID/标签/置信度持久缓存至用户手动刷新；无匹配结果缓存 24 小时；原文件标签不被修改。 |
| Match policy | 仅返回候选与确定性置信度。低置信度结果可显示但不得取代原标签或驱动未经校验的曲目身份。 |
| Test | Fake 验证文本白名单、User-Agent、1 req/s 和缓存；真实测试只用合成标签并标记 `real_musicbrainz`。 |

MusicBrainz 当前公开规则要求有可联系维护者信息的 User-Agent；未另行约定时，来源 IP 的平均请求速率上限为每秒一次。[MusicBrainz rate limiting](https://musicbrainz.org/doc/MusicBrainz_API/Rate_Limiting)

## 5. Cover Art Archive

| Item | Decision |
|---|---|
| Authentication | 无。仅在已有经验证 release MBID 时 GET `https://coverartarchive.org/release/{mbid}/front-{size}`，MVP size 为 500。 |
| Data sent | release MBID 与固定尺寸；不发送本地标签、路径或用户数据。 |
| Redirect policy | 最多 3 次；只接受 HTTPS，目标 host 为 `coverartarchive.org`、`archive.org` 或其子域；禁止私网/link-local/loopback 地址。 |
| Response policy | 只接受声明与 sniff 均为受支持图片的响应；压缩数据最多 5 MiB、解码最多 4096×4096；失败不解析为 HTML。 |
| Cache | 成功图片缓存 30 天；404 缓存 24 小时；删除全部用户数据时清空。UI 可退回本地封面或占位图。 |
| Test | Fake 覆盖 307、重定向逃逸、MIME 欺骗、图片炸弹、404 与 503；真实测试使用公开 MBID 并标记 `real_cover_art`。 |

官方 API 使用 release MBID 获取 JSON 或 front/back 图片，并可能通过 307 重定向到实际文件；404 与 503 必须作为正常失败处理。[Cover Art Archive API](https://musicbrainz.org/doc/Cover_Art_Archive/API) 封面权利并不因为可公开访问而自动清除，项目按“use at your own risk”处理。[Cover Art Archive policy](https://musicbrainz.org/doc/Cover_Art_Archive)

## 6. Open-Meteo

| Item | Decision |
|---|---|
| Authentication | 非商业内测使用 free/open-access endpoint，不使用 API Key；商业化前必须切换为有商业许可的方案或移除集成。 |
| Endpoints | 用户显式搜索时只允许 `https://geocoding-api.open-meteo.com/v1/search` GET；当前天气只允许 `https://api.open-meteo.com/v1/forecast` GET。 |
| Data sent | Search：用户输入的城市/邮编查询文本、`language=zh`、`count<=10`。Forecast：四位小数纬度/经度、IANA timezone、所需变量。两类请求在网络层不可避免披露 IP。 |
| Data never sent | 姓名、称呼、设备位置权限、GPS、IP geolocation、对话、音乐、API Key；Forecast 不发送 search query、城市自定义 label、country/region。 |
| Selection/cache | Search 结果只在内存保留 10 分钟，用户明确选择后才持久化 city/region/country/coordinates/timezone；Forecast 按选定坐标缓存。不会后台搜索、按 IP 猜城市或自动改选。 |
| Forecast cache | 按四位小数坐标、timezone 和变量集缓存成功结果 30 分钟；离线可显示“上次更新”但不把过期天气放进新 LLM context。 |
| Limits | App 自限每个位置每 30 分钟一次，远低于当前 free tier 600/min、5,000/hour、10,000/day；不在整点批量唤醒。 |
| Attribution | 城市搜索结果显示 “Location data by GeoNames via Open-Meteo”；天气处提供可点击的 “Weather data by Open-Meteo.com” 和 CC BY 4.0 说明。 |
| Failure | 超时/5xx/网络 → ERR-1303/1304；400/无效 JSON/非有限数值 → ERR-1305；节目省略天气而不中止。 |
| Test | Fake 验证无用户动作不搜索、query 长度、candidate expiry/选择、坐标取整、字段白名单、forecast 缓存/过期和归因；真实测试只用通用城市名/非敏感坐标并标记 `real_open_meteo`。 |

Open-Meteo Geocoding API accepts an explicit `name` search and returns coordinates, timezone, country and administrative area；其 location data 基于 GeoNames。[Geocoding API](https://open-meteo.com/en/docs/geocoding-api) Forecast API 支持 current conditions 与坐标参数。[Forecast API](https://open-meteo.com/en/docs) 当前免费服务限定非商业使用并公布调用限额，服务日志可能包含 IP、search query 与坐标；商业使用需要相应订阅。[Terms & Privacy](https://open-meteo.com/en/terms) API 数据按 CC BY 4.0 提供并要求相邻归因链接。[Licence](https://open-meteo.com/en/license)

## 7. Change and disable switches

每个网络 provider 都有本地 enabled flag；关闭后不再发起请求且保留的数据按隐私生命周期处理。若条款、价格、端点或能力发生不兼容变化，发布负责人关闭对应 provider、在风险登记中记录触发条件，并在经过契约与隐私审查前不以非官方 scraping 代替。
