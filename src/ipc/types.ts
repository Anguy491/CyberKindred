import type { MemoryRecord, PlaybackState, ProgramPlan, ScheduleRule } from "../contracts";

export type { MemoryRecord, PlaybackEvent, PlaybackState, ProgramPlan, ScheduleRule } from "../contracts";

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

export interface MusicSourcesResponse {
  readonly sources: ReadonlyArray<SourceSummary>;
}

export interface SelectMusicSourceRequest {
  readonly clientRequestId: string;
  readonly sourceId: string;
}

export interface SelectMusicSourceResponse {
  readonly requestId: string;
  readonly state: PlaybackState;
}

export interface PlaybackControlRequest {
  readonly clientRequestId: string;
  readonly expectedStateRevision: number;
}

export interface SeekPlaybackRequest extends PlaybackControlRequest {
  readonly positionMs: number;
}

export interface StartProgramRequest {
  readonly clientRequestId: string;
  readonly sourceId: string;
  readonly trigger: "manual" | "notification";
}

export interface StartProgramResponse {
  readonly requestId: string;
  readonly programId: string;
  readonly plan: ProgramPlan | null;
}

export interface StopProgramRequest {
  readonly clientRequestId: string;
  readonly programId: string;
}

export interface CancelOperationRequest {
  readonly clientRequestId: string;
  readonly operationId: string;
  readonly expectedKind: "chat" | "voice_preview" | "library_scan" | "data_export";
}

export interface CancelOperationResponse {
  readonly requestId: string;
  readonly operationId: string;
  readonly state: "cancelled" | "already_terminal";
}

export interface SubmitChatRequest {
  readonly clientRequestId: string;
  readonly programId: string;
  readonly text: string;
}

export interface SubmitFeedbackRequest {
  readonly clientRequestId: string;
  readonly programId: string;
  readonly trackId: string | null;
  readonly kind: "like" | "skip" | "less_talk";
}

export interface ListMemoriesRequest {
  readonly cursor: string | null;
  readonly limit: number;
  readonly status: "proposed" | "approved" | "disabled" | null;
}

export interface MemoryPage {
  readonly items: ReadonlyArray<MemoryRecord>;
  readonly nextCursor: string | null;
}

export interface MemoryMutationRequest {
  readonly clientRequestId: string;
  readonly memoryId: string;
  readonly expectedRevision: number;
}

export interface UpdateMemoryRequest extends MemoryMutationRequest {
  readonly content: string;
  readonly enabled: boolean;
}

export interface RejectMemoryResponse {
  readonly requestId: string;
  readonly memoryId: string;
  readonly status: "rejected";
  readonly rejectedAt: string;
  readonly contentDeleteAt: string;
  readonly revision: number;
}

export interface UserProfileView {
  readonly displayName: string;
  readonly companionStyle: "quiet_warm";
  readonly initialPreferences: ReadonlyArray<string>;
  readonly narrationDensity: NarrationDensity;
  readonly weatherLocation: WeatherLocation | null;
}

export interface PreferenceTrend {
  readonly kind: string;
  readonly label: string;
  readonly direction: "up" | "stable" | "down";
  readonly sampleCount: number;
  readonly windowDays: number;
}

export interface ProfileViewResponse {
  readonly profile: UserProfileView;
  readonly preferenceTrends: ReadonlyArray<PreferenceTrend>;
  readonly revision: number;
}

export interface ProfilePatch {
  readonly displayName?: string;
  readonly companionStyle?: "quiet_warm";
  readonly initialPreferences?: ReadonlyArray<string>;
  readonly narrationDensity?: NarrationDensity;
}

export interface UpdateProfileRequest {
  readonly clientRequestId: string;
  readonly expectedRevision: number;
  readonly patch: ProfilePatch;
}

export interface PageRequest {
  readonly cursor: string | null;
  readonly limit: number;
}

