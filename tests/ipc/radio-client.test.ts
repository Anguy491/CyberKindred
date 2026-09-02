import { describe, expect, it } from "vitest";

import {
  CyberKindredIpcClient,
  IpcInvocationError,
  type IpcTransport,
  type IpcUnlisten,
} from "../../src/ipc";

const REQUEST_ID = "018f1f64-4ca0-7a2a-8e91-e89c389b3a31";
const PROGRAM_ID = "018f1f64-4ca0-7a2a-8e91-e89c389b3a32";
const SEGMENT_ID = "018f1f64-4ca0-7a2a-8e91-e89c389b3a33";
const TRACK_ID = "018f1f64-4ca0-7a2a-8e91-e89c389b3a34";
const OPERATION_ID = "018f1f64-4ca0-7a2a-8e91-e89c389b3a35";

const CAPABILITIES = {
  play: true,
  pause: true,
  seek: true,
  next: true,
  previous: true,
  setQueue: true,
} as const;

const PLAYBACK_STATE = {
  schemaVersion: "1.0.0",
  sourceId: "local",
  sourceKind: "local",
  status: "playing",
  capabilities: CAPABILITIES,
  currentTrack: {
    trackId: TRACK_ID,
    title: "Licensed Fixture",
    artist: "CyberKindred",
    album: null,
    artworkUri: null,
    origin: "local",
  },
  positionMs: 1_000,
  durationMs: 5_000,
  revision: 7,
  updatedAt: "2026-09-03T00:00:00Z",
  lastError: null,
} as const;

const PROGRAM_PLAN = {
  schemaVersion: "1.0.0",
  programId: PROGRAM_ID,
  sourceId: "local",
  mode: "local",
  createdAt: "2026-09-03T00:00:00Z",
  segments: [
    { type: "voice", segmentId: SEGMENT_ID, text: "节目开始。", trigger: "opening" },
    { type: "track", segmentId: REQUEST_ID, trackId: TRACK_ID, segueText: null },
  ],
} as const;

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

describe("TASK-014/TASK-018 radio IPC", () => {
  // API-016/API-018; FR-RAD-004; NFR-MAINT-003.
  it("reads exact source and playback snapshots without exposing a local path", async () => {
    const transport = new FakeTransport({
      sources: [{
        sourceId: "local",
        kind: "local",
        displayName: "本地曲库",
        connected: true,
        capabilities: CAPABILITIES,
      }],
    });
    const client = new CyberKindredIpcClient(transport);

    await expect(client.listMusicSources()).resolves.toEqual(transport.response);
    expect(transport.calls[0]).toEqual({
      command: "api_v1_list_music_sources",
      args: { request: {} },
    });

    transport.response = PLAYBACK_STATE;
    await expect(client.getPlaybackState()).resolves.toEqual(PLAYBACK_STATE);
    expect(transport.calls[1]).toEqual({
      command: "api_v1_get_playback_state",
      args: { request: {} },
    });

    transport.response = {
      ...PLAYBACK_STATE,
      currentTrack: { ...PLAYBACK_STATE.currentTrack, path: "C:\\Users\\Alice\\Music\\private.flac" },
    };
    await expect(client.getPlaybackState()).rejects.toBeInstanceOf(IpcInvocationError);
  });

  // API-017/API-019..023; FR-RAD-003/004; NFR-REL-001.
  it("serializes source selection and revision-bound playback controls", async () => {
    const transport = new FakeTransport({ requestId: REQUEST_ID, state: PLAYBACK_STATE });
    const client = new CyberKindredIpcClient(transport);
    await client.selectMusicSource({ clientRequestId: REQUEST_ID, sourceId: "local" });

    transport.response = PLAYBACK_STATE;
    await client.play({ clientRequestId: REQUEST_ID, expectedStateRevision: 7 });
    await client.pause({ clientRequestId: REQUEST_ID, expectedStateRevision: 7 });
    await client.seek({ clientRequestId: REQUEST_ID, expectedStateRevision: 7, positionMs: 2_000 });
    await client.next({ clientRequestId: REQUEST_ID, expectedStateRevision: 7 });
    await client.previous({ clientRequestId: REQUEST_ID, expectedStateRevision: 7 });

    expect(transport.calls.map(({ command }) => command)).toEqual([
      "api_v1_select_music_source",
      "api_v1_play",
      "api_v1_pause",
      "api_v1_seek",
      "api_v1_next",
      "api_v1_previous",
    ]);
    expect(transport.calls[3]?.args).toEqual({ request: {
      clientRequestId: REQUEST_ID,
      expectedStateRevision: 7,
      positionMs: 2_000,
    } });
  });

  // API-024/API-025; TEST-RAD-001; FR-RAD-001/002; NFR-REL-003.
  it("starts only with an explicit trigger and validates the exact returned plan", async () => {
    const transport = new FakeTransport({
      requestId: REQUEST_ID,
      programId: PROGRAM_ID,
      plan: PROGRAM_PLAN,
    });
    const client = new CyberKindredIpcClient(transport);

    await expect(client.startProgram({
      clientRequestId: REQUEST_ID,
      sourceId: "local",
      trigger: "manual",
    })).resolves.toEqual(transport.response);
    expect(transport.calls[0]).toEqual({
      command: "api_v1_start_program",
      args: { request: { clientRequestId: REQUEST_ID, sourceId: "local", trigger: "manual" } },
    });

    transport.response = { requestId: REQUEST_ID, revision: 8 };
    await client.stopProgram({ clientRequestId: REQUEST_ID, programId: PROGRAM_ID });
    expect(transport.calls[1]?.command).toBe("api_v1_stop_program");

    transport.response = {
      requestId: REQUEST_ID,
      programId: PROGRAM_ID,
      plan: { ...PROGRAM_PLAN, privatePath: "C:\\fixture.flac" },
    };
    await expect(client.startProgram({
      clientRequestId: REQUEST_ID,
      sourceId: "local",
      trigger: "manual",
    })).rejects.toBeInstanceOf(IpcInvocationError);
  });

  // API-038; FR-ONB-005; NFR-COST-001.
  it("cancels the declared operation kind and rejects extra response data", async () => {
    const transport = new FakeTransport({
      requestId: REQUEST_ID,
      operationId: OPERATION_ID,
      state: "cancelled",
    });
    const client = new CyberKindredIpcClient(transport);
    await client.cancelOperation({
      clientRequestId: REQUEST_ID,
      operationId: OPERATION_ID,
      expectedKind: "voice_preview",
    });
    expect(transport.calls[0]?.command).toBe("api_v1_cancel_operation");

    transport.response = { ...transport.response as object, providerBody: "never trust" };
    await expect(client.cancelOperation({
      clientRequestId: REQUEST_ID,
      operationId: OPERATION_ID,
      expectedKind: "voice_preview",
    })).rejects.toBeInstanceOf(IpcInvocationError);
  });
});
