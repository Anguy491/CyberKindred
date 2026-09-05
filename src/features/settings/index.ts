import { CyberKindredIpcClient } from "../../ipc";
import type {
  Ack,
  DataDeletionCategory,
  DeleteAllUserDataResponse,
  DeleteDataCategoryRequest,
  DeleteDataCategoryResponse,
  DeleteScheduleRequest,
  EventSubscriptionHandlers,
  GetDataInventoryResponse,
  IpcUnlisten,
  ListSchedulesResponse,
  NotificationActionRequest,
  NotificationActionResponse,
  OperationAccepted,
  PreviewDataDeletionResponse,
  SearchWeatherLocationsRequest,
  SearchWeatherLocationsResponse,
  SelectWeatherLocationRequest,
  SelectWeatherLocationResponse,
  SettingsView,
  StartProgramRequest,
  StartProgramResponse,
  UpsertScheduleRequest,
  UpsertScheduleResponse,
  UpdateSettingsRequest,
} from "../../ipc";

export interface SettingsIpc {
  getSettings(): Promise<SettingsView>;
  updateSettings(request: UpdateSettingsRequest, currentModelId: string): Promise<Ack>;
  searchWeatherLocations(
    request: SearchWeatherLocationsRequest,
  ): Promise<SearchWeatherLocationsResponse>;
  selectWeatherLocation(
    request: SelectWeatherLocationRequest,
  ): Promise<SelectWeatherLocationResponse>;
  listSchedules(): Promise<ListSchedulesResponse>;
  upsertSchedule(request: UpsertScheduleRequest): Promise<UpsertScheduleResponse>;
  deleteSchedule(request: DeleteScheduleRequest): Promise<Ack>;
  handleNotificationAction(
    request: NotificationActionRequest,
  ): Promise<NotificationActionResponse>;
  startProgram(request: StartProgramRequest): Promise<StartProgramResponse>;
  getDataInventory(): Promise<GetDataInventoryResponse>;
  previewDataDeletion(category: DataDeletionCategory): Promise<PreviewDataDeletionResponse>;
  deleteDataCategory(request: DeleteDataCategoryRequest): Promise<DeleteDataCategoryResponse>;
  exportUserData(clientRequestId: string): Promise<OperationAccepted>;
  deleteAllUserData(
    clientRequestId: string,
    confirmation: "DELETE CYBERKINDRED DATA",
  ): Promise<DeleteAllUserDataResponse>;
  subscribeToEvents(handlers: EventSubscriptionHandlers): Promise<IpcUnlisten>;
}

export function createSettingsIpc(): SettingsIpc {
  return new CyberKindredIpcClient();
}

export const BROWSER_SETTINGS_IPC: SettingsIpc = {
  async getSettings() {
    return {
      providerOrigin: "https://api.openai.com",
      llmModelId: "gpt-5.6-luna",
      ttsModelId: "gpt-4o-mini-tts",
      ttsVoiceId: "alloy",
      metadataEnabled: false,
      weatherEnabled: false,
      defaultSourceId: null,
      narrationDensity: "balanced",
      ttsEnabled: false,
      audioOutputDeviceId: null,
      audioOutputBehavior: "follow_system_default",
      minimizeToTray: false,
      launchAtStartup: false,
      notificationsEnabled: false,
      weatherLocation: null,
      secretStatus: { origins: [] },
      integrationStatuses: [],
      revision: 0,
    };
  },
  async updateSettings(request) {
    return { requestId: request.clientRequestId, revision: request.expectedRevision + 1 };
  },
  async searchWeatherLocations(request) {
    return { requestId: request.clientRequestId, candidates: [], expiresAt: new Date().toISOString() };
  },
  async selectWeatherLocation(request) {
    throw new Error(`Unknown browser candidate ${request.candidateId}`);
  },
  async listSchedules() {
    return { schedules: [], revision: 0 };
  },
  async upsertSchedule(request) {
    const now = new Date().toISOString();
    return {
      requestId: request.clientRequestId,
      schedule: {
        rule: { ...request.schedule, scheduleId: crypto.randomUUID(), createdAt: now, updatedAt: now, revision: 1 },
        nextOccurrenceAt: null,
      },
      revision: request.expectedRevision + 1,
    };
  },
  async deleteSchedule(request) {
    return { requestId: request.clientRequestId, revision: request.expectedRevision + 1 };
  },
  async handleNotificationAction(request) {
    return {
      requestId: request.clientRequestId,
      occurrenceId: request.occurrenceId,
      status: request.action === "start" ? "starting"
        : request.action === "snooze" ? "snoozed"
          : request.action === "dismiss" ? "dismissed" : "awaiting_user",
      nextNotificationAt: null,
      revision: 0,
    };
  },
  async startProgram(request) {
    return { requestId: request.clientRequestId, programId: crypto.randomUUID(), plan: null };
  },
  async getDataInventory() {
    return { generatedAt: new Date().toISOString(), categories: [] };
  },
  async previewDataDeletion(category) {
    return {
      previewToken: crypto.randomUUID(), expiresAt: new Date(Date.now() + 300_000).toISOString(),
      category, itemCount: 0, consequences: ["没有可删除的项目。"],
    };
  },
  async deleteDataCategory(request) {
    return {
      requestId: request.clientRequestId, category: request.category,
      deletedCount: 0, restartRequired: false,
    };
  },
  async exportUserData() {
    return { operationId: crypto.randomUUID(), acceptedAt: new Date().toISOString() };
  },
  async deleteAllUserData(clientRequestId) {
    return { requestId: clientRequestId, restartRequired: true };
  },
  async subscribeToEvents(handlers) {
    await handlers.refreshSnapshot("initial");
    return () => undefined;
  },
};
