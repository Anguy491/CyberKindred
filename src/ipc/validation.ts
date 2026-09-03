import {
  IPC_SCHEMA_VERSION,
  type ApiError,
  type ApiErrorDetails,
  type AppCapabilities,
  type Ack,
  type DeleteSecretResponse,
  type CancelLibraryScanResponse,
  type ErrorId,
  type IntegrationStatus,
  type LibraryRoot,
  type LibraryRootsResponse,
  type MusicSourcesResponse,
  type MemoryRecord,
  type OnboardingProfile,
  type OnboardingState,
  type OnboardingStep,
  type OperationAccepted,
  type OriginSecretStatus,
  type PickLibraryRootResponse,
  type TrackTagView,
  type TrackView,
  type TracksPage,
  type SettingsView,
  type SelectMusicSourceResponse,
  type PlaybackState,
  type ProgramPlan,
  type StartProgramResponse,
  type CancelOperationResponse,
  type MemoryPage,
  type ProfileViewResponse,
  type RejectMemoryResponse,
  type SessionSummaryPage,
  type TestProviderResponse,
  type ValidateSecretResponse,
  type VoiceView,
  type VoicesResponse,
  type WeatherLocation,
  type SourceCapabilities,
  type SourceSummary,
} from "./types";

const ERROR_IDS = new Set<ErrorId>([
  "ERR-1001", "ERR-1002", "ERR-1003", "ERR-1004", "ERR-1005", "ERR-1006",
  "ERR-1101", "ERR-1102", "ERR-1201", "ERR-1202", "ERR-1203", "ERR-1204",
  "ERR-1301", "ERR-1302", "ERR-1303", "ERR-1304", "ERR-1305", "ERR-1401",
  "ERR-1402", "ERR-1501", "ERR-1502", "ERR-1601",
]);
const PROVIDERS = new Set(["llm", "tts", "metadata", "weather"]);
const ONBOARDING_STEPS: ReadonlyArray<OnboardingStep> = [
  "welcome", "music_source", "openai_key", "voice", "profile", "city_schedule", "privacy",
];
const INTEGRATIONS = new Set(["openai", "apple_music", "musicbrainz", "weather"]);

export class IpcResponseValidationError extends Error {
  constructor() {
    super("IPC 响应未通过本地契约校验。");
    this.name = "IpcResponseValidationError";
  }
}

export function parseAppCapabilities(value: unknown): AppCapabilities {
  if (!isRecord(value) || !hasExactKeys(value, [
    "protocolVersion", "appVersion", "platform", "osVersion", "features", "sources", "providers",
  ])) {
    throw new IpcResponseValidationError();
  }
  const { protocolVersion, appVersion, platform, osVersion, features, sources, providers } = value;
  if (
    protocolVersion !== IPC_SCHEMA_VERSION
    || platform !== "windows"
    || !isSafeDisplay(appVersion, 100)
    || !isSafeDisplay(osVersion, 200)
    || !isAppFeatures(features)
    || !isSourceCollection(sources, features)
    || !Array.isArray(providers)
    || !providers.every((provider) => typeof provider === "string" && PROVIDERS.has(provider))
    || new Set(providers).size !== providers.length
  ) {
    throw new IpcResponseValidationError();
  }
  return value as unknown as AppCapabilities;
}

