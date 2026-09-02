import { readFileSync, readdirSync } from "node:fs";
import { extname, join, relative } from "node:path";

import { afterEach, describe, expect, it, vi } from "vitest";

import {
  CyberKindredIpcClient,
  IpcInvocationError,
  PUBLIC_EVENT_NAMES,
  type AppCapabilities,
  type IpcTransport,
  type IpcUnlisten,
  type PublicEventName,
  type PublicEventPayload,
  type ResyncReason,
} from "../../src/ipc";

const CAPABILITIES: AppCapabilities = {
  protocolVersion: "1.0.0",
  appVersion: "0.1.0",
  platform: "windows",
  osVersion: "Windows 11",
  features: {
    localLibrary: true,
    systemMediaSession: false,
    musicKit: false,
    appleMusicDomControl: false,
    externalHttpApi: false,
  },
  sources: [],
  providers: [],
};

class FakeIpcTransport implements IpcTransport {
  readonly invocations: Array<{ command: string; args: Readonly<Record<string, unknown>> }> = [];
  readonly listeners = new Map<string, Set<(payload: unknown) => void>>();
  response: unknown = CAPABILITIES;
  rejection: unknown;
  pending = false;

  async invoke<Response>(command: string, args: Readonly<Record<string, unknown>>): Promise<Response> {
    this.invocations.push({ command, args });
    if (this.pending) {
      return new Promise<Response>(() => undefined);
    }
    if (this.rejection !== undefined) {
      throw this.rejection;
    }
    return this.response as Response;
  }

  async listen<Payload>(eventName: string, handler: (payload: Payload) => void): Promise<IpcUnlisten> {
    const wrapped = (payload: unknown) => handler(payload as Payload);
    const listeners = this.listeners.get(eventName) ?? new Set<(payload: unknown) => void>();
    listeners.add(wrapped);
    this.listeners.set(eventName, listeners);
    return () => listeners.delete(wrapped);
  }

  emit(eventName: PublicEventName, payload: unknown): void {
    for (const listener of this.listeners.get(eventName) ?? []) {
      listener(payload);
    }
  }
}

afterEach(() => {
  vi.useRealTimers();
});

