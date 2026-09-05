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

function fixture(local = source(), additionalSources: ReadonlyArray<SourceSummary> = []) {
  let handler: ((event: RadioEvent) => void) | undefined;
  let refresh: (() => Promise<void>) | undefined;
  let currentPlayback = playback();
  const ipc: RadioIpc = {
    listMusicSources: vi.fn(async () => ({ sources: [local, ...additionalSources] })),
    getPlaybackState: vi.fn(async () => currentPlayback),
    selectMusicSource: vi.fn(async () => ({ requestId: id(100), state: currentPlayback })),
    startProgram: vi.fn(async () => ({ requestId: id(101), programId: PROGRAM_ID, plan: plan() })),
    stopProgram: vi.fn(async () => ({ requestId: id(102), revision: 9 })),
    play: vi.fn(async () => playback("playing", 2)),
    pause: vi.fn(async () => playback("paused", 3)),
    seek: vi.fn(async (_revision, positionMs) => ({ ...playback("playing", 4), positionMs })),
    next: vi.fn(async () => playback("playing", 5)),
    previous: vi.fn(async () => playback("playing", 6)),
    submitChat: vi.fn(async () => ({ operationId: id(120), acceptedAt: "2026-09-03T01:00:00.000Z" })),
    cancelChat: vi.fn(async () => undefined),
    submitFeedback: vi.fn(async () => ({ requestId: id(121), revision: 1 })),
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

  // FR-CHAT-001/003; cancellation must not invent a late assistant line.
  it("sends on Enter and exposes an explicit cancellation result", async () => {
    const test = fixture();
    const user = userEvent.setup();
    render(<RadioView ipc={test.ipc} />);
    await user.click(await screen.findByRole("button", { name: "开始节目" }));
    const input = screen.getByLabelText("告诉 CyberKindred 你现在想听什么");
    await user.type(input, "今天安静一点{Enter}");
    await waitFor(() => expect(test.ipc.submitChat).toHaveBeenCalledWith(PROGRAM_ID, "今天安静一点"));
    test.emit({ type: "chat-message", payload: {
      schemaVersion: "1.0.0", sequence: 5, occurredAt: "2026-09-03T01:00:04.000Z",
      operationId: id(120), programId: PROGRAM_ID, role: "user", text: "今天安静一点", final: false,
    } });
    expect(screen.getByText("今天安静一点")).not.toBeNull();
    await user.click(screen.getByRole("button", { name: "取消请求" }));
    expect(test.ipc.cancelChat).toHaveBeenCalledWith(id(120));
    test.emit({ type: "operation-cancelled", payload: {
      schemaVersion: "1.0.0", sequence: 6, occurredAt: "2026-09-03T01:00:05.000Z",
      operationId: id(120), kind: "chat",
    } });
    expect(screen.getByText(/不会产生 AI 回复或记忆提案/u)).not.toBeNull();
    expect(screen.queryByText("不应出现的回复")).toBeNull();
  });

  // FR-CHAT-001: a fast terminal event may arrive before the accept promise resolves.
  it("does not leave chat busy when a terminal event beats the accept response", async () => {
    const test = fixture();
    let resolveSubmit: ((value: { operationId: string; acceptedAt: string }) => void) | undefined;
    vi.mocked(test.ipc.submitChat).mockReturnValueOnce(new Promise((resolve) => {
      resolveSubmit = resolve;
    }));
    const user = userEvent.setup();
    render(<RadioView ipc={test.ipc} />);
    await user.click(await screen.findByRole("button", { name: "开始节目" }));
    await user.type(screen.getByLabelText("告诉 CyberKindred 你现在想听什么"), "现在怎么样{Enter}");
    await waitFor(() => expect(test.ipc.submitChat).toHaveBeenCalledTimes(1));
    test.emit({ type: "chat-message", payload: {
      schemaVersion: "1.0.0", sequence: 7, occurredAt: "2026-09-03T01:00:06.000Z",
      operationId: id(120), programId: PROGRAM_ID, role: "user", text: "现在怎么样", final: false,
    } });
    test.emit({ type: "chat-message", payload: {
      schemaVersion: "1.0.0", sequence: 8, occurredAt: "2026-09-03T01:00:07.000Z",
      operationId: id(120), programId: PROGRAM_ID, role: "assistant", text: "现在可以继续听。", final: true,
    } });
    await act(async () => resolveSubmit?.({
      operationId: id(120), acceptedAt: "2026-09-03T01:00:00.000Z",
    }));

    expect(await screen.findByText("现在可以继续听。")).not.toBeNull();
    expect(screen.getByRole("button", { name: "发送" }).getAttribute("aria-disabled")).toBe("true");
    await user.type(screen.getByLabelText("告诉 CyberKindred 你现在想听什么"), "再来一次");
    expect(screen.getByRole("button", { name: "发送" }).getAttribute("aria-disabled")).not.toBe("true");
  });

  // FR-RAD-005: acknowledgements are visible only after the persisted command resolves.
  it("persists like and less-talk feedback and shows the applied policy", async () => {
    const test = fixture();
    const user = userEvent.setup();
    render(<RadioView ipc={test.ipc} />);
    await user.click(await screen.findByRole("button", { name: "开始节目" }));
    test.emit({ type: "playback", payload: {
      schemaVersion: "1.0.0", eventId: id(130), sequence: 2, type: "track_changed",
      occurredAt: "2026-09-03T01:00:01.000Z", sourceId: "local", stateRevision: 2,
      reason: "adapter_update", state: playback("playing", 2),
    } });
    await user.click(await screen.findByRole("button", { name: "喜欢" }));
    expect(test.ipc.submitFeedback).toHaveBeenCalledWith(
      PROGRAM_ID, "018f47c0-8b8b-7c35-8bf7-278e15b1a101", "like",
    );
    expect(await screen.findByText(/后续节目选择会参考/u)).not.toBeNull();
    await user.click(screen.getByRole("button", { name: "少说一点" }));
    expect(test.ipc.submitFeedback).toHaveBeenCalledWith(PROGRAM_ID, null, "less_talk");
    expect(await screen.findByText(/每 4–6 首至多一次串场/u)).not.toBeNull();
  });
});

