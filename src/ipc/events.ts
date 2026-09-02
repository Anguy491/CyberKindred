import type { IpcTransport, IpcUnlisten } from "./transport";
import { IPC_SCHEMA_VERSION, type PublicEventPayload } from "./types";
import { isRecord } from "./validation";

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
    const payload = parseEventEnvelope(value);
    if (payload === "protocol_mismatch" || payload === "invalid_event") {
      requestResync(payload);
      return;
    }

    const boundaryReason = eventName === "cyberkindred://v1/app/resumed"
      ? "resume"
      : isTerminalEvent(eventName, payload) ? "terminal" : undefined;
    const observation = tracker.observe(payload.sequence);
    if (observation === "stale") {
      return;
    }
    if (observation === "gap") {
      requestResync("sequence_gap");
      return;
    }
    if (observation === "awaiting_snapshot") {
      requestResync(boundaryReason ?? "event_during_resync");
      return;
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
