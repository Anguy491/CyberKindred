import { subscribeToPublicEvents, type EventSubscriptionHandlers } from "./events";
import { tauriIpcTransport, type IpcTransport, type IpcUnlisten } from "./transport";
import type { ApiError, AppCapabilities } from "./types";
import type {
  Ack,
  CancelLibraryScanRequest,
  CancelLibraryScanResponse,
  DeleteSecretRequest,
  DeleteSecretResponse,
  DeleteScheduleRequest,
  LibraryRootsResponse,
  MusicSourcesResponse,
  ListTracksRequest,
  ListSchedulesResponse,
  OnboardingState,
  OperationAccepted,
  NotificationActionRequest,
  NotificationActionResponse,
  PickLibraryRootResponse,
  SaveOnboardingStepRequest,
  SettingsView,
  SearchWeatherLocationsRequest,
  SearchWeatherLocationsResponse,
  SelectWeatherLocationRequest,
  SelectWeatherLocationResponse,
  SelectMusicSourceRequest,
  SelectMusicSourceResponse,
  PlaybackControlRequest,
  PlaybackState,
  SeekPlaybackRequest,
  StartProgramRequest,
  StartProgramResponse,
  StopProgramRequest,
  CancelOperationRequest,
  CancelOperationResponse,
  DeleteSummaryRequest,
  ListMemoriesRequest,
  MemoryMutationRequest,
  MemoryPage,
  MemoryRecord,
  PageRequest,
  ProfileViewResponse,
  RejectMemoryResponse,
  SessionSummaryPage,
  SubmitChatRequest,
  SubmitFeedbackRequest,
  UpdateMemoryRequest,
  UpsertScheduleRequest,
  UpsertScheduleResponse,
  UpdateProfileRequest,
  StartLibraryScanRequest,
  TestProviderRequest,
  TestProviderResponse,
  TracksPage,
  UpdateSettingsRequest,
  ValidateSecretRequest,
  ValidateSecretResponse,
  VoicesResponse,
} from "./types";
import {
  normalizeApiError,
  parseAck,
  parseAppCapabilities,
  parseCancelLibraryScanResponse,
  parseDeleteSecretResponse,
  parseLibraryRootsResponse,
  parseListSchedulesResponse,
  parseMusicSourcesResponse,
  parseTracksPage,
  parseOnboardingState,
  parseOperationAccepted,
  parseNotificationActionResponse,
  parsePickLibraryRootResponse,
  parseSettingsView,
  parseSearchWeatherLocationsResponse,
  parseSelectWeatherLocationResponse,
  parseSelectMusicSourceResponse,
  parsePlaybackState,
  parseStartProgramResponse,
  parseCancelOperationResponse,
  parseMemoryPage,
  parseMemoryRecord,
  parseProfileViewResponse,
  parseRejectMemoryResponse,
  parseSessionSummaryPage,
  parseTestProviderResponse,
  parseUpsertScheduleResponse,
  parseValidateSecretResponse,
  parseVoicesResponse,
} from "./validation";

const API_V1_GET_CAPABILITIES = "api_v1_get_capabilities";
const CAPABILITIES_TIMEOUT_MS = 2_000;
const READ_FAST_TIMEOUT_MS = 2_000;
const STANDARD_TIMEOUT_MS = 5_000;
const PROVIDER_TIMEOUT_MS = 60_000;
const OPERATION_ACCEPT_TIMEOUT_MS = 2_000;

/** Safe failure thrown by every typed client command. */
export class IpcInvocationError extends Error {
  readonly apiError: ApiError;

  constructor(apiError: ApiError) {
    super(apiError.safeMessage);
    this.name = "IpcInvocationError";
    this.apiError = apiError;
  }
}

/** The only IPC client product modules should receive. */
export class CyberKindredIpcClient {
  readonly #transport: IpcTransport;

  constructor(transport: IpcTransport = tauriIpcTransport) {
    this.#transport = transport;
  }