export function parseOnboardingState(value: unknown): OnboardingState {
  if (!isRecord(value) || !hasExactKeys(value, [
    "completed", "completedSteps", "sourceSelection", "aiMode", "voiceMode",
    "cityScheduleMode", "profile", "privacyConfirmations", "revision",
  ])) throw new IpcResponseValidationError();
  const { completedSteps, sourceSelection, privacyConfirmations } = value;
  if (
    typeof value.completed !== "boolean"
    || !Array.isArray(completedSteps)
    || !completedSteps.every((step, index) => step === ONBOARDING_STEPS[index])
    || !Array.isArray(sourceSelection)
    || !sourceSelection.every((source) => source === "local" || source === "apple_music")
    || new Set(sourceSelection).size !== sourceSelection.length
    || (value.aiMode !== null && value.aiMode !== "verified" && value.aiMode !== "local_only")
    || (value.voiceMode !== null && value.voiceMode !== "selected" && value.voiceMode !== "text_only")
    || (value.cityScheduleMode !== null
      && value.cityScheduleMode !== "configured" && value.cityScheduleMode !== "not_now")
    || !isOnboardingProfile(value.profile)
    || !isRecord(privacyConfirmations)
    || !hasExactKeys(privacyConfirmations, ["explicitSound", "rawConversationRetention"])
    || typeof privacyConfirmations.explicitSound !== "boolean"
    || typeof privacyConfirmations.rawConversationRetention !== "boolean"
    || !isNonNegativeInteger(value.revision)
  ) throw new IpcResponseValidationError();
  const actuallyComplete = completedSteps.length === ONBOARDING_STEPS.length
    && privacyConfirmations.explicitSound
    && privacyConfirmations.rawConversationRetention;
  const coherentPrefix = (completedSteps.length >= 2 ? sourceSelection.length > 0 : sourceSelection.length === 0)
    && (completedSteps.length >= 3 ? value.aiMode !== null : value.aiMode === null)
    && (completedSteps.length >= 4 ? value.voiceMode !== null : value.voiceMode === null)
    && (completedSteps.length >= 6 ? value.cityScheduleMode !== null : value.cityScheduleMode === null)
    && (completedSteps.length === 7
      ? privacyConfirmations.explicitSound && privacyConfirmations.rawConversationRetention
      : !privacyConfirmations.explicitSound && !privacyConfirmations.rawConversationRetention);
  if (value.completed !== actuallyComplete || !coherentPrefix) throw new IpcResponseValidationError();
  return value as unknown as OnboardingState;
}

export function parseAck(value: unknown): Ack {
  if (!isRecord(value) || !hasExactKeys(value, ["requestId", "revision"])
    || !isUuid(value.requestId) || !isNonNegativeInteger(value.revision)) {
    throw new IpcResponseValidationError();
  }
  return value as unknown as Ack;
}

export function parseValidateSecretResponse(value: unknown): ValidateSecretResponse {
  if (!isRecord(value) || !hasExactKeys(value, ["requestId", "configured", "verifiedAt"])
    || !isUuid(value.requestId) || value.configured !== true || !isTimestamp(value.verifiedAt)) {
    throw new IpcResponseValidationError();
  }
  return value as unknown as ValidateSecretResponse;
}

export function parseDeleteSecretResponse(value: unknown): DeleteSecretResponse {
  if (!isRecord(value) || !hasExactKeys(value, ["requestId", "configured"])
    || !isUuid(value.requestId) || value.configured !== false) {
    throw new IpcResponseValidationError();
  }
  return value as unknown as DeleteSecretResponse;
}

export function parseTestProviderResponse(value: unknown): TestProviderResponse {
  if (!isRecord(value) || !hasExactKeys(value, ["requestId", "ok", "latencyMs", "safeMessage"])
    || !isUuid(value.requestId) || typeof value.ok !== "boolean"
    || !isNonNegativeInteger(value.latencyMs) || !isSafeDisplay(value.safeMessage, 300)) {
    throw new IpcResponseValidationError();
  }
  return value as unknown as TestProviderResponse;
}

export function parseSettingsView(value: unknown): SettingsView {
  if (!isRecord(value) || !hasExactKeys(value, [
    "providerOrigin", "llmModelId", "ttsModelId", "ttsVoiceId", "metadataEnabled",
    "weatherEnabled", "defaultSourceId", "narrationDensity", "ttsEnabled",
    "audioOutputDeviceId", "audioOutputBehavior", "minimizeToTray", "launchAtStartup",
    "notificationsEnabled", "weatherLocation", "secretStatus", "integrationStatuses", "revision",
  ])) throw new IpcResponseValidationError();
  const booleanKeys = [
    "metadataEnabled", "weatherEnabled", "ttsEnabled", "minimizeToTray",
    "launchAtStartup", "notificationsEnabled",
  ] as const;
  if (
    !isCanonicalHttpsOrigin(value.providerOrigin)
    || !isSettingToken(value.llmModelId) || !isSettingToken(value.ttsModelId)
    || !isSettingToken(value.ttsVoiceId)
    || !booleanKeys.every((key) => typeof value[key] === "boolean")
    || (value.defaultSourceId !== null && !isSafeToken(value.defaultSourceId))
    || !isNarrationDensity(value.narrationDensity)
    || (value.audioOutputDeviceId !== null && !isBoundedText(value.audioOutputDeviceId, 200))
    || (value.audioOutputBehavior !== "follow_system_default" && value.audioOutputBehavior !== "fixed_device")
    || (value.weatherLocation !== null && !isWeatherLocation(value.weatherLocation))
    || !isSecretStatus(value.secretStatus)
    || !Array.isArray(value.integrationStatuses)
    || !value.integrationStatuses.every(isIntegrationStatus)
    || new Set(value.integrationStatuses.map((status) => (status as IntegrationStatus).integration)).size
      !== value.integrationStatuses.length
    || !isNonNegativeInteger(value.revision)
  ) throw new IpcResponseValidationError();
  return value as unknown as SettingsView;
}

