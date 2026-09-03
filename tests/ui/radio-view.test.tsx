import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";

import { RadioView, type RadioEvent, type RadioIpc } from "../../src/features/radio";
import type { PlaybackState, ProgramPlan, SourceSummary } from "../../src/ipc";

const PROGRAM_ID = "018f47c0-8b8b-7c35-8bf7-278e15b1a001";

function source(connected = true, setQueue = true): SourceSummary {
  return {
    sourceId: "local", kind: "local", displayName: "本地曲库", connected,
    capabilities: { play: true, pause: true, seek: true, next: true, previous: true, setQueue },
  };
}

function playback(status: PlaybackState["status"] = "idle", revision = 1): PlaybackState {
  return {
    schemaVersion: "1.0.0", sourceId: "local", sourceKind: "local", status,
    capabilities: { play: true, pause: true, seek: true, next: true, previous: true, setQueue: true },
    currentTrack: status === "playing" ? {
      trackId: "018f47c0-8b8b-7c35-8bf7-278e15b1a101", title: "许可测试音",
      artist: "CyberKindred", album: "CC0 Fixtures", artworkUri: null, origin: "local",
    } : null,
    positionMs: status === "playing" ? 2_000 : 0,
    durationMs: status === "playing" ? 10_000 : null,
    revision, updatedAt: "2026-09-03T01:00:00.000Z", lastError: null,
  };
}

function plan(): ProgramPlan {
  const segments: ProgramPlan["segments"] = [
    { type: "voice", segmentId: id(1), text: "欢迎来到本地电台。", trigger: "opening" },
    ...Array.from({ length: 6 }, (_, index) => ({
      type: "track" as const, segmentId: id(index + 2), trackId: id(index + 20), segueText: null,
    })),
  ];
  return {
    schemaVersion: "1.0.0", programId: PROGRAM_ID, sourceId: "local", mode: "local",
    createdAt: "2026-09-03T01:00:00.000Z", segments,
  };
}

function id(index: number): string {
  return `018f47c0-8b8b-7c35-8bf7-${String(index).padStart(12, "0")}`;
}

function fixture(local = source()) {
  let handler: ((event: RadioEvent) => void) | undefined;
  let refresh: (() => Promise<void>) | undefined;
  let currentPlayback = playback();
  const ipc: RadioIpc = {
    listMusicSources: vi.fn(async () => ({ sources: [local] })),
    getPlaybackState: vi.fn(async () => currentPlayback),
    selectMusicSource: vi.fn(async () => ({ requestId: id(100), state: currentPlayback })),
    startProgram: vi.fn(async () => ({ requestId: id(101), programId: PROGRAM_ID, plan: plan() })),
    stopProgram: vi.fn(async () => ({ requestId: id(102), revision: 9 })),
    play: vi.fn(async () => playback("playing", 2)),
    pause: vi.fn(async () => playback("paused", 3)),
    seek: vi.fn(async (_revision, positionMs) => ({ ...playback("playing", 4), positionMs })),
    next: vi.fn(async () => playback("playing", 5)),
    previous: vi.fn(async () => playback("playing", 6)),
    subscribeRadio: vi.fn(async (nextHandler, nextRefresh) => {
      handler = nextHandler; refresh = nextRefresh; await nextRefresh(); return () => undefined;
    }),
  };
  return {
    ipc,
    emit: (event: RadioEvent) => act(() => handler?.(event)),
    resync: async (state: PlaybackState) => {
      currentPlayback = state;
      await act(async () => refresh?.());
    },
  };
}

