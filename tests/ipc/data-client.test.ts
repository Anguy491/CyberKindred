import { describe, expect, it } from "vitest";

import { CyberKindredIpcClient, type IpcTransport, type IpcUnlisten } from "../../src/ipc";

const REQUEST_ID = "018f1f64-4ca0-7a2a-8e91-e89c389b3a31";
const OPERATION_ID = "018f1f64-4ca0-7a2a-8e91-e89c389b3a32";
const PREVIEW_TOKEN = "018f1f64-4ca0-7a2a-8e91-e89c389b3a33";

class FakeTransport implements IpcTransport {
  readonly calls: Array<{ command: string; args: Readonly<Record<string, unknown>> }> = [];
  response: unknown;

  constructor(response: unknown) {
    this.response = response;
  }

  async invoke<Response>(command: string, args: Readonly<Record<string, unknown>>): Promise<Response> {
    this.calls.push({ command, args });
    return this.response as Response;
  }

  async listen<Payload>(_eventName: string, _handler: (payload: Payload) => void): Promise<IpcUnlisten> {
    return () => undefined;
  }
}

const inventoryCategories = [
  "credentials", "profile_and_preferences", "weather_location_and_cache",
  "library_roots_and_identity", "embedded_music_tags", "metadata_matches",
  "artwork_and_tts_cache", "system_media_runtime", "playback_history_and_feedback",
  "chat_messages", "voice_segment_text", "session_summaries", "memory_proposals",
  "approved_memories_and_revisions", "schedules_and_notifications", "provider_usage_facts",
  "operation_outbox", "diagnostic_logs", "migration_backups",
] as const;

describe("[TASK-027] data-control IPC", () => {
  it("validates the complete 19-row inventory and exact command envelope", async () => {
    const transport = new FakeTransport({
      generatedAt: "2026-09-05T00:00:00.000Z",
      categories: inventoryCategories.map((category) => ({
        category, itemCount: 0, storageClasses: ["sqlite"], retentionSummary: "bounded",
        externalRecipients: [], deletionControl: "automatic", deletionCategory: null,
      })),
    });
    const client = new CyberKindredIpcClient(transport);
    await expect(client.getDataInventory()).resolves.toEqual(transport.response);
    expect(transport.calls[0]).toEqual({ command: "api_v1_get_data_inventory", args: { request: {} } });
  });

  it("keeps preview, deletion, export, and reset as separate explicit operations", async () => {
    const transport = new FakeTransport({
      previewToken: PREVIEW_TOKEN, expiresAt: "2026-09-05T00:05:00.000Z",
      category: "library_index", itemCount: 3,
      consequences: ["源音乐文件不会被删除。"],
    });
    const client = new CyberKindredIpcClient(transport);
    await client.previewDataDeletion("library_index");
    expect(transport.calls.at(-1)).toEqual({
      command: "api_v1_preview_data_deletion", args: { request: { category: "library_index" } },
    });

    transport.response = {
      requestId: REQUEST_ID, category: "library_index", deletedCount: 3, restartRequired: false,
    };
    await client.deleteDataCategory({
      clientRequestId: REQUEST_ID, previewToken: PREVIEW_TOKEN,
      category: "library_index", confirmation: "DELETE SELECTED DATA",
    });
    expect(transport.calls.at(-1)?.command).toBe("api_v1_delete_data_category");

    transport.response = { operationId: OPERATION_ID, acceptedAt: "2026-09-05T00:00:00.000Z" };
    await client.exportUserData(REQUEST_ID);
    expect(transport.calls.at(-1)).toEqual({
      command: "api_v1_export_user_data", args: { request: { clientRequestId: REQUEST_ID } },
    });

    transport.response = { requestId: REQUEST_ID, restartRequired: true };
    await client.deleteAllUserData(REQUEST_ID, "DELETE CYBERKINDRED DATA");
    expect(transport.calls.at(-1)).toEqual({
      command: "api_v1_delete_all_user_data",
      args: { request: { clientRequestId: REQUEST_ID, confirmation: "DELETE CYBERKINDRED DATA" } },
    });
  });
});