export function parseOperationAccepted(value: unknown): OperationAccepted {
  if (!isRecord(value) || !hasExactKeys(value, ["operationId", "acceptedAt"])
    || !isUuid(value.operationId) || !isTimestamp(value.acceptedAt)) {
    throw new IpcResponseValidationError();
  }
  return value as unknown as OperationAccepted;
}

export function parseLibraryRootsResponse(value: unknown): LibraryRootsResponse {
  if (!isRecord(value) || !hasExactKeys(value, ["roots", "revision"])
    || !Array.isArray(value.roots) || !value.roots.every(isLibraryRoot)
    || new Set(value.roots.map((root) => (root as LibraryRoot).rootId)).size !== value.roots.length
    || !isNonNegativeInteger(value.revision)) {
    throw new IpcResponseValidationError();
  }
  return value as unknown as LibraryRootsResponse;
}

export function parsePickLibraryRootResponse(value: unknown): PickLibraryRootResponse {
  if (!isRecord(value) || !hasExactKeys(value, ["requestId", "root", "revision"])
    || !isUuid(value.requestId) || (value.root !== null && !isLibraryRoot(value.root))
    || !isNonNegativeInteger(value.revision)) {
    throw new IpcResponseValidationError();
  }
  return value as unknown as PickLibraryRootResponse;
}

export function parseCancelLibraryScanResponse(value: unknown): CancelLibraryScanResponse {
  if (!isRecord(value) || !hasExactKeys(value, ["requestId", "operationId", "state"])
    || !isUuid(value.requestId) || !isUuid(value.operationId)
    || (value.state !== "cancelled" && value.state !== "already_terminal")) {
    throw new IpcResponseValidationError();
  }
  return value as unknown as CancelLibraryScanResponse;
}

export function parseTracksPage(value: unknown): TracksPage {
  if (!isRecord(value) || !hasExactKeys(value, ["items", "nextCursor"])
    || !Array.isArray(value.items) || value.items.length > 200
    || !value.items.every(isTrackView)
    || new Set(value.items.map((item) => (item as TrackView).trackId)).size !== value.items.length
    || (value.nextCursor !== null && !isOpaqueCursor(value.nextCursor))) {
    throw new IpcResponseValidationError();
  }
  return value as unknown as TracksPage;
}

export function parseVoicesResponse(value: unknown): VoicesResponse {
  if (!isRecord(value) || !hasExactKeys(value, ["voices"])
    || !Array.isArray(value.voices) || !value.voices.every(isVoiceView)
    || new Set(value.voices.map((voice) => (voice as VoiceView).voiceId)).size !== value.voices.length) {
    throw new IpcResponseValidationError();
  }
  return value as unknown as VoicesResponse;
}

export function parseMusicSourcesResponse(value: unknown): MusicSourcesResponse {
  if (!isRecord(value) || !hasExactKeys(value, ["sources"])
    || !Array.isArray(value.sources) || !value.sources.every(isSourceSummary)
    || new Set(value.sources.map((source) => (source as SourceSummary).sourceId)).size
      !== value.sources.length) {
    throw new IpcResponseValidationError();
  }
  return value as unknown as MusicSourcesResponse;
}

export function parseSelectMusicSourceResponse(value: unknown): SelectMusicSourceResponse {
  if (!isRecord(value) || !hasExactKeys(value, ["requestId", "state"])
    || !isUuid(value.requestId) || !isPlaybackState(value.state)) {
    throw new IpcResponseValidationError();
  }
  return value as unknown as SelectMusicSourceResponse;
}

export function parsePlaybackState(value: unknown): PlaybackState {
  if (!isPlaybackState(value)) throw new IpcResponseValidationError();
  return value;
}

