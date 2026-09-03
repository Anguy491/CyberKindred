import type {
  Ack,
  MusicSourcesResponse,
  PlaybackEvent,
  PlaybackState,
  ProgramPlan,
  ProgramSegmentEvent,
  ProgramStateEvent,
  ChatMessageEvent,
  OperationAccepted,
  OperationCancelledEvent,
  SelectMusicSourceResponse,
  StartProgramResponse,
} from "../../ipc";

export type RadioEvent =
  | { readonly type: "playback"; readonly payload: PlaybackEvent }
  | { readonly type: "program-state"; readonly payload: ProgramStateEvent }
  | { readonly type: "program-segment"; readonly payload: ProgramSegmentEvent }
  | { readonly type: "chat-message"; readonly payload: ChatMessageEvent }
  | { readonly type: "operation-cancelled"; readonly payload: OperationCancelledEvent };

/** Minimal, path-free command surface owned by the radio feature. */
export interface RadioIpc {
  listMusicSources(): Promise<MusicSourcesResponse>;
  getPlaybackState(): Promise<PlaybackState>;
  selectMusicSource(sourceId: string): Promise<SelectMusicSourceResponse>;
  startProgram(sourceId: string): Promise<StartProgramResponse>;
  stopProgram(programId: string): Promise<Ack>;
  play(expectedStateRevision: number): Promise<PlaybackState>;
  pause(expectedStateRevision: number): Promise<PlaybackState>;
  seek(expectedStateRevision: number, positionMs: number): Promise<PlaybackState>;
  next(expectedStateRevision: number): Promise<PlaybackState>;
  previous(expectedStateRevision: number): Promise<PlaybackState>;
  submitChat(programId: string, text: string): Promise<OperationAccepted>;
  cancelChat(operationId: string): Promise<void>;
  submitFeedback(
    programId: string,
    trackId: string | null,
    kind: "like" | "skip" | "less_talk",
  ): Promise<Ack>;
  subscribeRadio(
    handler: (event: RadioEvent) => void,
    refreshSnapshot: () => Promise<void>,
  ): Promise<() => void>;
}

export type { PlaybackState, ProgramPlan };
