# CyberKindred AI 编排

| Metadata | Value |
|---|---|
| Status | Approved |
| Owner | AI & Program Architecture |
| Last Verified | 2026-09-03 |
| Source of Truth For | Context 组装、候选曲目约束、节目规划、LLM/TTS 调用、校验、重试、缓存、记忆提案与 AI 降级流程 |
| Related Documents | [AI Behavior](../product/AI-BEHAVIOR.md), [FRS](../product/FRS.md), [NFRS](../product/NFRS.md), [Architecture](ARCHITECTURE.md), [Runtime State Machines](RUNTIME-STATE-MACHINES.md), [Data Model](DATA-MODEL.md), [Provider Contracts](../contracts/PROVIDER-CONTRACTS.md), [Program Plan Schema](../contracts/schemas/program-plan.schema.json) |

## 1. 原则

AI 是受约束的节目策划与表达组件，不是播放、存储或权限主体。

1. Rust 对数据选择、权限、候选集、状态转换和最终动作拥有决定权。
2. 模型只看到当前用例所需的最小上下文；不提供文件内容、绝对路径、API Key、Credential handle、屏幕、麦克风或完整数据库。
3. 所有会改变节目、偏好或记忆的模型输出必须为 Structured Output，并依次通过 JSON Schema、引用完整性与领域规则校验。
4. `store: false` 是每次 OpenAI Responses 请求的强制参数；v1 不使用 OpenAI conversation 对象或后台 response 保存。多轮上下文由本地选择后显式重发。
5. Provider 不直接写数据库或控制播放器。它返回 typed result；application service 在事务和状态机内应用结果。
6. 外部 AI 失败不得使已经可用的音乐突然停止；使用缓存、确定性策略或纯文字降级。

## 2. 用例与输出权限

| 用例 | 输入 | 允许输出 | 禁止输出 |
|---|---|---|---|
| `PlanLocalProgram` | 画像、已批准记忆、环境、反馈、有限候选曲目 | `ProgramPlan` 中的 voice/track segment 与已有 `track_id` | 新曲目 ID、文件路径、播放器命令、记忆直接写入 |
| `ComposeChatReply` | 当前用户消息、有限近期对话、相关已批准记忆 | 面向用户的文字回复、可选 `MemoryProposal` 候选 | secret、隐藏推理、未批准记忆写入、系统动作 |
| `SummarizeSession` | 最小必要会话消息、该 run 的聚合反馈与实际播放事实 | session summary、preference signals、可选 memory proposals | 改写原始记录、删除记录、批准 proposal |
| `SynthesizeSpeech` | 已校验 voice text、voice/model/speed/format | 音频 bytes/stream 与格式元数据 | 播放、暂停、resume 决策 |

## 3. Context Window 组装

### 3.1 固定顺序

每次请求由 `ContextBuilder` 按以下顺序生成不可变 `ProgramContext`；后层不能覆盖前层规则：

1. **AI policy**：版本化 prompt ID、AI 身份披露、禁止角色、工具/输出约束。
2. **Program rules**：节目类型、目标时长、串场频率、语言、内容边界；只有 local source capability/state 可进入，system-session metadata/state 不进入。
3. **Environment**：本地时间、星期、时区和用户手动城市的新鲜天气摘要；天气不可用或过期时整个字段省略。
4. **Minimized profile**：语言、作息、音乐偏好和禁忌；display name 只有在用户明确选择将其作为主播称呼时才加入。
5. **Approved memories**：按相关性、用户 pinned、最近确认排序的 `status=approved` memory；最多 20 条、每条最多 240 字。
6. **Aggregate feedback**：最近 30 个 local playback/feedback facts 的有界聚合，不发送完整时间线。
7. **Local candidates**：仅 local 模式本次可选择的候选文本元数据与 opaque `track_id`；system-session 模式不创建该 fragment。
8. **Current-session turns**：当前会话最近 12 条必要消息，每条最多 500 字；不发送更早原文。

### 3.2 预算与裁剪