export function parseStartProgramResponse(value: unknown): StartProgramResponse {
  if (!isRecord(value) || !hasExactKeys(value, ["requestId", "programId", "plan"])
    || !isUuid(value.requestId) || !isUuid(value.programId)
    || (value.plan !== null && !isProgramPlan(value.plan))
    || (value.plan !== null && value.plan.programId !== value.programId)) {
    throw new IpcResponseValidationError();
  }
  return value as unknown as StartProgramResponse;
}

export function parseCancelOperationResponse(value: unknown): CancelOperationResponse {
  if (!isRecord(value) || !hasExactKeys(value, ["requestId", "operationId", "state"])
    || !isUuid(value.requestId) || !isUuid(value.operationId)
    || (value.state !== "cancelled" && value.state !== "already_terminal")) {
    throw new IpcResponseValidationError();
  }
  return value as unknown as CancelOperationResponse;
}

export function parseMemoryRecord(value: unknown): MemoryRecord {
  if (!isMemoryRecord(value)) throw new IpcResponseValidationError();
  return value;
}

export function parseMemoryPage(value: unknown): MemoryPage {
  if (!isRecord(value) || !hasExactKeys(value, ["items", "nextCursor"])
    || !Array.isArray(value.items) || value.items.length > 100
    || !value.items.every(isMemoryRecord)
    || new Set(value.items.map((item) => (item as MemoryRecord).memoryId)).size !== value.items.length
    || (value.nextCursor !== null && !isOpaqueCursor(value.nextCursor))) {
    throw new IpcResponseValidationError();
  }
  return value as unknown as MemoryPage;
}

export function parseRejectMemoryResponse(value: unknown): RejectMemoryResponse {
  if (!isRecord(value) || !hasExactKeys(value, [
    "requestId", "memoryId", "status", "rejectedAt", "contentDeleteAt", "revision",
  ]) || !isUuid(value.requestId) || !isUuid(value.memoryId) || value.status !== "rejected"
    || !isTimestamp(value.rejectedAt) || !isTimestamp(value.contentDeleteAt)
    || Date.parse(value.contentDeleteAt) < Date.parse(value.rejectedAt)
    || !isNonNegativeInteger(value.revision)) {
    throw new IpcResponseValidationError();
  }
  return value as unknown as RejectMemoryResponse;
}

export function parseProfileViewResponse(value: unknown): ProfileViewResponse {
  if (!isRecord(value) || !hasExactKeys(value, ["profile", "preferenceTrends", "revision"])
    || !isUserProfileView(value.profile) || !Array.isArray(value.preferenceTrends)
    || value.preferenceTrends.length > 100 || !value.preferenceTrends.every(isPreferenceTrend)
    || !isNonNegativeInteger(value.revision)) {
    throw new IpcResponseValidationError();
  }
  return value as unknown as ProfileViewResponse;
}

export function parseSessionSummaryPage(value: unknown): SessionSummaryPage {
  if (!isRecord(value) || !hasExactKeys(value, ["items", "nextCursor"])
    || !Array.isArray(value.items) || value.items.length > 100
    || !value.items.every(isSessionSummary)
    || new Set(value.items.map((item) => (item as { summaryId: string }).summaryId)).size
      !== value.items.length
    || (value.nextCursor !== null && !isOpaqueCursor(value.nextCursor))) {
    throw new IpcResponseValidationError();
  }
  return value as unknown as SessionSummaryPage;
}

export function normalizeApiError(value: unknown): ApiError {
  if (isApiError(value)) {
    return value;
  }
  return localUnexpectedError();
}

function localUnexpectedError(): ApiError {
  return {
    schemaVersion: IPC_SCHEMA_VERSION,
    errorId: "ERR-1601",
    safeMessage: "本地通信失败，请刷新后重试。",
    retryable: false,
    retryAfterMs: null,
    correlationId: createCorrelationId(),
    details: {
      field: null,
      reason: "unexpected_internal",
      currentRevision: null,
      capability: null,
      operationId: null,
    },
  };
}

export function isApiError(value: unknown): value is ApiError {
  if (!isRecord(value) || !hasExactKeys(value, [
    "schemaVersion", "errorId", "safeMessage", "retryable", "retryAfterMs", "correlationId", "details",
  ])) {
    return false;
  }
  return value.schemaVersion === IPC_SCHEMA_VERSION
    && typeof value.errorId === "string"
    && ERROR_IDS.has(value.errorId as ErrorId)
    && isSafeDisplay(value.safeMessage, 300)
    && typeof value.retryable === "boolean"
    && (value.retryAfterMs === null || isNonNegativeInteger(value.retryAfterMs))
    && isUuid(value.correlationId)
    && (value.details === null || isApiErrorDetails(value.details));
}

