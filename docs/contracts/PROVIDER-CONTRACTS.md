# Provider Contracts v1

| Field | Value |
|---|---|
| Status | Approved |
| Owner | Architecture Owner |
| Last Verified | 2026-09-02 |
| Source of Truth For | Music source 与 LLM、TTS、metadata、weather provider 的统一接口和失败语义 |
| Related Documents | `docs/contracts/API-CONTRACT.md`, `docs/contracts/schemas/`, `docs/architecture/AI-ORCHESTRATION.md`, `docs/integrations/EXTERNAL-INTEGRATIONS.md` |

## 1. Common rules

Provider interface lives in Rust Core. React cannot instantiate providers, supply arbitrary endpoints, read credentials or receive upstream response bodies. Every method accepts a cancellation token and a `correlationId` UUID, uses bounded timeouts, and returns `Result<T, ProviderError>` or the source-specific error union. No provider may write SQLite directly; orchestration validates and persists returned values.

```rust
struct ProviderContext {
    correlation_id: Uuid,
    locale: String,       // MVP: zh-CN
    deadline: Instant,
    cancellation: CancellationToken,
}
```

`ProviderError` is the machine type in `provider-error.schema.json`. Mapping is fixed:

| Category | Public ID | Retryable | Meaning |
|---|---|---:|---|
| `authentication` | ERR-1301 | false | Credential or provider configuration rejected |
| `rate_limit` | ERR-1302 | true | Provider throttled; `retryAfterMs` when known |
| `timeout` | ERR-1303 | true | Client deadline elapsed |
| `unavailable` | ERR-1304 | true | DNS, TLS, network, 5xx or service unavailable |
| `invalid_response` | ERR-1305 | false | Successful transport but response/schema/content invalid |

Provider retries occur in orchestration, not adapters. Retryable reads/generation receive at most two retries with exponential backoff, full jitter and the original deadline. Authentication and invalid response are not blindly retried; LLM structured output permits exactly one schema-repair request, which counts separately and receives no tools.

## 2. `MusicSourceAdapter`

```rust
#[async_trait]
trait MusicSourceAdapter: Send + Sync {
    fn source_id(&self) -> &str;
    fn kind(&self) -> MusicSourceKind; // Local | SystemSession
    async fn capabilities(&self, ctx: &ProviderContext) -> SourceCapabilities;
    async fn now_playing(&self, ctx: &ProviderContext) -> Result<PlaybackState, MusicSourceError>;
    async fn play(&self, expected_revision: u64, ctx: &ProviderContext) -> Result<PlaybackState, MusicSourceError>;
    async fn pause(&self, expected_revision: u64, ctx: &ProviderContext) -> Result<PlaybackState, MusicSourceError>;
    async fn seek(&self, position_ms: u64, expected_revision: u64, ctx: &ProviderContext) -> Result<PlaybackState, MusicSourceError>;
    async fn next(&self, expected_revision: u64, ctx: &ProviderContext) -> Result<PlaybackState, MusicSourceError>;
    async fn previous(&self, expected_revision: u64, ctx: &ProviderContext) -> Result<PlaybackState, MusicSourceError>;
    async fn set_queue(&self, track_ids: &[TrackId], ctx: &ProviderContext) -> Result<PlaybackState, MusicSourceError>;
    fn subscribe(&self) -> broadcast::Receiver<PlaybackEvent>;
}
```

### 2.1 Invariants

- Every returned `PlaybackState` and emitted `PlaybackEvent` validates against the v1 schemas; revisions increase on material state, current-track or capability changes.
- A method checks its capability immediately before side effects. Unsupported methods return `ERR-1201`; unavailable source returns `ERR-1202`; revision/session identity drift returns `ERR-1203`; unreadable local media returns `ERR-1204`.
- `set_queue` accepts 1..200 opaque local `trackId` values. It is available only on `LocalMusicSource`; it rejects unknown IDs before replacing the current queue.
- `SystemMediaSessionSource` binds to one explicit GSMTC `SourceAppUserModelId` selected from discovered sessions. It never falls back to another media app, never controls browser DOM and never claims Apple Music identity solely from display text.
- GSMTC action results are provisional: after a successful Windows call the adapter re-reads session state within 2 seconds. If the session changed, it returns `ERR-1203` and does not retry against the new session.

