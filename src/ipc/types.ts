export const IPC_SCHEMA_VERSION = "1.0.0" as const;

export type ErrorId =
  | "ERR-1001"
  | "ERR-1002"
  | "ERR-1003"
  | "ERR-1004"
  | "ERR-1005"
  | "ERR-1006"
  | "ERR-1101"
  | "ERR-1102"
  | "ERR-1201"
  | "ERR-1202"
  | "ERR-1203"
  | "ERR-1204"
  | "ERR-1301"
  | "ERR-1302"
  | "ERR-1303"
  | "ERR-1304"
  | "ERR-1305"
  | "ERR-1401"
  | "ERR-1402"
  | "ERR-1501"
  | "ERR-1502"
  | "ERR-1601";

export interface ApiErrorDetails {
  readonly field: string | null;
  readonly reason: string | null;
  readonly currentRevision: number | null;
  readonly capability: string | null;
  readonly operationId: string | null;
}

export interface ApiError {
  readonly schemaVersion: typeof IPC_SCHEMA_VERSION;
  readonly errorId: ErrorId;
  readonly safeMessage: string;
  readonly retryable: boolean;
  readonly retryAfterMs: number | null;
  readonly correlationId: string;
  readonly details: ApiErrorDetails | null;
}

export interface SourceCapabilities {
  readonly play: boolean;
  readonly pause: boolean;
  readonly seek: boolean;
  readonly next: boolean;
  readonly previous: boolean;
  readonly setQueue: boolean;
}

export interface SourceSummary {
  readonly sourceId: string;
  readonly kind: "local" | "system_session";
  readonly displayName: string;
  readonly connected: boolean;
  readonly capabilities: SourceCapabilities;
}

export interface AppCapabilities {
  readonly protocolVersion: typeof IPC_SCHEMA_VERSION;
  readonly appVersion: string;
  readonly platform: "windows";
  readonly osVersion: string;
  readonly features: {
    readonly localLibrary: true;
    readonly systemMediaSession: boolean;
    readonly musicKit: false;
    readonly appleMusicDomControl: false;
    readonly externalHttpApi: false;
  };
  readonly sources: ReadonlyArray<SourceSummary>;
  readonly providers: ReadonlyArray<"llm" | "tts" | "metadata" | "weather">;
}

export interface EventEnvelope {
  readonly schemaVersion: string;
  readonly sequence: number;
  readonly occurredAt: string;
}

export type PublicEventPayload = Readonly<Record<string, unknown>> & EventEnvelope;
