import { describe, expect, it } from "vitest";

import { CyberKindredIpcClient, IpcInvocationError, type IpcTransport } from "../../src/ipc";

const REQUEST_ID = "018f47c0-8b8b-7c35-8bf7-278e15b1a210";
const SCHEDULE_ID = "018f47c0-8b8b-7c35-8bf7-278e15b1a211";
const OCCURRENCE_ID = "018f47c0-8b8b-7c35-8bf7-278e15b1a212";
const rule = {
  schemaVersion: "1.0.0" as const,
  scheduleId: SCHEDULE_ID,
  name: "Morning radio",
  timezone: "Australia/Sydney",
  daysOfWeek: ["mon", "wed", "fri"] as const,
  localTime: "07:00",
  enabled: true,
  notificationOnly: true as const,
  createdAt: "2026-09-03T03:00:00.000Z",
  updatedAt: "2026-09-03T03:00:00.000Z",
  revision: 1,
};

class Transport implements IpcTransport {
  readonly calls: Array<{ command: string; args: Readonly<Record<string, unknown>> }> = [];
  response: unknown;
  async invoke<Response>(command: string, args: Readonly<Record<string, unknown>>): Promise<Response> {
    this.calls.push({ command, args });
    return this.response as Response;
  }
  async listen(): Promise<() => void> { return () => undefined; }
}

describe("[TASK-024] schedule IPC", () => {
  it("uses exact API-032..035 envelopes and validates their responses", async () => {
    const transport = new Transport();
    const client = new CyberKindredIpcClient(transport);
    const view = { rule, nextOccurrenceAt: "2026-09-07T21:00:00.000Z" };

    transport.response = { schedules: [view], revision: 4 };
    await expect(client.listSchedules()).resolves.toEqual({ schedules: [view], revision: 4 });
    expect(transport.calls.at(-1)).toEqual({
      command: "api_v1_list_schedules", args: { request: {} },
    });

    transport.response = { requestId: REQUEST_ID, schedule: view, revision: 5 };
    await client.upsertSchedule({ clientRequestId: REQUEST_ID, expectedRevision: 4, schedule: rule });
    expect(transport.calls.at(-1)).toEqual({
      command: "api_v1_upsert_schedule",
      args: { request: { clientRequestId: REQUEST_ID, expectedRevision: 4, schedule: rule } },
    });

    transport.response = { requestId: REQUEST_ID, revision: 6 };
    await client.deleteSchedule({
      clientRequestId: REQUEST_ID, scheduleId: SCHEDULE_ID, expectedRevision: 5,
    });
    expect(transport.calls.at(-1)?.command).toBe("api_v1_delete_schedule");

    transport.response = {
      requestId: REQUEST_ID, occurrenceId: OCCURRENCE_ID, status: "snoozed",
      nextNotificationAt: "2026-09-07T21:30:00.000Z", revision: 6,
    };
    await client.handleNotificationAction({
      clientRequestId: REQUEST_ID, scheduleId: SCHEDULE_ID, occurrenceId: OCCURRENCE_ID,
      action: "snooze", snoozeMinutes: 30,
    });
    expect(transport.calls.at(-1)).toEqual({
      command: "api_v1_handle_notification_action",
      args: { request: {
        clientRequestId: REQUEST_ID, scheduleId: SCHEDULE_ID, occurrenceId: OCCURRENCE_ID,
        action: "snooze", snoozeMinutes: 30,
      } },
    });
  });

  it("rejects a schedule response that weakens notification-only or IANA constraints", async () => {
    const transport = new Transport();
    transport.response = {
      schedules: [{ rule: { ...rule, notificationOnly: false, timezone: "local" }, nextOccurrenceAt: null }],
      revision: 1,
    };
    await expect(new CyberKindredIpcClient(transport).listSchedules())
      .rejects.toBeInstanceOf(IpcInvocationError);
  });
});