describe("TASK-006 typed IPC client", () => {
  // API-001; FR-RAD-004; FR-SET-004; NFR-SEC-002.
  it("API-001 serializes only an empty request and accepts the strict capability DTO", async () => {
    const transport = new FakeIpcTransport();
    const client = new CyberKindredIpcClient(transport);

    await expect(client.getCapabilities()).resolves.toEqual(CAPABILITIES);
    expect(transport.invocations).toEqual([{
      command: "api_v1_get_capabilities",
      args: {},
    }]);
  });

  // API-001; NFR-MAINT-003; NFR-SEC-004.
  it("rejects malformed IPC data without echoing the response body", async () => {
    const transport = new FakeIpcTransport();
    transport.response = { ...CAPABILITIES, secret: "sk-canary-never-echo" };
    const client = new CyberKindredIpcClient(transport);

    const result = client.getCapabilities();
    await expect(result).rejects.toMatchObject({
      name: "IpcInvocationError",
      apiError: { errorId: "ERR-1601", safeMessage: "本地通信失败，请刷新后重试。" },
    });
    await expect(result).rejects.not.toThrow("sk-canary-never-echo");
  });

  // API Contract section 5; NFR-SEC-004.
  it("normalizes an upstream rejection without retaining body, path, or secret text", async () => {
    const transport = new FakeIpcTransport();
    transport.rejection = new Error(
      "Authorization: Bearer sk-upstream-canary C:\\Users\\Alice\\Music private body",
    );
    const client = new CyberKindredIpcClient(transport);

    const result = client.getCapabilities();
    await expect(result).rejects.toMatchObject({
      apiError: {
        errorId: "ERR-1601",
        details: { reason: "unexpected_internal" },
      },
    });
    await expect(result).rejects.not.toThrow("sk-upstream-canary");
  });

  // API-001; NFR-MAINT-003.
  it("stops waiting after two seconds without issuing a cancellation IPC", async () => {
    vi.useFakeTimers();
    const transport = new FakeIpcTransport();
    transport.pending = true;
    const client = new CyberKindredIpcClient(transport);

    const result = client.getCapabilities();
    const assertion = expect(result).rejects.toBeInstanceOf(IpcInvocationError);
    await vi.advanceTimersByTimeAsync(2_000);
    await assertion;
    expect(transport.invocations).toHaveLength(1);
    expect(transport.invocations[0]?.command).toBe("api_v1_get_capabilities");
  });

  // API Contract section 4; FR-RAD-004; NFR-MAINT-003.
  it("subscribes to the global sequence and resyncs on gap, resume, and terminal", async () => {
    const transport = new FakeIpcTransport();
    const client = new CyberKindredIpcClient(transport);
    const received: Array<{ name: PublicEventName; payload: PublicEventPayload }> = [];
    const reasons: ResyncReason[] = [];
    const stop = await client.subscribeToEvents({
      onEvent: (name, payload) => received.push({ name, payload }),
      refreshSnapshot: async (reason) => {
        reasons.push(reason);
      },
    });
    await vi.waitFor(() => expect(reasons).toContain("initial"));
    expect([...transport.listeners.keys()]).toEqual(PUBLIC_EVENT_NAMES);

    transport.emit("cyberkindred://v1/playback/event", event(1));
    expect(received).toHaveLength(1);
    transport.emit("cyberkindred://v1/playback/event", event(3));
    await vi.waitFor(() => expect(reasons).toContain("sequence_gap"));
    expect(received).toHaveLength(1);

    transport.emit("cyberkindred://v1/app/resumed", event(4));
    await vi.waitFor(() => expect(reasons).toContain("resume"));
    transport.emit("cyberkindred://v1/operation/completed", event(5));
    await vi.waitFor(() => expect(reasons).toContain("terminal"));
    stop();
    expect([...transport.listeners.values()].every((listeners) => listeners.size === 0)).toBe(true);
  });

  // API Contract section 6; NFR-MAINT-003.
  it("ignores a newer protocol event and refreshes authoritative snapshots", async () => {
    const transport = new FakeIpcTransport();
    const client = new CyberKindredIpcClient(transport);
    const onEvent = vi.fn();
    const reasons: ResyncReason[] = [];
    const stop = await client.subscribeToEvents({
      onEvent,
      refreshSnapshot: async (reason) => {
        reasons.push(reason);
      },
    });
    await vi.waitFor(() => expect(reasons).toContain("initial"));
    transport.emit("cyberkindred://v1/playback/event", { ...event(1), schemaVersion: "2.0.0" });
    await vi.waitFor(() => expect(reasons).toContain("protocol_mismatch"));
    expect(onEvent).not.toHaveBeenCalled();
    stop();
  });

  // ARCH-001; NFR-SEC-002; NFR-SEC-004.
  it("keeps frontend file, database, credential, and network access outside product runtime", () => {
    const sourceRoot = join(process.cwd(), "src");
    const files = readdirSync(sourceRoot, { recursive: true, withFileTypes: true })
      .filter((entry) => entry.isFile() && [".ts", ".tsx"].includes(extname(entry.name)))
      .map((entry) => join(entry.parentPath, entry.name));
    const directAccess = /\b(?:fetch|XMLHttpRequest|WebSocket)\s*\(|from\s+["'](?:node:)?(?:fs|path|net|http|https|sqlite)["']|CredentialManager/gu;

    for (const file of files) {
      const source = readFileSync(file, "utf8");
      expect(source.match(directAccess), relative(sourceRoot, file)).toBeNull();
      if (source.includes("@tauri-apps/api")) {
        expect(relative(sourceRoot, file)).toBe(join("ipc", "transport.ts"));
      }
    }
  });
});

function event(sequence: number): PublicEventPayload {
  return {
    schemaVersion: "1.0.0",
    sequence,
    occurredAt: "2026-09-02T00:00:00Z",
  };
}
