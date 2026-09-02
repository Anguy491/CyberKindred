import { subscribeToPublicEvents, type EventSubscriptionHandlers } from "./events";
import { tauriIpcTransport, type IpcTransport, type IpcUnlisten } from "./transport";
import type { ApiError, AppCapabilities } from "./types";
import type {
  Ack,
  DeleteSecretRequest,
  DeleteSecretResponse,
  LibraryRootsResponse,
  OnboardingState,
  OperationAccepted,
  PickLibraryRootResponse,
  SaveOnboardingStepRequest,
  SettingsView,
  TestProviderRequest,
  TestProviderResponse,
  UpdateSettingsRequest,
  ValidateSecretRequest,
  ValidateSecretResponse,
  VoicesResponse,
} from "./types";
import {
  normalizeApiError,
  parseAck,
  parseAppCapabilities,
  parseDeleteSecretResponse,
  parseLibraryRootsResponse,
  parseOnboardingState,
  parseOperationAccepted,
  parsePickLibraryRootResponse,
  parseSettingsView,
  parseTestProviderResponse,
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