export interface SessionSummaryView {
  readonly summaryId: string;
  readonly coveredFrom: string;
  readonly coveredTo: string;
  readonly summary: string;
  readonly generationKind: "llm" | "deterministic";
  readonly revision: number;
}

export interface SessionSummaryPage {
  readonly items: ReadonlyArray<SessionSummaryView>;
  readonly nextCursor: string | null;
}

export interface DeleteSummaryRequest {
  readonly clientRequestId: string;
  readonly summaryId: string;
  readonly expectedRevision: number;
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

export interface WeatherLocationCandidate extends WeatherLocation {
  readonly candidateId: string;
}

export interface SearchWeatherLocationsRequest {
  readonly clientRequestId: string;
  readonly query: string;
  readonly limit: number;
}

export interface SearchWeatherLocationsResponse {
  readonly requestId: string;
  readonly candidates: ReadonlyArray<WeatherLocationCandidate>;
  readonly expiresAt: string;
}

export interface SelectWeatherLocationRequest {
  readonly clientRequestId: string;
  readonly candidateId: string;
  readonly expectedRevision: number;
}

export interface SelectWeatherLocationResponse {
  readonly requestId: string;
  readonly location: WeatherLocation;
  readonly revision: number;
}

export interface ScheduleView {
  readonly rule: ScheduleRule;
  readonly nextOccurrenceAt: string | null;
}

export interface ListSchedulesResponse {
  readonly schedules: ReadonlyArray<ScheduleView>;
  readonly revision: number;
}

export interface UpsertScheduleRequest {
  readonly clientRequestId: string;
  readonly expectedRevision: number;
  readonly schedule: ScheduleRule;
}

export interface UpsertScheduleResponse {
  readonly requestId: string;
  readonly schedule: ScheduleView;
  readonly revision: number;
}

export interface DeleteScheduleRequest {
  readonly clientRequestId: string;
  readonly scheduleId: string;
  readonly expectedRevision: number;
}

export interface NotificationActionRequest {
  readonly clientRequestId: string;
  readonly scheduleId: string;
  readonly occurrenceId: string;
  readonly action: "open" | "dismiss" | "start" | "snooze";
  readonly snoozeMinutes: 10 | 30 | 60 | null;
}

export interface NotificationActionResponse {
  readonly requestId: string;
  readonly occurrenceId: string;
  readonly status: "awaiting_user" | "snoozed" | "starting" | "dismissed";
  readonly nextNotificationAt: string | null;
  readonly revision: number;
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

export type DataDeletionCategory = "profile_and_memories" | "conversations_and_summaries"
  | "playback_history" | "metadata_cache" | "library_index";

export type DataInventoryCategory = "credentials" | "profile_and_preferences"
  | "weather_location_and_cache" | "library_roots_and_identity" | "embedded_music_tags"
  | "metadata_matches" | "artwork_and_tts_cache" | "system_media_runtime"
  | "playback_history_and_feedback" | "chat_messages" | "voice_segment_text"
  | "session_summaries" | "memory_proposals" | "approved_memories_and_revisions"
  | "schedules_and_notifications" | "provider_usage_facts" | "operation_outbox"
  | "diagnostic_logs" | "migration_backups";

export type DataStorageClass = "memory" | "credential_manager" | "sqlite" | "app_data"
  | "app_cache" | "windows_task" | "windows_notification";

export interface DataCategoryInventory {
  readonly category: DataInventoryCategory;
  readonly itemCount: number;
  readonly storageClasses: ReadonlyArray<DataStorageClass>;
  readonly retentionSummary: string;
  readonly externalRecipients: ReadonlyArray<string>;
  readonly deletionControl: "category" | "credential" | "automatic" | "reset_only";
  readonly deletionCategory: DataDeletionCategory | null;
}

export interface GetDataInventoryResponse {
  readonly generatedAt: string;
  readonly categories: ReadonlyArray<DataCategoryInventory>;
}

export interface PreviewDataDeletionResponse {
  readonly previewToken: string;
  readonly expiresAt: string;
  readonly category: DataDeletionCategory;
  readonly itemCount: number;
  readonly consequences: ReadonlyArray<string>;
}

export interface DeleteDataCategoryRequest {
  readonly clientRequestId: string;
  readonly previewToken: string;
  readonly category: DataDeletionCategory;
  readonly confirmation: "DELETE SELECTED DATA";
}

export interface DeleteDataCategoryResponse {
  readonly requestId: string;
  readonly category: DataDeletionCategory;
  readonly deletedCount: number;
  readonly restartRequired: boolean;
}

export interface DeleteAllUserDataResponse {
  readonly requestId: string;
  readonly restartRequired: true;
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

export interface StartLibraryScanRequest {
  readonly clientRequestId: string;
  readonly rootIds: ReadonlyArray<string>;
}

export interface CancelLibraryScanRequest {
  readonly clientRequestId: string;
  readonly operationId: string;
}

export interface CancelLibraryScanResponse {
  readonly requestId: string;
  readonly operationId: string;
  readonly state: "cancelled" | "already_terminal";
}

export type TrackSort = "title" | "artist" | "album" | "recent";
export type TrackAvailabilityFilter = "playable" | "missing";
export type TrackMatchStatus = "matched" | "unmatched" | "review";

export interface TrackFilters {
  readonly availability: TrackAvailabilityFilter | null;
  readonly matchStatus: TrackMatchStatus | null;
}

export interface ListTracksRequest {
  readonly cursor: string | null;
  readonly limit: number;
  readonly query: string | null;
  readonly sort: TrackSort;
  readonly filters: TrackFilters;
}

export interface TrackTagView {
  readonly title: string | null;
  readonly artist: string | null;
  readonly album: string | null;
}

export interface EnrichedTrackTagView extends TrackTagView {
  readonly provider: "musicbrainz";
  readonly confidence: number;
  readonly fetchedAt: string;
}

export interface TrackView {
  readonly trackId: string;
  readonly availability: "playable" | "missing" | "corrupt" | "unsupported";
  readonly durationMs: number;
  readonly artworkAvailable: boolean;
  readonly original: TrackTagView;
  readonly enriched: EnrichedTrackTagView | null;
  readonly matchStatus: TrackMatchStatus;
}

export interface TracksPage {
  readonly items: ReadonlyArray<TrackView>;
  readonly nextCursor: string | null;
}

export interface LibraryScanEvent extends EventEnvelope {
  readonly schemaVersion: typeof IPC_SCHEMA_VERSION;
  readonly operationId: string;
  readonly state: "running" | "completed" | "cancelled" | "failed";
  readonly scanned: number;
  readonly discovered: number;
  readonly failed: number;
  readonly safeMessage: string | null;
}

export interface ProgramStateEvent extends EventEnvelope {
  readonly schemaVersion: typeof IPC_SCHEMA_VERSION;
  readonly programId: string;
  readonly state: "planning" | "running" | "paused" | "stopping" | "completed" | "failed";
  readonly safeMessage: string | null;
}

export interface ProgramSegmentEvent extends EventEnvelope {
  readonly schemaVersion: typeof IPC_SCHEMA_VERSION;
  readonly programId: string;
  readonly segmentId: string;
  readonly state: "queued" | "playing" | "completed" | "skipped" | "failed";
}

export interface ChatMessageEvent extends EventEnvelope {
  readonly operationId: string;
  readonly programId: string;
  readonly role: "user" | "assistant";
  readonly text: string;
  readonly final: boolean;
}

export interface MemoryProposedEvent extends EventEnvelope {
  readonly memory: MemoryRecord;
}

export interface ScheduleDueEvent extends EventEnvelope {
  readonly schemaVersion: typeof IPC_SCHEMA_VERSION;
  readonly scheduleId: string;
  readonly occurrenceId: string;
  readonly notificationShown: boolean;
}

export interface OperationCancelledEvent extends EventEnvelope {
  readonly operationId: string;
  readonly kind: "chat" | "voice_preview" | "library_scan" | "data_export";
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
