import { CyberKindredIpcClient } from "../../ipc";
import type { YouIpc } from "./types";

type YouClient = Pick<CyberKindredIpcClient,
  | "getProfileView" | "updateProfile" | "listMemories" | "approveMemory" | "updateMemory"
  | "rejectMemoryProposal" | "deleteMemory" | "listSessionSummaries" | "deleteSessionSummary"
>;

export function createYouIpc(client: YouClient = new CyberKindredIpcClient()): YouIpc {
  const mutation = (memoryId: string, expectedRevision: number) => ({
    clientRequestId: globalThis.crypto.randomUUID(), memoryId, expectedRevision,
  });
  return {
    getProfile: () => client.getProfileView(),
    updateProfile: (expectedRevision, patch) => client.updateProfile({
      clientRequestId: globalThis.crypto.randomUUID(), expectedRevision, patch,
    }),
    listMemories: () => client.listMemories({ cursor: null, limit: 100, status: null }),
    approveMemory: (memoryId, expectedRevision) => client.approveMemory(mutation(memoryId, expectedRevision)),
    updateMemory: (memoryId, expectedRevision, content, enabled) => client.updateMemory({
      ...mutation(memoryId, expectedRevision), content, enabled,
    }),
    rejectMemory: (memoryId, expectedRevision) => client.rejectMemoryProposal(mutation(memoryId, expectedRevision)),
    deleteMemory: (memoryId, expectedRevision) => client.deleteMemory(mutation(memoryId, expectedRevision)),
    listSummaries: () => client.listSessionSummaries({ cursor: null, limit: 100 }),
    deleteSummary: (summaryId, expectedRevision) => client.deleteSessionSummary({
      clientRequestId: globalThis.crypto.randomUUID(), summaryId, expectedRevision,
    }),
  };
}

export const BROWSER_YOU_IPC: YouIpc = {
  getProfile: async () => ({
    profile: { displayName: "", companionStyle: "quiet_warm", initialPreferences: [], narrationDensity: "balanced", weatherLocation: null },
    preferenceTrends: [], revision: 0,
  }),
  updateProfile: async () => { throw new Error("YOU controls require the desktop runtime."); },
  listMemories: async () => ({ items: [], nextCursor: null }),
  approveMemory: async () => { throw new Error("YOU controls require the desktop runtime."); },
  updateMemory: async () => { throw new Error("YOU controls require the desktop runtime."); },
  rejectMemory: async () => { throw new Error("YOU controls require the desktop runtime."); },
  deleteMemory: async () => { throw new Error("YOU controls require the desktop runtime."); },
  listSummaries: async () => ({ items: [], nextCursor: null }),
  deleteSummary: async () => { throw new Error("YOU controls require the desktop runtime."); },
};
