import type {
  Ack,
  MemoryPage,
  MemoryRecord,
  ProfilePatch,
  ProfileViewResponse,
  RejectMemoryResponse,
  SessionSummaryPage,
} from "../../ipc";

export interface YouIpc {
  getProfile(): Promise<ProfileViewResponse>;
  updateProfile(expectedRevision: number, patch: ProfilePatch): Promise<Ack>;
  listMemories(): Promise<MemoryPage>;
  approveMemory(memoryId: string, expectedRevision: number): Promise<MemoryRecord>;
  updateMemory(memoryId: string, expectedRevision: number, content: string, enabled: boolean): Promise<MemoryRecord>;
  rejectMemory(memoryId: string, expectedRevision: number): Promise<RejectMemoryResponse>;
  deleteMemory(memoryId: string, expectedRevision: number): Promise<Ack>;
  listSummaries(): Promise<SessionSummaryPage>;
  deleteSummary(summaryId: string, expectedRevision: number): Promise<Ack>;
}