function isApiErrorDetails(value: unknown): value is ApiErrorDetails {
  if (!isRecord(value) || !hasExactKeys(value, [
    "field", "reason", "currentRevision", "capability", "operationId",
  ])) {
    return false;
  }
  return isNullableSafeToken(value.field)
    && isNullableSafeToken(value.reason)
    && (value.currentRevision === null || isNonNegativeInteger(value.currentRevision))
    && isNullableSafeToken(value.capability)
    && (value.operationId === null || isUuid(value.operationId));
}

function isAppFeatures(value: unknown): boolean {
  return isRecord(value)
    && hasExactKeys(value, [
      "localLibrary", "systemMediaSession", "musicKit", "appleMusicDomControl", "externalHttpApi",
    ])
    && value.localLibrary === true
    && typeof value.systemMediaSession === "boolean"
    && value.musicKit === false
    && value.appleMusicDomControl === false
    && value.externalHttpApi === false;
}

function isSourceSummary(value: unknown): value is SourceSummary {
  if (!isRecord(value) || !hasExactKeys(value, [
    "sourceId", "kind", "displayName", "connected", "capabilities",
  ])) {
    return false;
  }
  return isContractToken(value.sourceId, 128)
    && (value.kind === "local" || value.kind === "system_session")
    && isSafeDisplay(value.displayName, 200)
    && typeof value.connected === "boolean"
    && isSourceCapabilities(value.capabilities)
    && !(value.kind === "system_session" && value.capabilities.setQueue);
}

export function isPlaybackState(value: unknown): value is PlaybackState {
  if (!isRecord(value) || !hasExactKeys(value, [
    "schemaVersion", "sourceId", "sourceKind", "status", "capabilities", "currentTrack",
    "positionMs", "durationMs", "revision", "updatedAt", "lastError",
  ])) return false;
  const disconnected = value.status === "disconnected";
  const error = value.status === "error";
  return value.schemaVersion === IPC_SCHEMA_VERSION
    && isContractToken(value.sourceId, 128)
    && (value.sourceKind === "local" || value.sourceKind === "system_session")
    && isPlaybackStatus(value.status)
    && isSourceCapabilities(value.capabilities)
    && !(value.sourceKind === "system_session" && value.capabilities.setQueue)
    && (value.currentTrack === null || isPlaybackTrack(value.currentTrack))
    && isBoundedMilliseconds(value.positionMs)
    && (value.durationMs === null || isBoundedMilliseconds(value.durationMs))
    && isNonNegativeInteger(value.revision)
    && isTimestamp(value.updatedAt)
    && (value.lastError === null || isPlaybackSafeError(value.lastError))
    && (!disconnected || (value.currentTrack === null && value.positionMs === 0 && value.durationMs === null))
    && (!error || value.lastError !== null);
}

export function isProgramPlan(value: unknown): value is ProgramPlan {
  if (!isRecord(value) || !hasExactKeys(value, [
    "schemaVersion", "programId", "sourceId", "mode", "createdAt", "segments",
  ])) return false;
  if (value.schemaVersion !== IPC_SCHEMA_VERSION || !isUuid(value.programId)
    || !isContractToken(value.sourceId, 128)
    || (value.mode !== "local" && value.mode !== "system_session")
    || !isTimestamp(value.createdAt) || !Array.isArray(value.segments)
    || value.segments.length < 1 || value.segments.length > 200
    || !value.segments.every(isProgramSegment)) return false;
  const trackCount = value.segments.filter((segment) => (segment as { type?: unknown }).type === "track").length;
  return value.mode === "local" ? trackCount > 0 : trackCount === 0;
}

