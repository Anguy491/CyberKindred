import {
  CyberKindredIpcClient,
  type PlaybackEvent,
  type PlaybackState,
  type ProgramSegmentEvent,
  type ProgramStateEvent,
  type ChatMessageEvent,
  type OperationCancelledEvent,
} from "../../ipc";
import type { RadioIpc } from "./types";

type RadioClient = Pick<CyberKindredIpcClient,
  | "listMusicSources"
  | "getPlaybackState"
  | "selectMusicSource"
  | "startProgram"
  | "stopProgram"
  | "play"
  | "pause"
  | "seek"
  | "next"
  | "previous"
  | "submitChat"
  | "submitFeedback"
  | "cancelOperation"
  | "subscribeToEvents"
>;

/** Adapts strict API-016..025 and the process-global event stream. */
export function createRadioIpc(client: RadioClient = new CyberKindredIpcClient()): RadioIpc {
  const control = (expectedStateRevision: number) => ({
    clientRequestId: globalThis.crypto.randomUUID(),
    expectedStateRevision,
  });
  return {
    listMusicSources: () => client.listMusicSources(),
    getPlaybackState: () => client.getPlaybackState(),
    selectMusicSource: (sourceId) => client.selectMusicSource({
      clientRequestId: globalThis.crypto.randomUUID(), sourceId,
    }),
    startProgram: (sourceId) => client.startProgram({
      clientRequestId: globalThis.crypto.randomUUID(), sourceId, trigger: "manual",
    }),
    stopProgram: (programId) => client.stopProgram({
      clientRequestId: globalThis.crypto.randomUUID(), programId,
    }),
    play: (revision) => client.play(control(revision)),
    pause: (revision) => client.pause(control(revision)),
    seek: (revision, positionMs) => client.seek({ ...control(revision), positionMs }),
    next: (revision) => client.next(control(revision)),
    previous: (revision) => client.previous(control(revision)),
    submitChat: (programId, text) => client.submitChat({
      clientRequestId: globalThis.crypto.randomUUID(), programId, text,
    }),
    cancelChat: async (operationId) => {
      await client.cancelOperation({
        clientRequestId: globalThis.crypto.randomUUID(), operationId, expectedKind: "chat",
      });
    },
    submitFeedback: (programId, trackId, kind) => client.submitFeedback({
      clientRequestId: globalThis.crypto.randomUUID(), programId, trackId, kind,
    }),
    subscribeRadio: (handler, refreshSnapshot) => client.subscribeToEvents({
      onEvent: (eventName, payload) => {
        if (eventName === "cyberkindred://v1/playback/event") {
          handler({ type: "playback", payload: payload as unknown as PlaybackEvent });
        } else if (eventName === "cyberkindred://v1/program/state") {
          handler({ type: "program-state", payload: payload as unknown as ProgramStateEvent });
        } else if (eventName === "cyberkindred://v1/program/segment") {
          handler({ type: "program-segment", payload: payload as unknown as ProgramSegmentEvent });
        } else if (eventName === "cyberkindred://v1/chat/message") {
          handler({ type: "chat-message", payload: payload as unknown as ChatMessageEvent });
        } else if (eventName === "cyberkindred://v1/operation/cancelled") {
          handler({ type: "operation-cancelled", payload: payload as unknown as OperationCancelledEvent });
        }
      },
      refreshSnapshot,
    }),
  };
}

const EMPTY_PLAYBACK: PlaybackState = {
  schemaVersion: "1.0.0",
  sourceId: "local",
  sourceKind: "local",
  status: "disconnected",
  capabilities: {
    play: false, pause: false, seek: false, next: false, previous: false, setQueue: false,
  },
  currentTrack: null,
  positionMs: 0,
  durationMs: null,
  revision: 0,
  updatedAt: "2026-09-03T00:00:00.000Z",
  lastError: null,
};

/** Browser-only fixture. It exposes no sound-capable operation. */
export const BROWSER_RADIO_IPC: RadioIpc = {
  listMusicSources: async () => ({ sources: [] }),
  getPlaybackState: async () => EMPTY_PLAYBACK,
  selectMusicSource: async () => { throw new Error("Radio requires the desktop runtime."); },
  startProgram: async () => { throw new Error("Radio requires the desktop runtime."); },
  stopProgram: async () => { throw new Error("Radio requires the desktop runtime."); },
  play: async () => { throw new Error("Radio requires the desktop runtime."); },
  pause: async () => { throw new Error("Radio requires the desktop runtime."); },
  seek: async () => { throw new Error("Radio requires the desktop runtime."); },
  next: async () => { throw new Error("Radio requires the desktop runtime."); },
  previous: async () => { throw new Error("Radio requires the desktop runtime."); },
  submitChat: async () => { throw new Error("Radio requires the desktop runtime."); },
  cancelChat: async () => { throw new Error("Radio requires the desktop runtime."); },
  submitFeedback: async () => { throw new Error("Radio requires the desktop runtime."); },
  subscribeRadio: async (_handler, refreshSnapshot) => {
    await refreshSnapshot();
    return () => undefined;
  },
};
