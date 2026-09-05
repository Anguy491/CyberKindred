import { CyberKindredIpcClient } from "../../ipc";
import type {
  Ack,
  DataDeletionCategory,
  DeleteAllUserDataResponse,
  DeleteDataCategoryRequest,
  DeleteDataCategoryResponse,
  DeleteScheduleRequest,
  DeleteSecretRequest,
  DeleteSecretResponse,
  EventSubscriptionHandlers,
  GetDataInventoryResponse,
  IpcUnlisten,
  ListSchedulesResponse,
  MusicSourcesResponse,
  NotificationActionRequest,
  NotificationActionResponse,
  OperationAccepted,
  PreviewDataDeletionResponse,
  TestProviderRequest,
  TestProviderResponse,
  SearchWeatherLocationsRequest,
  SearchWeatherLocationsResponse,
  SelectMusicSourceRequest,
  SelectMusicSourceResponse,
  SelectWeatherLocationRequest,
  SelectWeatherLocationResponse,
  SettingsView,
  StartProgramRequest,
  StartProgramResponse,
  UpsertScheduleRequest,
  UpsertScheduleResponse,
  UpdateSettingsRequest,
  ValidateSecretRequest,
  ValidateSecretResponse,
  VoicesResponse,
} from "../../ipc";

export interface SettingsIpc {
  getSettings(): Promise<SettingsView>;
  validateAndSetSecret(request: ValidateSecretRequest): Promise<ValidateSecretResponse>;
  deleteSecret(request: DeleteSecretRequest): Promise<DeleteSecretResponse>;
  testProvider(request: TestProviderRequest): Promise<TestProviderResponse>;
  listVoices(): Promise<VoicesResponse>;
  listMusicSources(): Promise<MusicSourcesResponse>;
  selectMusicSource(request: SelectMusicSourceRequest): Promise<SelectMusicSourceResponse>;
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
  async validateAndSetSecret(request) {
    return { requestId: request.clientRequestId, configured: true, verifiedAt: new Date().toISOString() };
  },
  async deleteSecret(request) {
    return { requestId: request.clientRequestId, configured: false };
  },
  async testProvider(request) {
    return { requestId: request.clientRequestId, ok: true, latencyMs: 12, safeMessage: "连接测试成功。" };
  },
  async listVoices() {
    return { voices: [{ voiceId: "alloy", displayName: "Alloy", previewAvailable: true }] };
  },
  async listMusicSources() {
    return {
      sources: [
        { sourceId: "local", kind: "local", displayName: "本地曲库", connected: true,
          capabilities: { play: true, pause: true, seek: true, next: true, previous: true, setQueue: true } },
        { sourceId: "apple_music", kind: "system_session", displayName: "Apple Music Windows App",
          connected: false, capabilities: { play: false, pause: false, seek: false, next: false, previous: false, setQueue: false } },
      ],
    };
  },
  async selectMusicSource() {
    throw new Error("Apple Music connection requires the desktop runtime.");
  },
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