export function isMemoryRecord(value: unknown): value is MemoryRecord {
  if (!isRecord(value) || !hasExactKeys(value, [
    "schemaVersion", "memoryId", "status", "kind", "content", "confidence",
    "sourceSessionId", "createdAt", "updatedAt", "approvedAt", "lastUsedAt", "enabled",
    "revision",
  ])) return false;
  const statusCoherent = value.status === "proposed"
    ? value.approvedAt === null && value.enabled === false
    : value.status === "approved"
      ? isTimestamp(value.approvedAt) && value.enabled === true
      : value.status === "disabled" && isTimestamp(value.approvedAt) && value.enabled === false;
  return value.schemaVersion === IPC_SCHEMA_VERSION
    && isUuid(value.memoryId)
    && (value.status === "proposed" || value.status === "approved" || value.status === "disabled")
    && (value.kind === "preference" || value.kind === "routine"
      || value.kind === "boundary" || value.kind === "biographical")
    && isContractText(value.content, 1, 500)
    && typeof value.confidence === "number" && Number.isFinite(value.confidence)
    && value.confidence >= 0 && value.confidence <= 1
    && (value.sourceSessionId === null || isUuid(value.sourceSessionId))
    && isTimestamp(value.createdAt) && isTimestamp(value.updatedAt)
    && (value.approvedAt === null || isTimestamp(value.approvedAt))
    && (value.lastUsedAt === null || isTimestamp(value.lastUsedAt))
    && typeof value.enabled === "boolean" && isNonNegativeInteger(value.revision)
    && statusCoherent;
}

function isUserProfileView(value: unknown): boolean {
  return isRecord(value) && hasExactKeys(value, [
    "displayName", "companionStyle", "initialPreferences", "narrationDensity", "weatherLocation",
  ]) && isContractText(value.displayName, 0, 80) && value.companionStyle === "quiet_warm"
    && Array.isArray(value.initialPreferences) && value.initialPreferences.length <= 20
    && value.initialPreferences.every((item) => isContractText(item, 1, 100))
    && isNarrationDensity(value.narrationDensity)
    && (value.weatherLocation === null || isWeatherLocation(value.weatherLocation));
}

function isPreferenceTrend(value: unknown): boolean {
  return isRecord(value) && hasExactKeys(value, [
    "kind", "label", "direction", "sampleCount", "windowDays",
  ]) && isSafeToken(value.kind) && isSafeDisplay(value.label, 100)
    && (value.direction === "up" || value.direction === "stable" || value.direction === "down")
    && isNonNegativeInteger(value.sampleCount) && isNonNegativeInteger(value.windowDays)
    && value.windowDays > 0;
}

function isSessionSummary(value: unknown): boolean {
  return isRecord(value) && hasExactKeys(value, [
    "summaryId", "coveredFrom", "coveredTo", "summary", "generationKind", "revision",
  ]) && isUuid(value.summaryId) && isTimestamp(value.coveredFrom) && isTimestamp(value.coveredTo)
    && Date.parse(value.coveredTo) >= Date.parse(value.coveredFrom)
    && isContractText(value.summary, 1, 1_000)
    && (value.generationKind === "llm" || value.generationKind === "deterministic")
    && isNonNegativeInteger(value.revision);
}

function isPlaybackStatus(value: unknown): boolean {
  return value === "disconnected" || value === "idle" || value === "loading"
    || value === "playing" || value === "paused" || value === "stopped" || value === "error";
}

function isPlaybackTrack(value: unknown): boolean {
  return isRecord(value) && hasExactKeys(value, [
    "trackId", "title", "artist", "album", "artworkUri", "origin",
  ]) && isContractToken(value.trackId, 128)
    && isContractText(value.title, 1, 300)
    && (value.artist === null || isContractText(value.artist, 1, 300))
    && (value.album === null || isContractText(value.album, 1, 300))
    && (value.artworkUri === null || (typeof value.artworkUri === "string"
      && value.artworkUri.length <= 512 && /^asset:\/\/[A-Za-z0-9._:/-]+$/u.test(value.artworkUri)))
    && (value.origin === "local" || value.origin === "system_session");
}

function isPlaybackSafeError(value: unknown): boolean {
  return isRecord(value) && hasExactKeys(value, ["errorId", "safeMessage", "retryable"])
    && typeof value.errorId === "string" && /^ERR-[0-9]{4}$/u.test(value.errorId)
    && isContractText(value.safeMessage, 1, 300) && typeof value.retryable === "boolean";
}

function isProgramSegment(value: unknown): boolean {
  if (!isRecord(value) || !isUuid(value.segmentId)) return false;
  if (value.type === "track") {
    return hasExactKeys(value, ["type", "segmentId", "trackId", "segueText"])
      && isContractToken(value.trackId, 128)
      && (value.segueText === null || isContractText(value.segueText, 1, 400));
  }
  return value.type === "voice"
    && hasExactKeys(value, ["type", "segmentId", "text", "trigger"])
    && isContractText(value.text, 1, 500)
    && (value.trigger === "opening" || value.trigger === "between_tracks"
      || value.trigger === "user_message" || value.trigger === "track_changed"
      || value.trigger === "closing");
}

