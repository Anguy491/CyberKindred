import type { IpcTransport, IpcUnlisten } from "./transport";
import { IPC_SCHEMA_VERSION, type PublicEventPayload } from "./types";
import { isApiError, isMemoryRecord, isPlaybackState, isRecord } from "./validation";

export const PUBLIC_EVENT_NAMES = [
  "cyberkindred://v1/playback/event",
  "cyberkindred://v1/program/state",
  "cyberkindred://v1/program/segment",
  "cyberkindred://v1/chat/message",
  "cyberkindred://v1/library/scan",
  "cyberkindred://v1/memory/proposed",
  "cyberkindred://v1/schedule/due",
  "cyberkindred://v1/operation/completed",
  "cyberkindred://v1/operation/failed",
  "cyberkindred://v1/app/resumed",
  "cyberkindred://v1/operation/cancelled",
] as const;

export type PublicEventName = (typeof PUBLIC_EVENT_NAMES)[number];
const MAX_PROCESS_OPERATION_TERMINALS = 4_096;
export type ResyncReason =
  | "initial"
  | "sequence_gap"
  | "resume"
  | "terminal"
  | "protocol_mismatch"
  | "invalid_event"
  | "event_during_resync";

export interface EventSubscriptionHandlers {
  /** Event payloads are hints only and must never replace the refreshed snapshot. */
  readonly onEvent: (eventName: PublicEventName, payload: PublicEventPayload) => void;
  /** Re-read all authoritative state needed by the active UI. */
  readonly refreshSnapshot: (reason: ResyncReason) => Promise<void>;
}

type SequenceObservation = "apply" | "awaiting_snapshot" | "gap" | "stale";
type OperationTerminalKind = "completed" | "failed" | "cancelled";
export type OperationTerminalObservation = "new" | "replay" | "conflict";

/** Process-local at-least-once dedupe with a deterministic memory bound. */
export class OperationTerminalTracker {
  readonly #capacity: number;
  readonly #entries = new Map<string, OperationTerminalKind>();

  constructor(capacity = MAX_PROCESS_OPERATION_TERMINALS) {
    if (!Number.isSafeInteger(capacity) || capacity <= 0) {
      throw new Error("Operation terminal tracker capacity must be a positive safe integer.");
    }
    this.#capacity = capacity;
  }

  get size(): number {
    return this.#entries.size;
  }

  classify(operationId: string, kind: OperationTerminalKind): OperationTerminalObservation {
    const known = this.#entries.get(operationId);
    if (known === undefined) return "new";
    return known === kind ? "replay" : "conflict";
  }

  remember(operationId: string, kind: OperationTerminalKind): void {
    if (this.#entries.has(operationId)) return;
    if (this.#entries.size === this.#capacity) {
      const oldest = this.#entries.keys().next().value;
      if (oldest !== undefined) this.#entries.delete(oldest);
    }
    this.#entries.set(operationId, kind);
  }
}

class GlobalSequenceTracker {
  private lastApplied: number | undefined;
  private highWater: number | undefined;
  private snapshotRequired = true;

  observe(sequence: number): SequenceObservation {
    this.highWater = this.highWater === undefined ? sequence : Math.max(this.highWater, sequence);
    if (this.snapshotRequired) {
      return "awaiting_snapshot";
    }
    if (this.lastApplied === undefined) {
      this.lastApplied = sequence;
      return "apply";
    }
    if (sequence <= this.lastApplied) {
      return "stale";
    }
    if (sequence === this.lastApplied + 1) {
      this.lastApplied = sequence;
      return "apply";
    }
    this.snapshotRequired = true;
    return "gap";
  }

  requireSnapshot(): void {
    this.snapshotRequired = true;
  }

