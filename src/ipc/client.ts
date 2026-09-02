import { subscribeToPublicEvents, type EventSubscriptionHandlers } from "./events";
import { tauriIpcTransport, type IpcTransport, type IpcUnlisten } from "./transport";
import type { ApiError, AppCapabilities } from "./types";
import { normalizeApiError, parseAppCapabilities } from "./validation";

const API_V1_GET_CAPABILITIES = "api_v1_get_capabilities";
const CAPABILITIES_TIMEOUT_MS = 2_000;

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
    const response = await this.#invoke<unknown>(
      API_V1_GET_CAPABILITIES,
      {},
      CAPABILITIES_TIMEOUT_MS,
    );
    try {
      return parseAppCapabilities(response);
    } catch {
      throw new IpcInvocationError(normalizeApiError(undefined));
    }
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
    timeoutMs: number,
  ): Promise<Response> {
    let timeoutHandle: ReturnType<typeof setTimeout> | undefined;
    const timeout = new Promise<never>((_resolve, reject) => {
      timeoutHandle = setTimeout(() => reject(new IpcInvocationError(normalizeApiError(undefined))), timeoutMs);
    });
    try {
      return await Promise.race([this.#transport.invoke<Response>(command, request), timeout]);
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
}