describe("[TASK-019] local radio view", () => {
  // FR-RAD-001; NFR-COST-001; NFR-PRIV-001.
  it("mounts silently and starts exactly once only after the explicit click", async () => {
    const test = fixture();
    const user = userEvent.setup();
    render(<RadioView ipc={test.ipc} />);
    const start = await screen.findByRole("button", { name: "开始节目" });
    expect(test.ipc.startProgram).not.toHaveBeenCalled();
    expect(test.ipc.selectMusicSource).not.toHaveBeenCalled();
    expect(test.ipc.play).not.toHaveBeenCalled();

    await user.dblClick(start);
    await waitFor(() => expect(test.ipc.startProgram).toHaveBeenCalledTimes(1));
    expect(test.ipc.startProgram).toHaveBeenCalledWith("local");
    expect(screen.getByRole("heading", { name: "节目计划 / 6 首" })).not.toBeNull();
    expect(screen.getAllByText(/曲目 0[1-6]/u)).toHaveLength(6);
  });

  // FR-RAD-003/004/006; NFR-REL-001.
  it("applies validated events, refreshes snapshots, and sends revision-bound controls", async () => {
    const test = fixture();
    const user = userEvent.setup();
    render(<RadioView ipc={test.ipc} />);
    await user.click(await screen.findByRole("button", { name: "开始节目" }));
    test.emit({ type: "playback", payload: {
      schemaVersion: "1.0.0", eventId: id(110), sequence: 2, type: "state_changed",
      occurredAt: "2026-09-03T01:00:01.000Z", sourceId: "local", stateRevision: 2,
      reason: "adapter_update", state: playback("playing", 2),
    } });
    expect(await screen.findByRole("heading", { name: "许可测试音" })).not.toBeNull();
    await user.click(screen.getByRole("button", { name: "暂停" }));
    expect(test.ipc.pause).toHaveBeenCalledWith(2);
    await user.click(screen.getByRole("button", { name: "下一首" }));
    expect(test.ipc.next).toHaveBeenCalledWith(3);

    await test.resync(playback("paused", 8));
    expect(screen.getByText("8")).not.toBeNull();
    await user.click(screen.getByRole("button", { name: "停止节目" }));
    expect(test.ipc.stopProgram).toHaveBeenCalledWith(PROGRAM_ID);
  });

  // FR-RAD-002/007; NFR-REL-003; NFR-COST-002.
  it("keeps deterministic planning and TTS failures visible while text continues", async () => {
    const test = fixture();
    const user = userEvent.setup();
    render(<RadioView ipc={test.ipc} />);
    await user.click(await screen.findByRole("button", { name: "开始节目" }));
    test.emit({ type: "program-state", payload: {
      schemaVersion: "1.0.0", sequence: 2, occurredAt: "2026-09-03T01:00:01.000Z",
      programId: PROGRAM_ID, state: "running", safeMessage: "AI 计划不可用，已使用确定性本地队列。",
    } });
    test.emit({ type: "program-state", payload: {
      schemaVersion: "1.0.0", sequence: 3, occurredAt: "2026-09-03T01:00:02.000Z",
      programId: PROGRAM_ID, state: "running", safeMessage: "语音不可用，已保留文字并继续播放。",
    } });
    test.emit({ type: "program-segment", payload: {
      schemaVersion: "1.0.0", sequence: 4, occurredAt: "2026-09-03T01:00:03.000Z",
      programId: PROGRAM_ID, segmentId: id(1), state: "playing",
    } });
    expect(screen.getByText(/\[DETERMINISTIC LOCAL QUEUE\]/u)).not.toBeNull();
    expect(screen.getByText(/\[TTS UNAVAILABLE — TEXT CONTINUES\]/u)).not.toBeNull();
    expect(screen.getAllByText("欢迎来到本地电台。").length).toBeGreaterThan(0);
  });

  // FR-RAD-004; NFR-PRIV-001.
  it("fails closed when local queue capability is unavailable", async () => {
    const test = fixture(source(true, false));
    const user = userEvent.setup();
    render(<RadioView ipc={test.ipc} />);
    const start = await screen.findByRole("button", { name: "开始节目" });
    expect(start.getAttribute("aria-disabled")).toBe("true");
    await user.click(start);
    expect(test.ipc.startProgram).not.toHaveBeenCalled();
    expect(screen.getByText("[UNAVAILABLE: QUEUE CAPABILITY]")).not.toBeNull();
  });

  // NFR-SEC-002; NFR-PRIV-003.
  it("never renders raw path or secret text from an untrusted adapter failure", async () => {
    const test = fixture();
    vi.mocked(test.ipc.startProgram).mockRejectedValueOnce(
      new Error("C:\\Users\\Alice\\Music sk-radio-secret-canary"),
    );
    const user = userEvent.setup();
    render(<RadioView ipc={test.ipc} />);
    await user.click(await screen.findByRole("button", { name: "开始节目" }));
    expect((await screen.findByRole("alert")).textContent).toContain("本地电台操作失败");
    expect(document.body.textContent).not.toContain("Alice");
    expect(document.body.textContent).not.toContain("sk-radio-secret-canary");
  });
});