  snapshotApplied(): void {
    this.lastApplied = this.highWater ?? this.lastApplied;
    this.snapshotRequired = false;
  }
}

/**
 * Subscribes to the complete v1 event allowlist so the process-global sequence
 * can be checked without treating other event channels as false gaps.
 */
export async function subscribeToPublicEvents(
  transport: IpcTransport,
  handlers: EventSubscriptionHandlers,
): Promise<IpcUnlisten> {
  const tracker = new GlobalSequenceTracker();
  const unlisteners: IpcUnlisten[] = [];
  let stopped = false;
  let resyncInFlight: Promise<void> | undefined;
  let pendingReason: ResyncReason | undefined;
  const operationTerminals = new OperationTerminalTracker();

  const requestResync = (reason: ResyncReason): void => {
    tracker.requireSnapshot();
    if (resyncInFlight !== undefined) {
      pendingReason = preferredReason(pendingReason, reason);
      return;
    }

    resyncInFlight = (async () => {
      let nextReason = reason;
      while (!stopped) {
        pendingReason = undefined;
        await handlers.refreshSnapshot(nextReason);
        tracker.snapshotApplied();
        if (pendingReason === undefined) {
          break;
        }
        tracker.requireSnapshot();
        nextReason = pendingReason;
      }
    })().finally(() => {
      const queuedReason = pendingReason;
      pendingReason = undefined;
      resyncInFlight = undefined;
      if (queuedReason !== undefined && !stopped) {
        requestResync(queuedReason);
      }
    });
    void resyncInFlight.catch(() => {
      // The view owns user-visible snapshot errors. Events remain blocked until
      // the caller starts a fresh subscription or another event retries resync.
      tracker.requireSnapshot();
    });
  };

  const receive = (eventName: PublicEventName, value: unknown): void => {
    const payload = parseEventPayload(eventName, value);
    if (payload === "protocol_mismatch" || payload === "invalid_event") {
      requestResync(payload);
      return;
    }

    const operationTerminal = getOperationTerminal(eventName, payload);
    const terminalObservation = operationTerminal === undefined
      ? undefined
      : operationTerminals.classify(operationTerminal.operationId, operationTerminal.kind);
    const terminalReplay = terminalObservation === "replay";
    const terminalConflict = terminalObservation === "conflict";
    const boundaryReason = eventName === "cyberkindred://v1/app/resumed"
      ? "resume"
      : isTerminalEvent(eventName, payload) ? "terminal" : undefined;
    const observation = tracker.observe(payload.sequence);
    if (terminalConflict) {
      requestResync("invalid_event");
      return;
    }
    if (observation === "stale") {
      return;
    }
    if (observation === "gap") {
      requestResync("sequence_gap");
      return;
    }
    if (observation === "awaiting_snapshot") {
      if (!terminalReplay) {
        requestResync(boundaryReason ?? "event_during_resync");
      }
      return;
    }

    if (terminalReplay) {
      return;
    }
    if (operationTerminal !== undefined) {
      operationTerminals.remember(operationTerminal.operationId, operationTerminal.kind);
    }

    handlers.onEvent(eventName, payload);
    if (boundaryReason !== undefined) {
      requestResync(boundaryReason);
    }
  };

  try {
    for (const eventName of PUBLIC_EVENT_NAMES) {
      const unlisten = await transport.listen<unknown>(eventName, (payload) => receive(eventName, payload));
      unlisteners.push(unlisten);
    }
  } catch {
    for (const unlisten of unlisteners) {
      unlisten();
    }
    throw new Error("无法建立本地事件订阅。");
  }

  requestResync("initial");
  if (resyncInFlight !== undefined) {
    await resyncInFlight;
  }
  return () => {
    stopped = true;
    for (const unlisten of unlisteners) {
      unlisten();
    }
  };
}

function parseEventPayload(
  eventName: PublicEventName,
  value: unknown,
): PublicEventPayload | "protocol_mismatch" | "invalid_event" {
  const envelope = parseEventEnvelope(value);
  if (envelope === "protocol_mismatch" || envelope === "invalid_event") {
    return envelope;
  }
  if (eventName === "cyberkindred://v1/operation/completed") {
    if (
      !hasExactKeys(envelope, [
        "schemaVersion", "sequence", "occurredAt", "operationId", "kind", "outputLabel",
      ])
      || !isUuid(envelope.operationId)
      || (envelope.kind !== "voice_preview" && envelope.kind !== "data_export")
      || (envelope.outputLabel !== null && !isSafeOutputLabel(envelope.outputLabel))
    ) {
      return "invalid_event";
    }
  }
  if (eventName === "cyberkindred://v1/operation/failed") {
    if (
      !hasExactKeys(envelope, [
        "schemaVersion", "sequence", "occurredAt", "operationId", "error",
      ])
      || !isUuid(envelope.operationId)
      || !isApiError(envelope.error)
    ) {
      return "invalid_event";
    }
  }
  if (eventName === "cyberkindred://v1/operation/cancelled") {
    if (
      !hasExactKeys(envelope, [
        "schemaVersion", "sequence", "occurredAt", "operationId", "kind",
      ])
      || !isUuid(envelope.operationId)
      || !isCancelledOperationKind(envelope.kind)
    ) {
      return "invalid_event";
    }
  }
  if (eventName === "cyberkindred://v1/library/scan") {
    if (
      !hasExactKeys(envelope, [
        "schemaVersion", "sequence", "occurredAt", "operationId", "state",
        "scanned", "discovered", "failed", "safeMessage",
      ])
      || !isUuid(envelope.operationId)
      || !isLibraryScanState(envelope.state)
      || !isNonNegativeSafeInteger(envelope.scanned)
      || !isNonNegativeSafeInteger(envelope.discovered)
      || !isNonNegativeSafeInteger(envelope.failed)
      || envelope.discovered > envelope.scanned
      || envelope.failed > envelope.scanned
      || (envelope.safeMessage !== null && !isBoundedEventText(envelope.safeMessage, 300))
    ) {
      return "invalid_event";
    }
  }
  if (eventName === "cyberkindred://v1/playback/event") {
    const stateRequired = envelope.type === "state_changed"
      || envelope.type === "track_changed" || envelope.type === "capabilities_changed";
    const disconnectedReasonValid = envelope.type !== "source_disconnected"
      || envelope.reason === "session_lost" || envelope.reason === "session_replaced"
      || envelope.reason === "media_error";
    if (
      !hasExactKeys(envelope, [
        "schemaVersion", "eventId", "sequence", "type", "occurredAt", "sourceId",
        "stateRevision", "reason", "state",
      ])
      || !isUuid(envelope.eventId)
      || !isPlaybackEventType(envelope.type)
      || !isSourceId(envelope.sourceId)
      || !isNonNegativeSafeInteger(envelope.stateRevision)
      || !isPlaybackReason(envelope.reason)
      || (envelope.state !== null && !isPlaybackState(envelope.state))
      || (stateRequired && envelope.state === null)
      || !disconnectedReasonValid
    ) {
      return "invalid_event";
    }
  }
  if (eventName === "cyberkindred://v1/program/state") {
    if (
      !hasExactKeys(envelope, [
        "schemaVersion", "sequence", "occurredAt", "programId", "state", "safeMessage",
      ])
      || !isUuid(envelope.programId)
      || !isProgramState(envelope.state)
      || (envelope.safeMessage !== null && !isBoundedEventText(envelope.safeMessage, 300))
    ) {
      return "invalid_event";
    }
  }
  if (eventName === "cyberkindred://v1/program/segment") {
    if (
      !hasExactKeys(envelope, [
        "schemaVersion", "sequence", "occurredAt", "programId", "segmentId", "state",
      ])
      || !isUuid(envelope.programId)
      || !isUuid(envelope.segmentId)
      || !isProgramSegmentState(envelope.state)
    ) {
      return "invalid_event";
    }
  }
  if (eventName === "cyberkindred://v1/chat/message") {
    if (
      !hasExactKeys(envelope, [
        "schemaVersion", "sequence", "occurredAt", "operationId", "programId", "role",
        "text", "final",
      ])
      || !isUuid(envelope.operationId) || !isUuid(envelope.programId)
      || (envelope.role !== "user" && envelope.role !== "assistant")
      || !isEventText(envelope.text, 20_000) || typeof envelope.final !== "boolean"
    ) {
      return "invalid_event";
    }
  }
  if (eventName === "cyberkindred://v1/memory/proposed") {
    if (!hasExactKeys(envelope, ["schemaVersion", "sequence", "occurredAt", "memory"])
      || !isMemoryRecord(envelope.memory) || envelope.memory.status !== "proposed") {
      return "invalid_event";
    }
  }
  if (eventName === "cyberkindred://v1/schedule/due") {
    if (!hasExactKeys(envelope, [
      "schemaVersion", "sequence", "occurredAt", "scheduleId", "occurrenceId",
      "notificationShown",
    ]) || !isUuid(envelope.scheduleId) || !isUuid(envelope.occurrenceId)
      || typeof envelope.notificationShown !== "boolean") {
      return "invalid_event";
    }
  }
  if (eventName === "cyberkindred://v1/app/resumed") {
    if (!hasExactKeys(envelope, ["schemaVersion", "sequence", "occurredAt", "sleptAt"])
      || (envelope.sleptAt !== null && !isTimestamp(envelope.sleptAt))) {
      return "invalid_event";
    }
  }
  return envelope;
}

function preferredReason(
  current: ResyncReason | undefined,
  candidate: ResyncReason,
): ResyncReason {
  const priority: Record<ResyncReason, number> = {
    initial: 0,
    event_during_resync: 1,
    sequence_gap: 2,
    invalid_event: 3,
    protocol_mismatch: 4,
    terminal: 5,
    resume: 6,
  };
  return current === undefined || priority[candidate] > priority[current] ? candidate : current;
}

function parseEventEnvelope(value: unknown): PublicEventPayload | "protocol_mismatch" | "invalid_event" {
  if (!isRecord(value)) {
    return "invalid_event";
  }
  if (value.schemaVersion !== IPC_SCHEMA_VERSION) {
    return "protocol_mismatch";
  }
  if (
    typeof value.sequence !== "number"
    || !Number.isSafeInteger(value.sequence)
    || value.sequence <= 0
    || typeof value.occurredAt !== "string"
    || Number.isNaN(Date.parse(value.occurredAt))
  ) {
    return "invalid_event";
  }
  return value as PublicEventPayload;
}

function isTerminalEvent(eventName: PublicEventName, payload: PublicEventPayload): boolean {
  if (
    eventName === "cyberkindred://v1/operation/completed"
    || eventName === "cyberkindred://v1/operation/failed"
    || eventName === "cyberkindred://v1/operation/cancelled"
  ) {
    return true;
  }
  if (eventName === "cyberkindred://v1/chat/message") {
    return payload.final === true;
  }
  if (eventName === "cyberkindred://v1/program/state") {
    return payload.state === "completed" || payload.state === "failed";
  }
  if (eventName === "cyberkindred://v1/program/segment") {
    return payload.state === "completed" || payload.state === "skipped" || payload.state === "failed";
  }
  if (eventName === "cyberkindred://v1/library/scan") {
    return payload.state === "completed" || payload.state === "cancelled" || payload.state === "failed";
  }
  return false;
}

function getOperationTerminal(
  eventName: PublicEventName,
  payload: PublicEventPayload,
): { readonly operationId: string; readonly kind: OperationTerminalKind } | undefined {
  const kind = eventName === "cyberkindred://v1/operation/completed"
    ? "completed"
    : eventName === "cyberkindred://v1/operation/failed"
      ? "failed"
      : eventName === "cyberkindred://v1/operation/cancelled" ? "cancelled" : undefined;
  if (kind !== undefined && typeof payload.operationId === "string") {
    return { operationId: payload.operationId, kind };
  }
  return undefined;
}

function isCancelledOperationKind(
  value: unknown,
): value is "chat" | "voice_preview" | "library_scan" | "data_export" {
  return value === "chat"
    || value === "voice_preview"
    || value === "library_scan"
    || value === "data_export";
}

function isLibraryScanState(value: unknown): value is "running" | "completed" | "cancelled" | "failed" {
  return value === "running" || value === "completed" || value === "cancelled" || value === "failed";
}

function isPlaybackEventType(value: unknown): boolean {
  return value === "state_changed" || value === "track_changed"
    || value === "capabilities_changed" || value === "source_disconnected"
    || value === "user_override" || value === "program_interrupted";
}

function isPlaybackReason(value: unknown): boolean {
  return value === null || value === "adapter_update" || value === "user_command"
    || value === "session_lost" || value === "session_replaced" || value === "user_media_key"
    || value === "tts_resume_aborted" || value === "media_error";
}

function isProgramState(value: unknown): boolean {
  return value === "planning" || value === "running" || value === "paused"
    || value === "stopping" || value === "completed" || value === "failed";
}

function isProgramSegmentState(value: unknown): boolean {
  return value === "queued" || value === "playing" || value === "completed"
    || value === "skipped" || value === "failed";
}

function isSourceId(value: unknown): boolean {
  return typeof value === "string" && value.length <= 128
    && /^[A-Za-z0-9._:-]+$/u.test(value);
}

function isNonNegativeSafeInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function isBoundedEventText(value: unknown, maxCodePoints: number): value is string {
  return typeof value === "string" && Array.from(value).length >= 1
    && Array.from(value).length <= maxCodePoints && !/\p{Cc}/u.test(value);
}

function isEventText(value: unknown, maxCodePoints: number): value is string {
  if (typeof value !== "string") return false;
  const length = Array.from(value).length;
  return length >= 1 && length <= maxCodePoints
    && !Array.from(value).some((character) => /\p{Cc}/u.test(character) && !/\s/u.test(character));
}

function isTimestamp(value: unknown): value is string {
  return typeof value === "string" && value.length <= 40 && !Number.isNaN(Date.parse(value));
}

function hasExactKeys(
  value: Record<string, unknown>,
  expected: ReadonlyArray<string>,
): boolean {
  const actual = Object.keys(value);
  return actual.length === expected.length && expected.every((key) => Object.hasOwn(value, key));
}

function isUuid(value: unknown): value is string {
  return typeof value === "string"
    && /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/iu.test(value);
}

function isSafeOutputLabel(value: unknown): value is string {
  return typeof value === "string"
    && value.length >= 1
    && value.length <= 200
    && !/\p{Cc}/u.test(value)
    && !/[\\/]/u.test(value);
}
