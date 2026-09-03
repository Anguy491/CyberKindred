import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import axe from "axe-core";
import { describe, expect, it, vi } from "vitest";

import { App } from "../../src/App";
import type { SettingsIpc } from "../../src/features/settings";
import type { EventSubscriptionHandlers, ScheduleRule } from "../../src/ipc";

const SCHEDULE_ID = "018f47c0-8b8b-7c35-8bf7-278e15b1a211";
const OCCURRENCE_ID = "018f47c0-8b8b-7c35-8bf7-278e15b1a212";

function fixture(): { ipc: SettingsIpc; handlers: { current?: EventSubscriptionHandlers } } {
  const handlers: { current?: EventSubscriptionHandlers } = {};
  const ipc: SettingsIpc = {
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
    searchWeatherLocations: vi.fn(async () => { throw new Error("not used"); }),
    selectWeatherLocation: vi.fn(async () => { throw new Error("not used"); }),
    listSchedules: vi.fn(async () => ({ schedules: [], revision: 0 })),
    upsertSchedule: vi.fn(async (request) => {
      const saved: ScheduleRule = {
        ...request.schedule, scheduleId: SCHEDULE_ID, revision: 1,
      };
      return {
        requestId: request.clientRequestId,
        schedule: { rule: saved, nextOccurrenceAt: "2026-09-07T21:00:00.000Z" },
        revision: 1,
      };
    }),
    deleteSchedule: vi.fn(async (request) => ({
      requestId: request.clientRequestId, revision: request.expectedRevision + 1,
    })),
    handleNotificationAction: vi.fn(async (request) => ({
      requestId: request.clientRequestId,
      occurrenceId: request.occurrenceId,
      status: request.action === "start" ? "starting" as const
        : request.action === "snooze" ? "snoozed" as const
          : request.action === "dismiss" ? "dismissed" as const : "awaiting_user" as const,
      nextNotificationAt: null,
      revision: 1,
    })),
    startProgram: vi.fn(async (request) => ({
      requestId: request.clientRequestId,
      programId: "018f47c0-8b8b-7c35-8bf7-278e15b1a213",
      plan: null,
    })),
    subscribeToEvents: vi.fn(async (nextHandlers) => {
      handlers.current = nextHandlers;
      await nextHandlers.refreshSnapshot("initial");
      return () => undefined;
    }),
  };
  return { ipc, handlers };
}

describe("[TASK-024] schedule settings", () => {
  it("creates a notification-only rule and starts only after an explicit due action", async () => {
    const { ipc, handlers } = fixture();
    const user = userEvent.setup();
    const { container } = render(<App scenario="ready" initialRoute="settings" settingsIpc={ipc}
      fontStatusLoader={async () => "loaded"} />);
    await user.click(screen.getByRole("button", { name: "SCHEDULE" }));
    expect(await screen.findByText("[NO RULES]")).not.toBeNull();

    await user.click(screen.getByRole("button", { name: "启用系统通知" }));
    expect(ipc.updateSettings).toHaveBeenCalledWith(expect.objectContaining({
      expectedRevision: 2, patch: { notificationsEnabled: true },
    }), "gpt-5.6-luna");

    await user.click(screen.getByRole("button", { name: "创建日程" }));
    await waitFor(() => expect(ipc.upsertSchedule).toHaveBeenCalled());
    const request = vi.mocked(ipc.upsertSchedule).mock.calls[0]?.[0];
    expect(request).toMatchObject({
      expectedRevision: 0,
      schedule: { enabled: true, notificationOnly: true, revision: 0 },
    });
    expect(request?.schedule.daysOfWeek).toEqual(["mon", "tue", "wed", "thu", "fri"]);
    expect(await screen.findByText("Morning radio")).not.toBeNull();
    const accessibility = await axe.run(container, { rules: { "color-contrast": { enabled: false } } });
    expect(accessibility.violations).toEqual([]);

    await act(async () => {
      handlers.current?.onEvent("cyberkindred://v1/schedule/due", {
        schemaVersion: "1.0.0", sequence: 1, occurredAt: "2026-09-07T21:00:00.000Z",
        scheduleId: SCHEDULE_ID, occurrenceId: OCCURRENCE_ID, notificationShown: true,
      });
    });
    const duePanel = await screen.findByRole("region", { name: "到点节目选择" });
    expect(within(duePanel).getByRole("button", { name: "60 分钟后" })).not.toBeNull();
    expect(ipc.startProgram).not.toHaveBeenCalled();

    await user.click(within(duePanel).getByRole("button", { name: "开始节目" }));
    await waitFor(() => expect(ipc.startProgram).toHaveBeenCalledWith(expect.objectContaining({
      sourceId: "local", trigger: "notification",
    })));
    expect(ipc.handleNotificationAction).toHaveBeenCalledWith(expect.objectContaining({
      scheduleId: SCHEDULE_ID, occurrenceId: OCCURRENCE_ID, action: "start", snoozeMinutes: null,
    }));
  });

  it("sends an exact 60-minute snooze without starting playback", async () => {
    const { ipc, handlers } = fixture();
    const user = userEvent.setup();
    render(<App scenario="ready" initialRoute="settings" settingsIpc={ipc}
      fontStatusLoader={async () => "loaded"} />);
    await user.click(screen.getByRole("button", { name: "SCHEDULE" }));
    await waitFor(() => expect(handlers.current).toBeDefined());
    await act(async () => {
      handlers.current?.onEvent("cyberkindred://v1/schedule/due", {
        schemaVersion: "1.0.0", sequence: 1, occurredAt: "2026-09-07T21:00:00.000Z",
        scheduleId: SCHEDULE_ID, occurrenceId: OCCURRENCE_ID, notificationShown: true,
      });
    });
    await user.click(await screen.findByRole("button", { name: "60 分钟后" }));
    expect(ipc.handleNotificationAction).toHaveBeenCalledWith(expect.objectContaining({
      action: "snooze", snoozeMinutes: 60,
    }));
    expect(ipc.startProgram).not.toHaveBeenCalled();
  });
});