### 2.2 TTS interruption protocol

Before an eligible generic voice segment over `SystemMediaSessionSource`, the coordinator snapshots `{ sourceId, sessionIdentity, stateRevision, status }`, pauses only if `pause` is supported and status is `playing`, then plays TTS through the local audio engine. It resumes only if the same session still exists, the revision changes are attributable to the coordinator, the user did not issue a media command, and prior status was `playing`. Otherwise it emits `user_override` or `program_interrupted` and leaves external playback untouched. Eligibility is strict: the text and its generation input contain no GSMTC/Apple metadata, session identifier, capability, timeline or playback event. Track-aware Apple reactions are deterministic local visible text and never enter `TTSProvider`.

## 3. `LLMProvider`

```rust
#[async_trait]
trait LlmProvider: Send + Sync {
    async fn generate_program(
        &self,
        input: ProgramGenerationInput,
        ctx: &ProviderContext,
    ) -> Result<ProgramPlan, ProviderError>;

    async fn generate_turn(
        &self,
        input: CompanionTurnInput,
        ctx: &ProviderContext,
    ) -> Result<CompanionTurn, ProviderError>;

    async fn summarize_session(
        &self,
        input: SessionSummaryInput,
        ctx: &ProviderContext,
    ) -> Result<SessionSummary, ProviderError>;
}
```

### 3.1 Inputs

`ProgramGenerationInput` contains:

- `programId`, selected `sourceId`, local time and optional weather summary.
- AI behavior policy version, user profile summary, approved memories only, recent aggregate feedback and program rules.
- For local mode, 1..200 candidate tracks with opaque `trackId`, title, artist, album, duration, normalized tags and recent-play penalty. It contains no file path or audio bytes.
- `generate_program` 只用于 local mode。System-session 的实际 track metadata、会话身份、capability、时间线与播放事件不进入任何 LLM input；曲目变化由本地状态机触发经 AI behavior 审核的 deterministic 可见文字模板，因此没有 candidate catalog，也不能请求 queue changes。

`CompanionTurnInput` contains the current user message, the same bounded policy/profile context and at most the current session's recent turns. It never augments the request with GSMTC/Apple Music metadata or playback events；用户自己输入的文字仍作为 user data 原样受有界规则处理。

`SessionSummaryInput` is `{ sessionId; coveredFrom; coveredTo; turns; explicitFeedback; localPlaybackFacts; unfinishedTopics }`：

- `turns` 是保留策略选中的 `{ messageId; role: "user" | "assistant"; text; createdAt }[]`；
- `explicitFeedback` 只含 `{ kind: "like" | "skip" | "less_talk"; localTrackId: string | null; occurredAt }[]`；
- `localPlaybackFacts` 只含已实际发生的 local-source `{ localTrackId; completed; occurredAt }[]`，不是候选或预测；
- `unfinishedTopics` 只来自用户/助手已显示的会话文字；
- 对 `system_session` run，`localPlaybackFacts` 必须为空，且所有 GSMTC/Apple metadata、opaque media identity、timeline、capability 与 playback event 均在组装前排除。

External metadata and user text are untrusted data fields, never concatenated into the developer instruction channel. 每次 LLM 调用在序列化后的 provider tokenization 上硬限制 input ≤ 24,000 tokens，并在请求参数设置 output ≤ 4,000 tokens；无法在既定裁剪优先级内满足 input limit 时不出网并返回 validation failure。候选曲目硬限制为 200，超过时必须在调用 adapter 前本机筛选，禁止通过截断 JSON 偷渡更多候选。

### 3.2 Outputs and validation