- `ContextBuilder` 先按字符上限 40,000 生成，再由 provider 按目标模型 tokenizer/限制校验；不能确定 token 数时按 4 字符/token 的保守估算并预留至少 25% 给输出。
- 超限裁剪固定为：未 pinned 的最旧 memory → 最旧 current-session turn → 最旧 aggregate feedback → 低分 local candidate。AI policy、Program rules、当前用户消息和 local 模式至少 10 个候选永不被裁剪。
- 若候选不足 10 个则保留全部，并由确定性策略决定是否能形成节目；不得用模型补造候选。
- 相同 context fragment 使用内容 hash 去重。日志只记录 fragment 类型、数量、字符数和 hash 前 8 位，不记录正文。

## 4. 本地候选曲目流程

### 4.1 Eligibility

SQL 先选择 `availability=available`、路径位于启用 library root、duration 在 30 秒至 30 分钟的曲目。以下是硬约束：

- 同一 `track_id` 在一个 program run 中最多出现一次。
- 当前扫描标记 missing/corrupt/unsupported 的曲目不进入候选。
- 节目剩余时间小于曲长且超出目标结束时间 5 分钟以上时不选择该曲；最后一首允许在 5 分钟容差内结束。

### 4.2 Deterministic ranking

为 eligible 集合计算可复现分数，所有子分归一化到 `[0,1]`：

```text
score = 0.35 * preference
      + 0.20 * context_fit
      + 0.15 * freshness
      + 0.15 * metadata_confidence
      + 0.10 * library_affinity
      + 0.05 * deterministic_jitter
```

- `preference`：like=1，未反馈=0.5，skip 在 30 天内从 0 线性恢复到 0.5。
- `context_fit`：用户显式 mood/routine/tag 与曲目标签的匹配比例；没有上下文时为 0.5。
- `freshness`：从最近播放的 0 到 30 天未播放的 1；从未播放为 1。
- `metadata_confidence`：本地 tag 可靠度与 MusicBrainz 匹配置信度的较高者；外部元数据缺失不低于 0.3。
- `library_affinity`：完整标签、封面和可稳定解码各贡献三分之一。
- `deterministic_jitter`：`hash(program_run_id, track_id)` 映射到 `[0,1]`，使同一 run 重试保持一致。

按分数取最多 80 首进入模型候选。先应用多样性过滤：最近 20 次播放过的 track 暂时排除、同一 artist 至少间隔 3 首、同一 album 至少间隔 2 首。若不足以覆盖目标时长，按“最旧播放优先”逐步放宽最近 20 次限制，再放宽 album、最后放宽 artist；run 内不重复永不放宽。

### 4.3 Model planning 与 validation

`PlanLocalProgram` 调用 OpenAI Responses：

- 模型 ID 来自用户可编辑的 provider setting；初始默认 `gpt-5.6-luna`。API-008 只有在 `llmModelId` 实际变化时，才以 patch 合并后的候选 origin/model 和该 origin 已验证 credential 执行固定 capability probe，确认账号可用且支持 Structured Outputs；该路径总 deadline 为 60 秒，probe 成功后才原子保存整个 patch，失败时任何设置字段都不得改变。API-006 仍只测试当前已保存配置。
- `store: false`、`text.format.type=json_schema`、schema=`program-plan.schema.json`、禁用 built-in tools、限制最大输出。
- 包含可选 schema repair 的总 deadline 为 60 秒；网络/5xx 在原 deadline 内最多重试 2 次，使用指数退避和 full jitter；429 遵循 `Retry-After`，若剩余 deadline 不足则直接降级。auth/invalid request 不重试。

返回后按固定顺序校验：

1. JSON 能解析并通过指定 schema。
2. `program_run_id`、source kind 与请求一致。
3. 每个 `TrackSegment.track_id` 属于候选且未重复。
4. segment 数量、总预计时长和 voice text 长度符合设置；voice text 单段不超过 500 个 Unicode 字符。
5. artist/album 间隔符合本轮实际放宽后的约束。
6. 不含模型生成的 URL、路径、命令或未知字段。

任何失败均不把部分结果入库。对“合法 JSON 但领域约束失败”允许一次 repair request，传入仅错误代码和原候选 ID，不传底层异常；repair 仍失败则走确定性 fallback。

