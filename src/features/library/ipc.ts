import {
  CyberKindredIpcClient,
  type LibraryScanEvent,
} from "../../ipc";
import type { LibraryIpc } from "./types";

type LibraryClient = Pick<CyberKindredIpcClient,
  | "listLibraryRoots"
  | "pickAndAddLibraryRoot"
  | "listTracks"
  | "startLibraryScan"
  | "cancelLibraryScan"
  | "subscribeToEvents"
>;

/** Adapts the strict process-global IPC/event consumer to the library feature. */
export function createLibraryIpc(
  client: LibraryClient = new CyberKindredIpcClient(),
): LibraryIpc {
  return {
    listLibraryRoots: () => client.listLibraryRoots(),
    pickAndAddLibraryRoot: () => client.pickAndAddLibraryRoot(createClientRequestId()),
    listTracks: (request) => client.listTracks(request),
    startLibraryScan: (rootIds) => client.startLibraryScan({
      clientRequestId: createClientRequestId(),
      rootIds,
    }),
    cancelLibraryScan: (operationId) => client.cancelLibraryScan({
      clientRequestId: createClientRequestId(),
      operationId,
    }),
    subscribeLibraryScan: (handler, refreshSnapshot) => client.subscribeToEvents({
      onEvent: (eventName, payload) => {
        if (eventName === "cyberkindred://v1/library/scan") {
          handler(payload as unknown as LibraryScanEvent);
        }
      },
      refreshSnapshot,
    }),
  };
}

function createClientRequestId(): string {
  return globalThis.crypto.randomUUID();
}

/** Silent, offline browser fixture used only when no Tauri runtime exists. */
export const BROWSER_LIBRARY_IPC: LibraryIpc = {
  listLibraryRoots: async () => ({ roots: [], revision: 0 }),
  pickAndAddLibraryRoot: async () => ({
    requestId: createClientRequestId(),
    root: null,
    revision: 0,
  }),
  listTracks: async () => ({ items: [], nextCursor: null }),
  startLibraryScan: async () => {
    throw new Error("Library scanning requires the desktop runtime.");
  },
  cancelLibraryScan: async () => {
    throw new Error("Library scanning requires the desktop runtime.");
  },
  subscribeLibraryScan: async (_handler, refreshSnapshot) => {
    await refreshSnapshot();
    return () => undefined;
  },
};