function isSourceCollection(value: unknown, features: unknown): value is SourceSummary[] {
  if (!Array.isArray(value) || !value.every(isSourceSummary) || !isRecord(features)) {
    return false;
  }
  const sourceIds = value.map((source) => source.sourceId);
  return new Set(sourceIds).size === sourceIds.length
    && (features.systemMediaSession === true
      || value.every((source) => source.kind !== "system_session"));
}

function isSourceCapabilities(value: unknown): value is SourceCapabilities {
  return isRecord(value)
    && hasExactKeys(value, ["play", "pause", "seek", "next", "previous", "setQueue"])
    && Object.values(value).every((field) => typeof field === "boolean");
}

function isOnboardingProfile(value: unknown): value is OnboardingProfile {
  return isRecord(value)
    && hasExactKeys(value, ["displayName", "companionStyle", "initialPreferences", "narrationDensity"])
    && isContractText(value.displayName, 0, 80)
    && value.companionStyle === "quiet_warm"
    && Array.isArray(value.initialPreferences)
    && value.initialPreferences.length <= 20
    && value.initialPreferences.every((item) => isContractText(item, 1, 100))
    && isNarrationDensity(value.narrationDensity);
}

function isNarrationDensity(value: unknown): boolean {
  return value === "quiet" || value === "balanced" || value === "frequent";
}

function isSecretStatus(value: unknown): boolean {
  return isRecord(value) && hasExactKeys(value, ["origins"])
    && Array.isArray(value.origins) && value.origins.every(isOriginSecretStatus)
    && new Set(value.origins.map((entry) => (entry as OriginSecretStatus).origin)).size
      === value.origins.length;
}

function isOriginSecretStatus(value: unknown): value is OriginSecretStatus {
  return isRecord(value) && hasExactKeys(value, [
    "origin", "openaiApiKeyConfigured", "lastVerifiedAt",
  ]) && isCanonicalHttpsOrigin(value.origin)
    && typeof value.openaiApiKeyConfigured === "boolean"
    && (value.lastVerifiedAt === null || isTimestamp(value.lastVerifiedAt));
}

function isIntegrationStatus(value: unknown): value is IntegrationStatus {
  return isRecord(value) && hasExactKeys(value, [
    "integration", "state", "lastSuccessAt", "safeMessage",
  ]) && typeof value.integration === "string" && INTEGRATIONS.has(value.integration)
    && (value.state === "connected" || value.state === "degraded"
      || value.state === "disabled" || value.state === "unavailable")
    && (value.lastSuccessAt === null || isTimestamp(value.lastSuccessAt))
    && isSafeDisplay(value.safeMessage, 300);
}

function isWeatherLocation(value: unknown): value is WeatherLocation {
  return isRecord(value) && hasExactKeys(value, [
    "city", "region", "country", "countryCode", "latitude", "longitude", "timezone",
  ]) && isSafeDisplay(value.city, 100)
    && (value.region === null || isSafeDisplay(value.region, 100))
    && isSafeDisplay(value.country, 100)
    && typeof value.countryCode === "string" && /^[A-Z]{2}$/u.test(value.countryCode)
    && typeof value.latitude === "number" && Number.isFinite(value.latitude)
    && value.latitude >= -90 && value.latitude <= 90
    && typeof value.longitude === "number" && Number.isFinite(value.longitude)
    && value.longitude >= -180 && value.longitude <= 180
    && typeof value.timezone === "string"
    && /^[A-Za-z0-9._+-]+(?:\/[A-Za-z0-9._+-]+)+$/u.test(value.timezone)
    && value.timezone.length <= 100;
}

function isLibraryRoot(value: unknown): value is LibraryRoot {
  return isRecord(value) && hasExactKeys(value, ["rootId", "displayName", "available"])
    && isUuid(value.rootId) && isSafeDisplay(value.displayName, 200)
    && typeof value.available === "boolean";
}