### 4.4 Deterministic fallback

fallback 直接按 4.2 的过滤后分数从高到低装入曲目，直到达到目标时长或候选耗尽；开场/串场使用本地模板，不调用 LLM：

- 开场：`现在是 {local_time}，为你准备了一段音乐。`
- 中间串场仅在设置允许且有安全事实时使用：`接下来继续听 {artist} 的 {title}。`
- 结束：`今天的节目先到这里。`

模板缺少 artist/title 时使用“下一首”，绝不输出 `unknown` 占位符。TTS 不可用时只显示文字。

## 5. Apple Music 陪伴编排

Apple Music 模式没有曲目候选、`setQueue`、云端搜索或 Responses 调用。GSMTC metadata、playback state 和 event 始终留在本机，不进入任何 LLM context。

- track-aware reaction 由 Rust 本地确定性模板在 metadata 连续稳定 1 秒、duration >30 秒后生成；模板只引用当前 snapshot 已有的 title/artist/album，不补充歌词、发行背景或推断事实。
- track-aware reaction **只显示文字**，不得送往 Responses 或 Audio Speech，也不得为此暂停 Apple Music。
- 开播最多显示一次 track-aware reaction；正常密度每 2–3 首且至少间隔 8 分钟，“少说一点”后每 4–5 首且至少 15 分钟。identity 不稳定、短片段或疑似广告不生成。
- 不包含任何 GSMTC/Apple metadata、identity、state 或 event 的通用 voice segment 可以使用 TTS；该 segment 的暂停、播放与恢复仍由 `Interruption` 状态机裁决。
- 通用 TTS 准备完成时若 session/revision 已变化，interruption token 按状态机失效；不得把旧命令重放到新 session。

## 6. 对话、摘要与记忆

### 6.1 对话

用户消息先在 Rust 边界做 UTF-8、空白、最大 4,000 字符与危险控制字符校验，然后持久化。LLM 请求失败时保留用户消息并返回可重试状态，不构造模型回复。输出只作为文字；任何播放意图必须来自显式 UI command，而不是从自由文本模型输出直接执行。

### 6.2 Session summary

节目结束后异步生成不超过 1,000 字符的 summary。输入固定为保留策略选出的最小必要会话消息、该 run 的聚合反馈和实际播放事实；Apple/system-session display snapshot 在进入 summary provider 前移除，只保留不含媒体 metadata 的计数。输出固定为 summary、preference signals 和可选 memory proposals；proposal 仍须用户批准。summary 保存 source message ID/time range 和 prompt/model version。若 AI 不可用则用确定性统计摘要；只有不存在用户删除 tombstone 时才允许重新生成，并保留 generation/version provenance。

### 6.3 Memory proposal

Memory 只可来自用户明确陈述或重复、可验证的反馈模式。模型返回 proposal 后，Rust 执行：

1. 分类为 `preference`、`routine`、`boundary` 或 `biographical`，与 `memory-record.schema.json` 保持一致。
2. 保存 source message IDs、简短 statement、置信度和建议原因；不复制整段对话。
3. 对 statement 正规化并计算 hash；与 proposed/approved 近似重复时合并 sources，不创建第二条。
4. 以 `status=proposed` 展示。只有用户 `Approve` 后建立 `status=approved` memory revision；`Reject` 后保存最小 tombstone，防止同一来源或旧摘要重复提案。
5. 用户编辑生成新的明确 statement revision；删除后立即从 context 排除并建立 deletion audit，不保留被删除正文。

不得自动提案精确地址、凭据、财务账号、医疗诊断、性取向、宗教、政治立场或其他敏感画像；即使用户提到，也仅保留在受 30 天清理的对话原文中，除非后续产品需求与隐私文档经明确批准改变此规则。

## 7. TTS 管线