- `generate_program` returns a local-mode `ProgramPlan` and must use only supplied `trackId` values. Unknown IDs, excessive repetition or policy-violating text is rejected as ERR-1305. System-session plans are assembled locally；track-aware segment 只能是 deterministic visible text，generic segment 只有在完全不含 GSMTC 数据时才可进入 Speech。
- `CompanionTurn` is `{ text: string, proposedMemories: MemoryProposal[] }`, with text 1..2000 characters and 0..3 proposals. A proposal contains `kind`, `content` (1..500) and confidence 0..1; it is persisted as `status: proposed`, never auto-approved.
- `SessionSummary` is `{ summary: string; preferenceSignals: PreferenceSignal[]; proposedMemories: MemoryProposal[] }`；`summary` 为 1..1000 Unicode 字符，只概括显式用户表达、实际 local-source play facts、显式 feedback 与 unfinished topics；`preferenceSignals` 为 0..20 项，每项严格为 `{ kind: "music_tag" | "listening_time" | "feedback" | "narration_density"; label: string; direction: "up" | "stable" | "down"; confidence: number }`，其中 label 1..100 Unicode 字符、confidence 0..1；`proposedMemories` 为 0..3 项并遵守与 `CompanionTurn` 相同约束。它不能创建 approved memory。Orchestration 在本机保存输入的 source message IDs、covered range、prompt/model version 与 `generationKind`；这些 provenance 不是模型自行声明的输出。空/不可发送输入不调用 provider，改用不含推断的 deterministic stats summary、0..20 个 deterministic preference signals 和空 proposals。
- The adapter requests strict structured output, disables provider-hosted tools for MVP, sets OpenAI `store: false`, and does not use provider conversation IDs. Orchestration supplies prior context on each stateless request.

Deadline: 60 seconds per call, including a possible schema-repair attempt. Cancellation discards late output and prevents persistence.

## 4. `TTSProvider`

```rust
#[async_trait]
trait TtsProvider: Send + Sync {
    async fn synthesize(
        &self,
        input: SpeechInput,
        sink: &mut dyn AsyncWrite,
        ctx: &ProviderContext,
    ) -> Result<SpeechArtifact, ProviderError>;
}

struct SpeechInput {
    text: String,          // 1..500 Unicode scalar values in CyberKindred
    voice_id: String,
    model_id: String,
    format: AudioFormat,   // Mp3 in MVP
    speed: f32,            // 0.75..1.25 app policy
    provenance: SpeechProvenance, // LocalProgram | Chat | VoicePreview | SystemSessionGeneric
}

struct SpeechArtifact {
    content_hash: String,  // SHA-256 hex of normalized non-secret request inputs
    format: AudioFormat,
    byte_length: u64,
}

enum SpeechProvenance {
    LocalProgram,
    Chat,
    VoicePreview,
    SystemSessionGeneric,
}
```

The adapter streams bytes into a Rust-owned temporary file, enforces 20 MiB maximum, verifies expected media framing before atomic cache promotion, and deletes partial files on failure/cancel. Cache keys exclude API keys and user identity. TTS text is disclosed to the provider; the UI must identify the voice as AI-generated. Deadline is 45 seconds. Failure degrades to visible text and never blocks music indefinitely.

`VoicePreview` 的 `text` 由 Rust 固定为 preview phrase v1 `你好，我是 CyberKindred，很高兴陪你听一会儿。`，不含用户数据；API-009 不接收 sample text。改变该短句需要契约版本审查。`SystemSessionGeneric` 只能由 orchestration 在 taint/provenance 检查通过后构造，且请求不得包含或由任何 GSMTC/Apple metadata、事件、timeline、capability、session/media identity 派生；track-aware Apple 文字即使短于 500 字符也不得合成。违反该边界在出网前映射 `provider_output_policy_violation` / `ERR-1305`。

## 5. `MetadataProvider`