  /** API-001: read the authoritative feature/source/provider capabilities. */
  async getCapabilities(): Promise<AppCapabilities> {
    return this.#invokeValidated(
      API_V1_GET_CAPABILITIES,
      { request: {} },
      CAPABILITIES_TIMEOUT_MS,
      parseAppCapabilities,
    );
  }

  /** API-002: reads the only authoritative onboarding completion gate. */
  async getOnboardingState(): Promise<OnboardingState> {
    return this.#invokeValidated(
      "api_v1_get_onboarding_state", { request: {} }, READ_FAST_TIMEOUT_MS, parseOnboardingState,
    );
  }

  /** API-003: persists exactly one current or previously completed step. */
  async saveOnboardingStep(request: SaveOnboardingStepRequest): Promise<Ack> {
    return this.#invokeValidated(
      "api_v1_save_onboarding_step", { request }, STANDARD_TIMEOUT_MS, parseAck,
    );
  }

  /** API-004: validates and stores a credential only after the explicit click path. */
  async validateAndSetSecret(request: ValidateSecretRequest): Promise<ValidateSecretResponse> {
    return this.#invokeValidated(
      "api_v1_validate_and_set_secret", { request }, PROVIDER_TIMEOUT_MS, parseValidateSecretResponse,
    );
  }

  /** API-005: deletes only the explicitly identified origin credential. */
  async deleteSecret(request: DeleteSecretRequest): Promise<DeleteSecretResponse> {
    return this.#invokeValidated(
      "api_v1_delete_secret", { request }, STANDARD_TIMEOUT_MS, parseDeleteSecretResponse,
    );
  }

  /** API-006: performs one explicit provider test. */
  async testProvider(request: TestProviderRequest): Promise<TestProviderResponse> {
    return this.#invokeValidated(
      "api_v1_test_provider", { request }, PROVIDER_TIMEOUT_MS, parseTestProviderResponse,
    );
  }

  /** API-007: reads the exact non-secret settings view. */
  async getSettings(): Promise<SettingsView> {
    return this.#invokeValidated(
      "api_v1_get_settings", { request: {} }, READ_FAST_TIMEOUT_MS, parseSettingsView,
    );
  }

  /** API-008: selects the 60-second caller path only for an actual model change. */
  async updateSettings(request: UpdateSettingsRequest, currentModelId: string): Promise<Ack> {
    const requestedModel = request.patch.llmModelId;
    const timeout = requestedModel !== undefined && requestedModel !== currentModelId
      ? PROVIDER_TIMEOUT_MS
      : STANDARD_TIMEOUT_MS;
    return this.#invokeValidated("api_v1_update_settings", { request }, timeout, parseAck);
  }

  /** API-048: sends a city query only after an explicit user search action. */
  async searchWeatherLocations(
    request: SearchWeatherLocationsRequest,
  ): Promise<SearchWeatherLocationsResponse> {
    return this.#invokeValidated(
      "api_v1_search_weather_locations", { request }, 10_000,
      parseSearchWeatherLocationsResponse,
    );
  }

  /** API-049: persists only a live Rust-signed search candidate. */
  async selectWeatherLocation(
    request: SelectWeatherLocationRequest,
  ): Promise<SelectWeatherLocationResponse> {
    return this.#invokeValidated(
      "api_v1_select_weather_location", { request }, STANDARD_TIMEOUT_MS,
      parseSelectWeatherLocationResponse,
    );
  }

  /** API-032: reads the authoritative persistent weekly schedule collection. */
  async listSchedules(): Promise<ListSchedulesResponse> {
    return this.#invokeValidated(
      "api_v1_list_schedules", { request: {} }, READ_FAST_TIMEOUT_MS, parseListSchedulesResponse,
    );
  }

  /** API-033: creates or updates one notification-only schedule rule. */
  async upsertSchedule(request: UpsertScheduleRequest): Promise<UpsertScheduleResponse> {
    return this.#invokeValidated(
      "api_v1_upsert_schedule", { request }, STANDARD_TIMEOUT_MS, parseUpsertScheduleResponse,
    );
  }

  /** API-034: deletes one schedule at the advertised collection revision. */
  async deleteSchedule(request: DeleteScheduleRequest): Promise<Ack> {
    return this.#invokeValidated(
      "api_v1_delete_schedule", { request }, STANDARD_TIMEOUT_MS, parseAck,
    );
  }

  /** API-035: records an explicit action for one due notification occurrence. */
  async handleNotificationAction(
    request: NotificationActionRequest,
  ): Promise<NotificationActionResponse> {
    return this.#invokeValidated(
      "api_v1_handle_notification_action", { request }, STANDARD_TIMEOUT_MS,
      parseNotificationActionResponse,
    );
  }

  /** API-009: accepts one explicit voice-preview operation. */
  async previewVoice(clientRequestId: string, voiceId: string): Promise<OperationAccepted> {
    return this.#invokeValidated(
      "api_v1_preview_voice", { request: { clientRequestId, voiceId } },
      OPERATION_ACCEPT_TIMEOUT_MS, parseOperationAccepted,
    );
  }

  /** API-010: lists path-free authorized library roots. */
  async listLibraryRoots(): Promise<LibraryRootsResponse> {
    return this.#invokeValidated(
      "api_v1_list_library_roots", { request: {} }, READ_FAST_TIMEOUT_MS, parseLibraryRootsResponse,
    );
  }

  /** API-011: opens the user-controlled native picker; cancellation is a null root. */
  async pickAndAddLibraryRoot(clientRequestId: string): Promise<PickLibraryRootResponse> {
    return this.#invokeValidated(
      "api_v1_pick_and_add_library_root", { request: { clientRequestId } },
      undefined, parsePickLibraryRootResponse,
    );
  }

  /** API-012: removes an authorized root record without touching music files. */
  async removeLibraryRoot(
    clientRequestId: string,
    rootId: string,
    expectedRevision: number,
  ): Promise<Ack> {
    return this.#invokeValidated(
      "api_v1_remove_library_root", { request: { clientRequestId, rootId, expectedRevision } },
      STANDARD_TIMEOUT_MS, parseAck,
    );
  }

  /** API-013: accepts a scan over only persisted authorized roots. */
  async startLibraryScan(request: StartLibraryScanRequest): Promise<OperationAccepted> {
    return this.#invokeValidated(
      "api_v1_start_library_scan", { request }, OPERATION_ACCEPT_TIMEOUT_MS, parseOperationAccepted,
    );
  }

  /** API-014: idempotently cancels one accepted scan operation. */
  async cancelLibraryScan(request: CancelLibraryScanRequest): Promise<CancelLibraryScanResponse> {
    return this.#invokeValidated(
      "api_v1_cancel_library_scan", { request }, OPERATION_ACCEPT_TIMEOUT_MS,
      parseCancelLibraryScanResponse,
    );
  }

  /** API-015: reads one strict, path-free local track page. */
  async listTracks(request: ListTracksRequest): Promise<TracksPage> {
    return this.#invokeValidated(
      "api_v1_list_tracks", { request }, STANDARD_TIMEOUT_MS, parseTracksPage,
    );
  }

  /** API-016: lists the authoritative source catalog and capabilities. */
  async listMusicSources(): Promise<MusicSourcesResponse> {
    return this.#invokeValidated(
      "api_v1_list_music_sources", { request: {} }, READ_FAST_TIMEOUT_MS, parseMusicSourcesResponse,
    );
  }

  /** API-017: selects one source without starting audio. */
  async selectMusicSource(request: SelectMusicSourceRequest): Promise<SelectMusicSourceResponse> {
    return this.#invokeValidated(
      "api_v1_select_music_source", { request }, STANDARD_TIMEOUT_MS, parseSelectMusicSourceResponse,
    );
  }

  /** API-018: reads the authoritative playback snapshot. */
  async getPlaybackState(): Promise<PlaybackState> {
    return this.#invokeValidated(
      "api_v1_get_playback_state", { request: {} }, READ_FAST_TIMEOUT_MS, parsePlaybackState,
    );
  }

  /** API-019: resumes playback from the advertised revision. */
  async play(request: PlaybackControlRequest): Promise<PlaybackState> {
    return this.#playbackCommand("api_v1_play", request);
  }

  /** API-020: pauses playback from the advertised revision. */
  async pause(request: PlaybackControlRequest): Promise<PlaybackState> {
    return this.#playbackCommand("api_v1_pause", request);
  }

  /** API-021: seeks to an absolute bounded position. */
  async seek(request: SeekPlaybackRequest): Promise<PlaybackState> {
    return this.#playbackCommand("api_v1_seek", request);
  }

  /** API-022: advances from the advertised revision. */
  async next(request: PlaybackControlRequest): Promise<PlaybackState> {
    return this.#playbackCommand("api_v1_next", request);
  }

  /** API-023: returns to the previous item from the advertised revision. */
  async previous(request: PlaybackControlRequest): Promise<PlaybackState> {
    return this.#playbackCommand("api_v1_previous", request);
  }

  /** API-024: starts only from a manual click or confirmed notification. */
  async startProgram(request: StartProgramRequest): Promise<StartProgramResponse> {
    return this.#invokeValidated(
      "api_v1_start_program", { request }, PROVIDER_TIMEOUT_MS, parseStartProgramResponse,
    );
  }

  /** API-025: idempotently stops one active program. */
  async stopProgram(request: StopProgramRequest): Promise<Ack> {
    return this.#invokeValidated(
      "api_v1_stop_program", { request }, STANDARD_TIMEOUT_MS, parseAck,
    );
  }

  /** API-026: accepts one explicit text request for the active program. */
  async submitChat(request: SubmitChatRequest): Promise<OperationAccepted> {
    return this.#invokeValidated(
      "api_v1_submit_chat", { request }, OPERATION_ACCEPT_TIMEOUT_MS, parseOperationAccepted,
    );
  }

  /** API-027: persists explicit feedback before acknowledging it. */
  async submitFeedback(request: SubmitFeedbackRequest): Promise<Ack> {
    return this.#invokeValidated(
      "api_v1_submit_feedback", { request }, STANDARD_TIMEOUT_MS, parseAck,
    );
  }

  async listMemories(request: ListMemoriesRequest): Promise<MemoryPage> {
    return this.#invokeValidated(
      "api_v1_list_memories", { request }, STANDARD_TIMEOUT_MS, parseMemoryPage,
    );
  }

  async approveMemory(request: MemoryMutationRequest): Promise<MemoryRecord> {
    return this.#invokeValidated(
      "api_v1_approve_memory", { request }, STANDARD_TIMEOUT_MS, parseMemoryRecord,
    );
  }

  async updateMemory(request: UpdateMemoryRequest): Promise<MemoryRecord> {
    return this.#invokeValidated(
      "api_v1_update_memory", { request }, STANDARD_TIMEOUT_MS, parseMemoryRecord,
    );
  }

  async deleteMemory(request: MemoryMutationRequest): Promise<Ack> {
    return this.#invokeValidated(
      "api_v1_delete_memory", { request }, STANDARD_TIMEOUT_MS, parseAck,
    );
  }

  async rejectMemoryProposal(request: MemoryMutationRequest): Promise<RejectMemoryResponse> {
    return this.#invokeValidated(
      "api_v1_reject_memory_proposal", { request }, STANDARD_TIMEOUT_MS, parseRejectMemoryResponse,
    );
  }

  async getProfileView(): Promise<ProfileViewResponse> {
    return this.#invokeValidated(
      "api_v1_get_profile_view", { request: {} }, STANDARD_TIMEOUT_MS, parseProfileViewResponse,
    );
  }

  async updateProfile(request: UpdateProfileRequest): Promise<Ack> {
    return this.#invokeValidated(
      "api_v1_update_profile", { request }, STANDARD_TIMEOUT_MS, parseAck,
    );
  }

  async listSessionSummaries(request: PageRequest): Promise<SessionSummaryPage> {
    return this.#invokeValidated(
      "api_v1_list_session_summaries", { request }, STANDARD_TIMEOUT_MS, parseSessionSummaryPage,
    );
  }

  async deleteSessionSummary(request: DeleteSummaryRequest): Promise<Ack> {
    return this.#invokeValidated(
      "api_v1_delete_session_summary", { request }, STANDARD_TIMEOUT_MS, parseAck,
    );
  }

  /** API-038: cancels one accepted operation by its declared kind. */
  async cancelOperation(request: CancelOperationRequest): Promise<CancelOperationResponse> {
    return this.#invokeValidated(
      "api_v1_cancel_operation", { request }, OPERATION_ACCEPT_TIMEOUT_MS,
      parseCancelOperationResponse,
    );
  }

  /** API-043: reads the local path-free voice catalog. */
  async listVoices(): Promise<VoicesResponse> {
    return this.#invokeValidated(
      "api_v1_list_voices", { request: { provider: "tts" } }, STANDARD_TIMEOUT_MS, parseVoicesResponse,
    );
  }

  /** Starts the process-global v1 event subscription and snapshot reconciliation. */
  async subscribeToEvents(handlers: EventSubscriptionHandlers): Promise<IpcUnlisten> {
    try {
      return await subscribeToPublicEvents(this.#transport, handlers);
    } catch (error) {
      throw new IpcInvocationError(normalizeApiError(error));
    }
  }

  async #invoke<Response>(
    command: string,
    request: Readonly<Record<string, unknown>>,
    timeoutMs: number | undefined,
  ): Promise<Response> {
    let timeoutHandle: ReturnType<typeof setTimeout> | undefined;
    const operation = this.#transport.invoke<Response>(command, request);
    const pending = timeoutMs === undefined ? operation : Promise.race([
      operation,
      new Promise<never>((_resolve, reject) => {
        timeoutHandle = setTimeout(
          () => reject(new IpcInvocationError(normalizeApiError(undefined))), timeoutMs,
        );
      }),
    ]);
    try {
      return await pending;
    } catch (error) {
      if (error instanceof IpcInvocationError) {
        throw error;
      }
      throw new IpcInvocationError(normalizeApiError(error));
    } finally {
      if (timeoutHandle !== undefined) {
        clearTimeout(timeoutHandle);
      }
    }
  }

  async #playbackCommand(
    command: string,
    request: PlaybackControlRequest | SeekPlaybackRequest,
  ): Promise<PlaybackState> {
    return this.#invokeValidated(command, { request }, STANDARD_TIMEOUT_MS, parsePlaybackState);
  }

  async #invokeValidated<Response>(
    command: string,
    request: Readonly<Record<string, unknown>>,
    timeoutMs: number | undefined,
    parse: (value: unknown) => Response,
  ): Promise<Response> {
    const response = await this.#invoke<unknown>(command, request, timeoutMs);
    try {
      return parse(response);
    } catch {
      throw new IpcInvocationError(normalizeApiError(undefined));
    }
  }
}
