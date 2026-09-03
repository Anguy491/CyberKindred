import { CyberKindredIpcClient } from "../../ipc";
import type {
  Ack,
  SearchWeatherLocationsRequest,
  SearchWeatherLocationsResponse,
  SelectWeatherLocationRequest,
  SelectWeatherLocationResponse,
  SettingsView,
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
};
