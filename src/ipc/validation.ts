import {
  IPC_SCHEMA_VERSION,
  type ApiError,
  type ApiErrorDetails,
  type AppCapabilities,
  type ErrorId,
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

function isApiError(value: unknown): value is ApiError {
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
  return isSafeToken(value.sourceId)
    && (value.kind === "local" || value.kind === "system_session")
    && isSafeDisplay(value.displayName, 200)
    && typeof value.connected === "boolean"
    && isSourceCapabilities(value.capabilities)
    && !(value.kind === "system_session" && value.capabilities.setQueue);
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

function isSafeToken(value: unknown): value is string {
  return typeof value === "string" && /^[A-Za-z0-9_.:-]{1,100}$/u.test(value);
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