function isTrackView(value: unknown): value is TrackView {
  if (!isRecord(value) || !hasExactKeys(value, [
    "trackId", "availability", "durationMs", "artworkAvailable", "original", "enriched", "matchStatus",
  ])) return false;
  const validAvailability = value.availability === "playable" || value.availability === "missing"
    || value.availability === "corrupt" || value.availability === "unsupported";
  const validMatch = value.matchStatus === "matched" || value.matchStatus === "unmatched"
    || value.matchStatus === "review";
  const enriched = value.enriched;
  const validEnriched = enriched === null || (isRecord(enriched)
    && hasExactKeys(enriched, ["title", "artist", "album", "provider", "confidence", "fetchedAt"])
    && hasValidTrackTagFields(enriched)
    && enriched.provider === "musicbrainz"
    && typeof enriched.confidence === "number" && Number.isFinite(enriched.confidence)
    && enriched.confidence >= 0 && enriched.confidence <= 1
    && isTimestamp(enriched.fetchedAt));
  return isUuid(value.trackId) && validAvailability
    && isNonNegativeInteger(value.durationMs)
    && typeof value.artworkAvailable === "boolean"
    && isTrackTags(value.original) && validEnriched && validMatch
    && ((value.matchStatus === "unmatched") === (value.enriched === null));
}

function isTrackTags(value: unknown): value is TrackTagView {
  return isRecord(value) && hasExactKeys(value, ["title", "artist", "album"])
    && hasValidTrackTagFields(value);
}

function hasValidTrackTagFields(value: Record<string, unknown>): boolean {
  return [value.title, value.artist, value.album]
    .every((field) => field === null || isContractText(field, 1, 1_000));
}

function isOpaqueCursor(value: unknown): value is string {
  return typeof value === "string" && value.length >= 1 && value.length <= 512 && !/\p{Cc}/u.test(value);
}

function isVoiceView(value: unknown): value is VoiceView {
  return isRecord(value) && hasExactKeys(value, ["voiceId", "displayName", "previewAvailable"])
    && isSettingToken(value.voiceId) && isSafeDisplay(value.displayName, 100)
    && typeof value.previewAvailable === "boolean";
}

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasExactKeys(value: Record<string, unknown>, expected: ReadonlyArray<string>): boolean {
  const actual = Object.keys(value);
  return actual.length === expected.length && expected.every((key) => Object.hasOwn(value, key));
}

function isSafeDisplay(value: unknown, maxLength: number): value is string {
  return typeof value === "string"
    && value.length >= 1
    && value.length <= maxLength
    && !/\p{Cc}/u.test(value)
    && !/[\\/]/u.test(value);
}

function isContractText(value: unknown, minCodePoints: number, maxCodePoints: number): value is string {
  if (typeof value !== "string") return false;
  const count = Array.from(value).length;
  return count >= minCodePoints && count <= maxCodePoints;
}

function isBoundedText(value: unknown, maxLength: number): value is string {
  return typeof value === "string"
    && value.length >= 1
    && value.length <= maxLength
    && !/\p{Cc}/u.test(value);
}

function isSettingToken(value: unknown): value is string {
  return typeof value === "string" && /^[A-Za-z0-9_.:@-]{1,100}$/u.test(value);
}

function isCanonicalHttpsOrigin(value: unknown): value is string {
  if (typeof value !== "string" || value.length > 300) return false;
  try {
    const parsed = new URL(value);
    return parsed.protocol === "https:"
      && parsed.username === "" && parsed.password === ""
      && parsed.pathname === "/" && parsed.search === "" && parsed.hash === ""
      && !/^\[.*\]$|^\d{1,3}(?:\.\d{1,3}){3}$/u.test(parsed.hostname);
  } catch {
    return false;
  }
}

function isTimestamp(value: unknown): value is string {
  return typeof value === "string" && value.length <= 40 && !Number.isNaN(Date.parse(value));
}

function isSafeToken(value: unknown): value is string {
  return typeof value === "string" && /^[A-Za-z0-9_.:-]{1,100}$/u.test(value);
}

function isContractToken(value: unknown, maxLength: number): value is string {
  return typeof value === "string" && value.length <= maxLength
    && /^[A-Za-z0-9._:-]+$/u.test(value);
}

function isBoundedMilliseconds(value: unknown): value is number {
  return isNonNegativeInteger(value) && value <= 86_400_000;
}

function isNullableSafeToken(value: unknown): value is string | null {
  return value === null || isSafeToken(value);
}

function isNonNegativeInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function isUuid(value: unknown): value is string {
  return typeof value === "string"
    && /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/iu.test(value);
}

function createCorrelationId(): string {
  return globalThis.crypto.randomUUID();
}
