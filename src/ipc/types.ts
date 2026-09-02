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

export type OnboardingStep =
  | "welcome"
  | "music_source"
  | "openai_key"
  | "voice"
  | "profile"
  | "city_schedule"
  | "privacy";

export type MusicSourceKind = "local" | "apple_music";
export type NarrationDensity = "quiet" | "balanced" | "frequent";

export interface OnboardingProfile {
  readonly displayName: string;
  readonly companionStyle: "quiet_warm";
  readonly initialPreferences: ReadonlyArray<string>;
  readonly narrationDensity: NarrationDensity;
}

export interface OnboardingState {
  readonly completed: boolean;
  readonly completedSteps: ReadonlyArray<OnboardingStep>;
  readonly sourceSelection: ReadonlyArray<MusicSourceKind>;
  readonly aiMode: "verified" | "local_only" | null;
  readonly voiceMode: "selected" | "text_only" | null;
  readonly cityScheduleMode: "configured" | "not_now" | null;
  readonly profile: OnboardingProfile;
  readonly privacyConfirmations: {
    readonly explicitSound: boolean;
    readonly rawConversationRetention: boolean;
  };
  readonly revision: number;
}

export type OnboardingStepSubmission =
  | { readonly step: "welcome" }
  | { readonly step: "music_source"; readonly sources: ReadonlyArray<MusicSourceKind> }
  | { readonly step: "openai_key"; readonly mode: "verified" | "local_only" }
  | { readonly step: "voice"; readonly mode: "selected" | "text_only" }
  | { readonly step: "profile"; readonly profile: OnboardingProfile }
  | { readonly step: "city_schedule"; readonly mode: "configured" | "not_now" }
  | {
    readonly step: "privacy";
    readonly confirmations: {
      readonly explicitSound: true;
      readonly rawConversationRetention: true;
    };
  };

export interface SaveOnboardingStepRequest {
  readonly clientRequestId: string;
  readonly expectedRevision: number;
  readonly submission: OnboardingStepSubmission;
}

export interface Ack {
  readonly requestId: string;
  readonly revision: number;
}

export interface ValidateSecretRequest {
  readonly clientRequestId: string;
  readonly kind: "openai_api_key";
  readonly origin: string;
  readonly value: string;
}

export interface ValidateSecretResponse {
  readonly requestId: string;
  readonly configured: true;
  readonly verifiedAt: string;
}

export interface DeleteSecretRequest {
  readonly clientRequestId: string;
  readonly kind: "openai_api_key";
  readonly origin: string;
}

export interface DeleteSecretResponse {
  readonly requestId: string;
  readonly configured: false;
}

export interface TestProviderRequest {
  readonly clientRequestId: string;
  readonly kind: "llm" | "tts" | "metadata" | "weather";
}

export interface TestProviderResponse {
  readonly requestId: string;
  readonly ok: boolean;
  readonly latencyMs: number;
  readonly safeMessage: string;
}

export interface WeatherLocation {
  readonly city: string;
  readonly region: string | null;
  readonly country: string;
  readonly countryCode: string;
  readonly latitude: number;
  readonly longitude: number;
  readonly timezone: string;
}

export interface OriginSecretStatus {
  readonly origin: string;
  readonly openaiApiKeyConfigured: boolean;
  readonly lastVerifiedAt: string | null;
}

export interface IntegrationStatus {
  readonly integration: "openai" | "apple_music" | "musicbrainz" | "weather";
  readonly state: "connected" | "degraded" | "disabled" | "unavailable";
  readonly lastSuccessAt: string | null;
  readonly safeMessage: string;
}

export interface SettingsView {
  readonly providerOrigin: string;
  readonly llmModelId: string;
  readonly ttsModelId: string;
  readonly ttsVoiceId: string;
  readonly metadataEnabled: boolean;
  readonly weatherEnabled: boolean;
  readonly defaultSourceId: string | null;
  readonly narrationDensity: NarrationDensity;
  readonly ttsEnabled: boolean;
  readonly audioOutputDeviceId: string | null;
  readonly audioOutputBehavior: "follow_system_default" | "fixed_device";
  readonly minimizeToTray: boolean;
  readonly launchAtStartup: boolean;
  readonly notificationsEnabled: boolean;
  readonly weatherLocation: WeatherLocation | null;
  readonly secretStatus: { readonly origins: ReadonlyArray<OriginSecretStatus> };
  readonly integrationStatuses: ReadonlyArray<IntegrationStatus>;
  readonly revision: number;
}

export interface SettingsPatch {
  readonly providerOrigin?: string;
  readonly llmModelId?: string;
  readonly ttsModelId?: string;
  readonly ttsVoiceId?: string;
  readonly metadataEnabled?: boolean;
  readonly weatherEnabled?: boolean;
  readonly defaultSourceId?: string | null;
  readonly narrationDensity?: NarrationDensity;
  readonly ttsEnabled?: boolean;
  readonly audioOutputDeviceId?: string | null;
  readonly audioOutputBehavior?: "follow_system_default" | "fixed_device";
  readonly minimizeToTray?: boolean;
  readonly launchAtStartup?: boolean;
  readonly notificationsEnabled?: boolean;
  readonly weatherLocationAction?: "clear";
}

export interface UpdateSettingsRequest {
  readonly clientRequestId: string;
  readonly expectedRevision: number;
  readonly patch: SettingsPatch;
}

export interface OperationAccepted {
  readonly operationId: string;
  readonly acceptedAt: string;
}

export interface LibraryRoot {
  readonly rootId: string;
  readonly displayName: string;
  readonly available: boolean;
}

export interface LibraryRootsResponse {
  readonly roots: ReadonlyArray<LibraryRoot>;
  readonly revision: number;
}

export interface PickLibraryRootResponse {
  readonly requestId: string;
  readonly root: LibraryRoot | null;
  readonly revision: number;
}

export interface VoiceView {
  readonly voiceId: string;
  readonly displayName: string;
  readonly previewAvailable: boolean;
}

export interface VoicesResponse {
  readonly voices: ReadonlyArray<VoiceView>;
}

export interface EventEnvelope {
  readonly schemaVersion: string;
  readonly sequence: number;
  readonly occurredAt: string;
}

export type PublicEventPayload = Readonly<Record<string, unknown>> & EventEnvelope;