1. `VoiceTextValidator` 检查来源、长度（单段 ≤500 字符）、语言和禁止 SSML/控制字符。
2. `TTSProvider` 使用 OpenAI Audio Speech；model/voice/speed 来自已验证设置，format 固定为 MP3，初始 model 为 `gpt-4o-mini-tts`。UI 必须披露声音由 AI 生成。
3. 单次 use-case deadline 为 45 秒；5xx/network 在原 deadline 内最多重试 2 次（1 秒、3 秒 + full jitter），429 按 `Retry-After`；auth/invalid input 不重试，失败后不得自动重复播音。
4. 输出固定请求 MP3；写入临时文件、完成格式/时长/大小检查后原子移动到 cache。单段上限 20 MiB、预期时长上限 120 秒，超限视为 invalid provider output。
5. cache key=`SHA-256(model|voice|speed|format|locale|normalized_text)`；只缓存最终音频，不缓存 Authorization/header。artifact 创建后 30 天过期，且受默认 512 MiB quota/LRU 限制；active artifact 有 lease，不得清理。
6. TTS 完成后只返回 artifact handle；播放与 resume 由 [Runtime State Machines](RUNTIME-STATE-MACHINES.md) 决定。

## 8. Metadata 与天气增强

- MusicBrainz query 只发送本地 tag 中的 title/artist/album/duration，不发送音频或本地路径。全应用 rate limiter 为 1 request/second，使用明确 User-Agent；成功匹配缓存到用户手动刷新，保存 MBID、normalized tags、confidence 与 fetched time。
- 自动采用要求 confidence ≥0.90 且 artist/title 精确正规化匹配；0.70–0.89 只保存 suggestion；<0.70 作为 no-match cache 24 小时。任何外部结果不覆盖原始 local tags。
- Cover Art Archive 仅在已有 release MBID 时请求固定 500px front image；保存来源 URL、ETag、license/attribution metadata 与本地 cache handle，成功图片缓存 30 天、404 缓存 24 小时。失败不影响 track eligibility。
- 城市选择只在用户显式搜索时调用 Open-Meteo Geocoding，发送 query/language/result count；query 与候选只在内存保留 10 分钟，选择后仅持久化 `WeatherLocation`。Forecast 只使用该城市的 latitude/longitude（出站前四舍五入到四位小数）、IANA timezone、所需 current variables 与单位；成功结果缓存 30 分钟，每个位置 30 分钟内最多请求一次。两条路径都不使用 GPS、IP geolocation 或后台定位；失败使用未过期缓存，无缓存或缓存已过期则省略整个 weather fragment。

## 9. Provider 可靠性、成本与可观测性

| 控制 | 决策 |
|---|---|
| 并发 | LLM 同时最多 1 个 program planning + 1 个 chat/summary；TTS 同时 1 个；MusicBrainz 1 req/s；天气同时 1 个。 |
| 取消 | 新 program、stop、source switch、power suspend 取消不再需要的 request；取消后的结果不得持久化或播放。 |
| circuit breaker | 同一 provider 连续 5 次 transient failure 后开启 60 秒；auth failure 立即开启，直到设置被修正并 probe 成功。 |
| usage | 保存 request kind、model、token/audio usage、latency 与 classified status，用于本机会话用量显示和可靠性诊断；不估算价格，不保存 request/response body。 |
| prompt version | 每个输出保存 `prompt_version`、`provider`、`model`、schema version 与 input fragment hashes，以便重现规则而不保留发送正文副本。 |

## 10. 验证要求

- golden tests 固定 context 与 candidate，验证裁剪顺序、分数、放宽顺序和 fallback 可复现。
- schema tests 必须拒绝未知/重复 track ID、越长 voice、未知字段、错误 source 和超时陈旧结果。
- property tests 验证任何模型输出都不能产生候选集外 track、未批准 memory 或直接 source command。
- race tests 覆盖新曲到达、stop、power suspend、用户手动控制与 LLM/TTS 返回的所有排列。
- privacy tests 对序列化 request 做 snapshot，证明不含 key、绝对路径、未选记忆或超出窗口的原始对话。
- provider tests 默认使用 hermetic fake；真实 OpenAI 测试需要显式环境标记、预算保护并可跳过。

## 11. 官方接口依据

- [OpenAI Responses API：Structured text output 与 `store`](https://developers.openai.com/api/reference/cli/resources/responses/methods/create)
- [OpenAI Text to speech](https://developers.openai.com/api/docs/guides/text-to-speech)
