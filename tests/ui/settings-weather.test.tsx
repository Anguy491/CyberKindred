import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";

import { App } from "../../src/App";
import type { SettingsIpc } from "../../src/features/settings";

const CANDIDATE_ID = "018f47c0-8b8b-7c35-8bf7-278e15b1a211";
const location = {
  city: "Sydney", region: "New South Wales", country: "Australia", countryCode: "AU",
  latitude: -33.8688, longitude: 151.2093, timezone: "Australia/Sydney",
};

function fixture(): SettingsIpc {
  return {
    getSettings: vi.fn(async () => ({
      providerOrigin: "https://api.openai.com", llmModelId: "gpt-5.6-luna",
      ttsModelId: "gpt-4o-mini-tts", ttsVoiceId: "alloy", metadataEnabled: false,
      weatherEnabled: false, defaultSourceId: null, narrationDensity: "balanced",
      ttsEnabled: false, audioOutputDeviceId: null, audioOutputBehavior: "follow_system_default",
      minimizeToTray: false, launchAtStartup: false, notificationsEnabled: false,
      weatherLocation: null, secretStatus: { origins: [] }, integrationStatuses: [], revision: 2,
    })),
    updateSettings: vi.fn(async (request) => ({
      requestId: request.clientRequestId, revision: request.expectedRevision + 1,
    })),
    searchWeatherLocations: vi.fn(async (request) => ({
      requestId: request.clientRequestId,
      candidates: [{ candidateId: CANDIDATE_ID, ...location }],
      expiresAt: "2026-09-03T03:10:00.000Z",
    })),
    selectWeatherLocation: vi.fn(async (request) => ({
      requestId: request.clientRequestId, location, revision: 3,
    })),
    listSchedules: vi.fn(async () => ({ schedules: [], revision: 0 })),
    upsertSchedule: vi.fn(async () => { throw new Error("not used"); }),
    deleteSchedule: vi.fn(async () => { throw new Error("not used"); }),
    handleNotificationAction: vi.fn(async () => { throw new Error("not used"); }),
    startProgram: vi.fn(async () => { throw new Error("not used"); }),
    subscribeToEvents: vi.fn(async (handlers) => {
      await handlers.refreshSnapshot("initial");
      return () => undefined;
    }),
  };
}

describe("[TASK-023] Settings weather context", () => {
  it("does not search while typing and saves only an explicitly selected candidate", async () => {
    const ipc = fixture();
    const user = userEvent.setup();
    render(<App
      scenario="ready"
      initialRoute="settings"
      settingsIpc={ipc}
      fontStatusLoader={async () => "loaded"}
    />);
    await user.click(screen.getByRole("button", { name: "CONTEXT" }));
    const input = await screen.findByRole("textbox", { name: "手动搜索城市" });
    await user.type(input, "Sydney");
    expect(ipc.searchWeatherLocations).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "搜索城市" }));
    expect(await screen.findByRole("button", { name: /Sydney/u })).not.toBeNull();
    expect(ipc.searchWeatherLocations).toHaveBeenCalledWith(expect.objectContaining({
      query: "Sydney", limit: 8,
    }));

    await user.click(screen.getByRole("button", { name: /Sydney/u }));
    await waitFor(() => expect(ipc.selectWeatherLocation).toHaveBeenCalledWith(
      expect.objectContaining({ candidateId: CANDIDATE_ID, expectedRevision: 2 }),
    ));
    expect(ipc.updateSettings).toHaveBeenCalledWith(
      expect.objectContaining({ expectedRevision: 3, patch: { weatherEnabled: true } }),
      "gpt-5.6-luna",
    );
    expect(await screen.findByText("[SAVED · WEATHER CONTEXT ON]")).not.toBeNull();
  });
});
