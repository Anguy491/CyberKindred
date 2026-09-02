import { describe, expect, it } from "vitest";

import {
  CyberKindredIpcClient,
  IpcInvocationError,
  type IpcTransport,
  type IpcUnlisten,
} from "../../src/ipc";

const REQUEST_ID = "018f1f64-4ca0-7a2a-8e91-e89c389b3a31";
const OPERATION_ID = "018f1f64-4ca0-7a2a-8e91-e89c389b3a32";
const ROOT_ID = "018f1f64-4ca0-7a2a-8e91-e89c389b3a33";
const TRACK_ID = "018f1f64-4ca0-7a2a-8e91-e89c389b3a34";

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

describe("TASK-011/TASK-012 library IPC", () => {
  // API-013/API-014; FR-LIB-002; NFR-SEC-002.
  it("serializes explicit scan start/cancel requests and validates exact responses", async () => {
    const transport = new FakeTransport({ operationId: OPERATION_ID, acceptedAt: "2026-09-03T00:00:00Z" });
    const client = new CyberKindredIpcClient(transport);
    await expect(client.startLibraryScan({ clientRequestId: REQUEST_ID, rootIds: [ROOT_ID] }))
      .resolves.toEqual(transport.response);
    expect(transport.calls[0]).toEqual({
      command: "api_v1_start_library_scan",
      args: { request: { clientRequestId: REQUEST_ID, rootIds: [ROOT_ID] } },
    });

    transport.response = { requestId: REQUEST_ID, operationId: OPERATION_ID, state: "cancelled" };
    await expect(client.cancelLibraryScan({ clientRequestId: REQUEST_ID, operationId: OPERATION_ID }))
      .resolves.toEqual(transport.response);
    expect(transport.calls[1]).toEqual({
      command: "api_v1_cancel_library_scan",
      args: { request: { clientRequestId: REQUEST_ID, operationId: OPERATION_ID } },
    });
  });

  // API-015; FR-LIB-004/005; NFR-PRIV-004.
  it("accepts a path-free page with cached provenance and rejects extra path fields", async () => {
    const track = {
      trackId: TRACK_ID,
      availability: "playable",
      durationMs: 183_000,
      artworkAvailable: true,
      original: { title: "夜航", artist: "Fixture", album: null },
      enriched: {
        title: "Night Flight",
        artist: "Fixture",
        album: null,
        provider: "musicbrainz",
        confidence: 0.8,
        fetchedAt: "2026-09-03T00:00:00Z",
      },
      matchStatus: "review",
    } as const;
    const transport = new FakeTransport({ items: [track], nextCursor: null });
    const client = new CyberKindredIpcClient(transport);
    await expect(client.listTracks({
      cursor: null,
      limit: 50,
      query: "夜",
      sort: "title",
      filters: { availability: "playable", matchStatus: "review" },
    })).resolves.toEqual(transport.response);
    expect(transport.calls[0]?.command).toBe("api_v1_list_tracks");

    transport.response = {
      items: [{ ...track, relativePath: "private/track.mp3" }],
      nextCursor: null,
    };
    await expect(client.listTracks({
      cursor: null,
      limit: 50,
      query: null,
      sort: "recent",
      filters: { availability: null, matchStatus: null },
    })).rejects.toBeInstanceOf(IpcInvocationError);
  });

  // API-015; FR-LIB-004; NFR-MAINT-003.
  it("rejects incoherent enrichment status instead of trusting WebView data", async () => {
    const transport = new FakeTransport({
      items: [{
        trackId: TRACK_ID,
        availability: "playable",
        durationMs: 1,
        artworkAvailable: false,
        original: { title: null, artist: null, album: null },
        enriched: null,
        matchStatus: "matched",
      }],
      nextCursor: null,
    });
    const client = new CyberKindredIpcClient(transport);
    await expect(client.listTracks({
      cursor: null,
      limit: 1,
      query: null,
      sort: "title",
      filters: { availability: null, matchStatus: null },
    })).rejects.toBeInstanceOf(IpcInvocationError);
  });
});
