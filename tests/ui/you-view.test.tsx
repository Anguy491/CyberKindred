import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";

import { YouView, type YouIpc } from "../../src/features/you";
import type { MemoryRecord } from "../../src/ipc";

const MEMORY_ID = "018f47c0-8b8b-7c35-8bf7-278e15b1a201";
const SUMMARY_ID = "018f47c0-8b8b-7c35-8bf7-278e15b1a202";

function memory(status: MemoryRecord["status"]): MemoryRecord {
  return {
    schemaVersion: "1.0.0", memoryId: MEMORY_ID, status, kind: "preference",
    content: "喜欢安静的环境音乐", confidence: 0.9,
    sourceSessionId: "018f47c0-8b8b-7c35-8bf7-278e15b1a203",
    createdAt: "2026-09-03T01:00:00.000Z", updatedAt: "2026-09-03T01:00:01.000Z",
    approvedAt: status === "proposed" ? null : "2026-09-03T01:00:01.000Z",
    lastUsedAt: null, enabled: status === "approved", revision: status === "proposed" ? 0 : 1,
  };
}

function fixture(): YouIpc {
  return {
    getProfile: vi.fn(async () => ({
      profile: { displayName: "小岚", companionStyle: "quiet_warm", initialPreferences: ["夜间"], narrationDensity: "balanced", weatherLocation: null },
      preferenceTrends: [{ kind: "like", label: "喜欢", direction: "up", sampleCount: 3, windowDays: 30 }], revision: 2,
    })),
    updateProfile: vi.fn(async () => ({ requestId: crypto.randomUUID(), revision: 3 })),
    listMemories: vi.fn(async () => ({ items: [memory("proposed")], nextCursor: null })),
    approveMemory: vi.fn(async () => memory("approved")),
    updateMemory: vi.fn(async (_id, _revision, content, enabled) => ({ ...memory(enabled ? "approved" : "proposed"), content, revision: 1 })),
    rejectMemory: vi.fn(async () => ({ requestId: crypto.randomUUID(), memoryId: MEMORY_ID, status: "rejected", rejectedAt: "2026-09-03T01:00:02.000Z", contentDeleteAt: "2026-10-03T01:00:02.000Z", revision: 1 })),
    deleteMemory: vi.fn(async () => ({ requestId: crypto.randomUUID(), revision: 2 })),
    listSummaries: vi.fn(async () => ({ items: [{
      summaryId: SUMMARY_ID, coveredFrom: "2026-09-01T00:00:00.000Z", coveredTo: "2026-09-01T00:10:00.000Z",
      summary: "会话包含 1 条用户消息和 1 条助手消息。", generationKind: "deterministic", revision: 1,
    }], nextCursor: null })),
    deleteSummary: vi.fn(async () => ({ requestId: crypto.randomUUID(), revision: 2 })),
  };
}

describe("[TASK-021/022] YOU memory and summary controls", () => {
  it("shows profile, memory and summary provenance/status and supports approval", async () => {
    const ipc = fixture();
    const user = userEvent.setup();
    render(<YouView ipc={ipc} />);
    expect(await screen.findByDisplayValue("小岚")).not.toBeNull();
    expect(screen.getByText(/来源：会话/u)).not.toBeNull();
    expect(screen.getByText(/状态：proposed/u)).not.toBeNull();
    expect(screen.getByText(/本机确定性统计/u)).not.toBeNull();
    expect(screen.getByText("喜欢：上升")).not.toBeNull();
    await user.click(screen.getByRole("button", { name: "批准" }));
    await waitFor(() => expect(ipc.approveMemory).toHaveBeenCalledWith(MEMORY_ID, 0));
  });

  it("edits the profile and can reject a proposal or delete a summary", async () => {
    const ipc = fixture();
    const user = userEvent.setup();
    render(<YouView ipc={ipc} />);
    const name = await screen.findByLabelText("称呼");
    await user.clear(name); await user.type(name, "新称呼");
    await user.click(screen.getByRole("button", { name: "保存画像" }));
    await waitFor(() => expect(ipc.updateProfile).toHaveBeenCalledWith(2, expect.objectContaining({ displayName: "新称呼" })));
    await user.click(screen.getByRole("button", { name: "拒绝" }));
    await waitFor(() => expect(ipc.rejectMemory).toHaveBeenCalledWith(MEMORY_ID, 0));
    await user.click(screen.getByRole("button", { name: "删除摘要" }));
    await waitFor(() => expect(ipc.deleteSummary).toHaveBeenCalledWith(SUMMARY_ID, 1));
  });
});
