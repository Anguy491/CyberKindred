import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { App } from "../../src/App";
import type { SettingsIpc } from "../../src/features/settings";
import type { DataCategoryInventory } from "../../src/ipc";

const PREVIEW_TOKEN = "018f47c0-8b8b-7c35-8bf7-278e15b1a211";

function fixture(): SettingsIpc {
  const inventory: DataCategoryInventory = {
    category: "chat_messages",
    itemCount: 2,
    storageClasses: ["sqlite"],
    retentionSummary: "创建后固定最多 30 天",
    externalRecipients: ["用户确认的 OpenAI-compatible origin"],
    deletionControl: "category",
    deletionCategory: "conversations_and_summaries",
  };
  return {
    getSettings: vi.fn(async () => { throw new Error("not used"); }),
    updateSettings: vi.fn(async () => { throw new Error("not used"); }),
    searchWeatherLocations: vi.fn(async () => { throw new Error("not used"); }),
    selectWeatherLocation: vi.fn(async () => { throw new Error("not used"); }),
    listSchedules: vi.fn(async () => ({ schedules: [], revision: 0 })),
    upsertSchedule: vi.fn(async () => { throw new Error("not used"); }),
    deleteSchedule: vi.fn(async () => { throw new Error("not used"); }),
    handleNotificationAction: vi.fn(async () => { throw new Error("not used"); }),
    startProgram: vi.fn(async () => { throw new Error("not used"); }),
    getDataInventory: vi.fn(async () => ({
      generatedAt: "2026-09-05T00:00:00.000Z", categories: [inventory],
    })),
    previewDataDeletion: vi.fn(async (category) => ({
      previewToken: PREVIEW_TOKEN,
      expiresAt: "2026-09-05T00:05:00.000Z",
      category,
      itemCount: 2,
      consequences: ["对话、摘要及其来源映射将被删除。"],
    })),
    deleteDataCategory: vi.fn(async (request) => ({
      requestId: request.clientRequestId,
      category: request.category,
      deletedCount: 2,
      restartRequired: false,
    })),
    exportUserData: vi.fn(async () => ({
      operationId: "018f47c0-8b8b-7c35-8bf7-278e15b1a212",
      acceptedAt: "2026-09-05T00:00:00.000Z",
    })),
    deleteAllUserData: vi.fn(async (clientRequestId) => ({
      requestId: clientRequestId, restartRequired: true,
    })),
    subscribeToEvents: vi.fn(async (handlers) => {
      await handlers.refreshSnapshot("initial");
      return () => undefined;
    }),
  };
}

describe("[TASK-027] privacy and data settings", () => {
  it("shows a path-free inventory and requires preview plus the exact category phrase", async () => {
    const ipc = fixture();
    const user = userEvent.setup();
    render(<App scenario="ready" initialRoute="settings" settingsIpc={ipc}
      fontStatusLoader={async () => "loaded"} />);
    await user.click(screen.getByRole("button", { name: "PRIVACY & DATA" }));

    const row = await screen.findByTestId("data-inventory-row");
    expect(within(row).getByText("CHAT MESSAGES")).not.toBeNull();
    expect(within(row).getByText(/2 项 · sqlite/u)).not.toBeNull();
    expect(row.textContent).not.toMatch(/[A-Z]:\\/u);

    await user.selectOptions(screen.getByLabelText("删除类别"), "conversations_and_summaries");
    await user.click(screen.getByRole("button", { name: "预览删除影响" }));
    const preview = await screen.findByTestId("data-deletion-preview");
    const deleteButton = within(preview).getByRole("button", { name: "永久删除所选类别" });
    expect((deleteButton as HTMLButtonElement).disabled).toBe(true);
    await user.type(within(preview).getByRole("textbox"), "DELETE SELECTED DATA");
    expect((deleteButton as HTMLButtonElement).disabled).toBe(false);
    await user.click(deleteButton);
    await waitFor(() => expect(ipc.deleteDataCategory).toHaveBeenCalledWith(expect.objectContaining({
      previewToken: PREVIEW_TOKEN,
      category: "conversations_and_summaries",
      confirmation: "DELETE SELECTED DATA",
    })));
  });

  it("keeps full reset disabled until the exact independent confirmation phrase", async () => {
    const ipc = fixture();
    const user = userEvent.setup();
    render(<App scenario="ready" initialRoute="settings" settingsIpc={ipc}
      fontStatusLoader={async () => "loaded"} />);
    await user.click(screen.getByRole("button", { name: "PRIVACY & DATA" }));
    const reset = screen.getByRole("button", { name: "全部重置并要求重启" });
    expect((reset as HTMLButtonElement).disabled).toBe(true);
    await user.type(screen.getByLabelText("输入 DELETE CYBERKINDRED DATA"), "DELETE SELECTED DATA");
    expect((reset as HTMLButtonElement).disabled).toBe(true);
    await user.clear(screen.getByLabelText("输入 DELETE CYBERKINDRED DATA"));
    await user.type(screen.getByLabelText("输入 DELETE CYBERKINDRED DATA"), "DELETE CYBERKINDRED DATA");
    expect((reset as HTMLButtonElement).disabled).toBe(false);
    await user.click(reset);
    await waitFor(() => expect(ipc.deleteAllUserData).toHaveBeenCalledWith(
      expect.any(String), "DELETE CYBERKINDRED DATA",
    ));
  });
});