```rust
#[async_trait]
trait MetadataProvider: Send + Sync {
    async fn match_recording(
        &self,
        input: MetadataQuery,
        ctx: &ProviderContext,
    ) -> Result<Vec<MetadataMatch>, ProviderError>;

    async fn fetch_cover(
        &self,
        release_mbid: Uuid,
        size: CoverSize,
        sink: &mut dyn AsyncWrite,
        ctx: &ProviderContext,
    ) -> Result<CoverArtifact, ProviderError>;
}
```

`MetadataQuery` contains only normalized text tags `{ title, artists, album, durationMs }`; null/empty optional fields are omitted. Audio bytes, fingerprints, local paths, user identity and playback history are prohibited. Results contain MBIDs, normalized display metadata, tags and a deterministic confidence score 0..1. The caller displays or caches low-confidence results but cannot overwrite source tags below the approved confidence threshold in `AI-ORCHESTRATION.md`.

MusicBrainz calls share one process-wide token bucket of one request per second, include a project/version/contact `User-Agent`, cache successful matches permanently until manual refresh, and apply negative-cache expiry of 24 hours. Cover Art Archive is queried only with a validated release MBID obtained from MusicBrainz; images are limited to HTTPS redirects, 5 MiB, declared image MIME types and decoded pixel limits. Deadlines are 15 seconds for metadata and 30 seconds for cover bytes.

## 6. `WeatherProvider`

```rust
#[async_trait]
trait WeatherProvider: Send + Sync {
    async fn search_locations(
        &self,
        query: LocationQuery,
        ctx: &ProviderContext,
    ) -> Result<Vec<WeatherLocationCandidate>, ProviderError>;

    async fn current(
        &self,
        location: WeatherLocation,
        ctx: &ProviderContext,
    ) -> Result<CurrentWeather, ProviderError>;
}
```

`LocationQuery` 为用户显式提交的 `{ text, language: "zh", limit }`，text 2..100 字符，limit 1..10。adapter 只在 API-048 的直接用户动作中把 search text、language 与 count 发送至 Open-Meteo Geocoding API；结果限制为 `{ candidateId, city, region, country, countryCode, latitude, longitude, timezone }`，保存在内存 10 分钟供 API-049 选择，不持久化 query 或未选 candidate。单字符/空查询不出网。

`WeatherLocation` comes from an explicitly selected search candidate. Forecast adapter sends only latitude, longitude, requested current variables and timezone to the configured Open-Meteo endpoint; it does not use IP geolocation, GPS or contacts. `CurrentWeather` is `{ observedAt, temperatureC, apparentTemperatureC, precipitationMm, weatherCode, isDay }` with finite numbers and a provider timestamp.

Successful forecasts cache for 30 minutes by rounded coordinates (4 decimal places) and variable set. An unavailable or stale provider causes the orchestration layer to omit weather after showing last-updated status; it never fabricates conditions. Search and forecast each have a 10-second deadline；search 不自动重试，forecast within the overall deadline at most one retry.

## 7. Conformance and fakes

Each provider has a deterministic fake implementing the same interface. Default unit, integration and E2E suites use fakes and cannot resolve public provider hosts. Conformance suites assert:

- cancellation, deadlines, error mapping and redaction;
- schema-valid outputs and rejection of unknown fields;
- capability enforcement and revision/session races;
- no secret, absolute path or audio bytes in LLM/metadata/weather requests;
- MusicBrainz pacing and User-Agent, cache policy and redirect limits;
- OpenAI `store: false` and stateless Responses requests;
- every LLM request input ≤24,000 tokens, requested output ≤4,000 tokens and candidate count ≤200；summary input/output and provenance obey §3;
- city search only after direct user action, query/result minimization and 10-minute in-memory candidate expiry;
- system-session user override prevents automatic resume；GSMTC-derived data appears in neither Responses nor Speech captures, and only `SystemSessionGeneric` can synthesize.

Tests using real Apple Music, OpenAI or public metadata/weather services are separately tagged, disabled by default and require an explicit operator action.