describe("[TASK-025] system media source events", () => {
  // FR-APL-001; user selection is the privacy gate for GSMTC discovery.
  it("allows an explicit connection attempt while the Apple session is disconnected", async () => {
    const apple: SourceSummary = {
      sourceId: "apple_music", kind: "system_session", displayName: "Apple Music Windows App",
      connected: false,
      capabilities: { play: false, pause: false, seek: false, next: false, previous: false, setQueue: false },
    };
    const test = fixture(source(), [apple]);
    const user = userEvent.setup();
    render(<RadioView ipc={test.ipc} />);

    const appleButton = await screen.findByRole("button", { name: "Apple Music Windows App" });
    expect(appleButton.getAttribute("aria-disabled")).not.toBe("true");
    expect(test.ipc.selectMusicSource).not.toHaveBeenCalled();
    await user.click(appleButton);
    await waitFor(() => expect(test.ipc.selectMusicSource).toHaveBeenCalledWith("apple_music"));
  });

  // FR-APL-001/002; NFR-REL-004.
  it("updates source availability without replacing the active source snapshot", async () => {
    const apple: SourceSummary = {
      sourceId: "apple_music", kind: "system_session", displayName: "Apple Music Windows App",
      connected: false,
      capabilities: { play: false, pause: false, seek: false, next: false, previous: false, setQueue: false },
    };
    const test = fixture(source(), [apple]);
    render(<RadioView ipc={test.ipc} />);
    await screen.findByRole("heading", { name: "今天想听什么状态？" });
    test.emit({ type: "playback", payload: {
      schemaVersion: "1.0.0", eventId: id(140), sequence: 2, type: "track_changed",
      occurredAt: "2026-09-03T01:00:01.000Z", sourceId: "apple_music", stateRevision: 1,
      reason: "adapter_update", state: {
        schemaVersion: "1.0.0", sourceId: "apple_music", sourceKind: "system_session",
        status: "playing",
        capabilities: { play: true, pause: true, seek: false, next: true, previous: true, setQueue: false },
        currentTrack: {
          trackId: "system:0123456789abcdef0123456789abcdef", title: "Apple 测试曲目",
          artist: null, album: null, artworkUri: null, origin: "system_session",
        },
        positionMs: 1_000, durationMs: 10_000, revision: 1,
        updatedAt: "2026-09-03T01:00:01.000Z", lastError: null,
      },
    } });
    expect(screen.getByRole("heading", { name: "今天想听什么状态？" })).not.toBeNull();
    const appleButton = screen.getByRole("button", { name: "Apple Music Windows App" });
    expect(appleButton.getAttribute("aria-disabled")).not.toBe("true");
  });

  // FR-APL-004; UX-RAD-006.
  it("starts companion mode without rendering an Apple queue plan", async () => {
    const apple: SourceSummary = {
      sourceId: "apple_music", kind: "system_session", displayName: "Apple Music Windows App",
      connected: true,
      capabilities: { play: true, pause: true, seek: false, next: true, previous: true, setQueue: false },
    };
    const state: PlaybackState = {
      schemaVersion: "1.0.0", sourceId: "apple_music", sourceKind: "system_session", status: "playing",
      capabilities: apple.capabilities,
      currentTrack: {
        trackId: "system:0123456789abcdef0123456789abcdef", title: "Apple 测试曲目",
        artist: null, album: null,
        artworkUri: "asset://artwork/system/0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        origin: "system_session",
      },
      positionMs: 1_000, durationMs: 10_000, revision: 1,
      updatedAt: "2026-09-03T01:00:01.000Z", lastError: null,
    };
    const test = fixture(source(), [apple]);
    vi.mocked(test.ipc.selectMusicSource).mockResolvedValue({ requestId: id(150), state });
    vi.mocked(test.ipc.startProgram).mockResolvedValue({
      requestId: id(151), programId: PROGRAM_ID, plan: null,
    });
    const user = userEvent.setup();
    render(<RadioView ipc={test.ipc} />);
    await user.click(await screen.findByRole("button", { name: "Apple Music Windows App" }));
    expect(await screen.findByText(/COMPANION MODE/u)).not.toBeNull();
    expect(screen.getByRole("img", { name: "Apple 测试曲目 封面" })
      .getAttribute("src")).toBe(state.currentTrack?.artworkUri);
    await user.click(screen.getByRole("button", { name: "开始节目" }));
    expect(test.ipc.startProgram).toHaveBeenCalledWith("apple_music");
    expect(screen.queryByRole("heading", { name: /节目计划/u })).toBeNull();
    expect(screen.getByText(/队列由 Apple Music 控制/u)).not.toBeNull();
  });
});
