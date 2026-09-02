import "@testing-library/jest-dom/vitest";

import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import axe from "axe-core";
import { afterEach, describe, expect, it, vi } from "vitest";

import { LibraryView } from "./LibraryView";
import type {
  CancelLibraryScanView,
  LibraryIpc,
  LibraryRootView,
  LibraryScanEvent,
  ListTracksRequest,
  OperationAcceptedView,
  PickLibraryRootView,
  TracksPage,
} from "./types";

const ROOT: LibraryRootView = {
  rootId: "01995b60-d40f-7a01-a7c9-75f50fb92871",
  displayName: "Licensed Music",
  available: true,
};

const TRACKS: TracksPage = {
  items: [
    {
      trackId: "01995b60-d40f-7a01-a7c9-75f50fb92872",
      availability: "playable",
      durationMs: 181_000,
      artworkAvailable: true,
      original: { title: "本地曲名", artist: "本地艺术家", album: "本地专辑" },
      enriched: {
        title: "补全曲名",
        artist: "补全艺术家",
        album: "补全专辑",
        provider: "musicbrainz",
        confidence: 0.91,
        fetchedAt: "2026-09-03T01:02:03.000Z",
      },
      matchStatus: "matched",
    },
    {
      trackId: "01995b60-d40f-7a01-a7c9-75f50fb92873",
      availability: "missing",
      durationMs: 62_000,
      artworkAvailable: false,
      original: { title: "第二首", artist: null, album: null },
      enriched: null,
      matchStatus: "unmatched",
    },
  ],
  nextCursor: null,
};

class FakeLibraryIpc implements LibraryIpc {
  readonly requests: ListTracksRequest[] = [];
  pickCalls = 0;
  startCalls = 0;
  cancelCalls = 0;
  roots: ReadonlyArray<LibraryRootView> = [ROOT];
  page: TracksPage = TRACKS;
  listError: unknown = null;
  private handler: ((event: LibraryScanEvent) => void) | null = null;

  async listLibraryRoots() {
    return { roots: this.roots, revision: 1 };
  }

  async pickAndAddLibraryRoot(): Promise<PickLibraryRootView> {
    this.pickCalls += 1;
    return { requestId: "pick-request", root: ROOT, revision: 1 };
  }

  async listTracks(request: ListTracksRequest): Promise<TracksPage> {
    this.requests.push(request);
    if (this.listError !== null) throw this.listError;
    return this.page;
  }

  async startLibraryScan(rootIds: ReadonlyArray<string>): Promise<OperationAcceptedView> {
    this.startCalls += 1;
    expect(rootIds).toEqual([ROOT.rootId]);
    return {
      operationId: "01995b60-d40f-7a01-a7c9-75f50fb92874",
      acceptedAt: "2026-09-03T01:02:03.000Z",
    };
  }

  async cancelLibraryScan(operationId: string): Promise<CancelLibraryScanView> {
    this.cancelCalls += 1;
    return { requestId: "cancel-request", operationId, state: "cancelled" };
  }

  async subscribeLibraryScan(handler: (event: LibraryScanEvent) => void) {
    this.handler = handler;
    return () => { this.handler = null; };
  }

  emit(event: LibraryScanEvent) {
    this.handler?.(event);
  }
}

afterEach(() => cleanup());

describe("LibraryView / FR-LIB-002, FR-LIB-005, NFR-A11Y-001", () => {
  it("renders bounded loading and empty states without reading a path", async () => {
    const ipc = new FakeLibraryIpc();
    ipc.roots = [];
    ipc.page = { items: [], nextCursor: null };
    render(<LibraryView ipc={ipc} />);

    expect(screen.getByText("[LOADING…]")).toBeInTheDocument();
    expect(await screen.findByRole("heading", { name: "还没有本地曲库" })).toBeInTheDocument();
    expect(screen.getByText("暂无曲目。选择目录后扫描即可建立本地索引。")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "扫描曲库" })).toBeDisabled();
    expect(document.body.textContent).not.toContain("C:\\");
  });

  it("keeps search text while applying AND filters and a deterministic sort", async () => {
    const user = userEvent.setup();
    const ipc = new FakeLibraryIpc();
    render(<LibraryView ipc={ipc} />);
    await screen.findByRole("listbox", { name: "曲库曲目" });

    const search = screen.getByRole("searchbox", { name: /搜索曲库/ });
    await user.type(search, "  夜曲  ");
    await user.selectOptions(screen.getByLabelText("文件状态"), "playable");
    await user.selectOptions(screen.getByLabelText("匹配状态"), "review");
    await user.selectOptions(screen.getByLabelText("排序"), "artist");
    await user.click(screen.getByRole("button", { name: "搜索" }));

    await waitFor(() => {
      expect(ipc.requests.at(-1)).toEqual({
        cursor: null,
        limit: 50,
        query: "夜曲",
        sort: "artist",
        filters: { availability: "playable", matchStatus: "review" },
      });
    });
    expect(search).toHaveValue("  夜曲  ");
    expect(ipc.requests.every((request) => request.limit <= 200)).toBe(true);
  });

  it("supports Arrow navigation, Enter details, Escape return, and Ctrl+K", async () => {
    const user = userEvent.setup();
    const ipc = new FakeLibraryIpc();
    render(<LibraryView ipc={ipc} />);
    const list = await screen.findByRole("listbox", { name: "曲库曲目" });
    const rows = within(list).getAllByRole("option");
    rows[0]?.focus();
    await user.keyboard("{ArrowDown}");
    expect(rows[1]).toHaveFocus();
    await user.keyboard("{Enter}");
    expect(screen.getByText("LOCAL TAGS")).toBeInTheDocument();
    expect(screen.getByText(/MUSICBRAINZ MATCH/)).toBeInTheDocument();
    await user.keyboard("{Escape}");
    await waitFor(() => expect(rows[1]).toHaveFocus());

    await user.keyboard("{Control>}k{/Control}");
    expect(screen.getByRole("searchbox", { name: /搜索曲库/ })).toHaveFocus();
  });

  it("keeps only one bounded cursor page mounted while moving forward and back", async () => {
    const user = userEvent.setup();
    const ipc = new FakeLibraryIpc();
    ipc.page = { items: [TRACKS.items[0]!], nextCursor: "v1:50" };
    render(<LibraryView ipc={ipc} />);
    expect(await screen.findByText("补全曲名")).toBeInTheDocument();

    ipc.page = { items: [TRACKS.items[1]!], nextCursor: null };
    await user.click(screen.getByRole("button", { name: "下一页" }));
    expect(await screen.findByText("第二首")).toBeInTheDocument();
    expect(screen.queryByText("补全曲名")).not.toBeInTheDocument();
    expect(ipc.requests.at(-1)?.cursor).toBe("v1:50");

    ipc.page = { items: [TRACKS.items[0]!], nextCursor: "v1:50" };
    await user.click(screen.getByRole("button", { name: "上一页" }));
    expect(await screen.findByText("补全曲名")).toBeInTheDocument();
    expect(ipc.requests.at(-1)?.cursor).toBeNull();
  });

  it("starts and cancels only from explicit controls while offline metadata stays isolated", async () => {
    const user = userEvent.setup();
    const ipc = new FakeLibraryIpc();
    render(<LibraryView ipc={ipc} metadataState="offline" />);
    await screen.findByRole("listbox", { name: "曲库曲目" });
    expect(screen.getByText("[OFFLINE: METADATA PAUSED]")).toBeInTheDocument();
    expect(ipc.startCalls).toBe(0);

    await user.click(screen.getByRole("button", { name: "扫描曲库" }));
    expect(ipc.startCalls).toBe(1);
    await act(async () => {
      ipc.emit({
        schemaVersion: "1.0.0",
        sequence: 1,
        occurredAt: "2026-09-03T01:02:04.000Z",
        operationId: "01995b60-d40f-7a01-a7c9-75f50fb92874",
        state: "running",
        scanned: 8,
        discovered: 6,
        failed: 2,
        safeMessage: null,
      });
    });
    expect(screen.getByRole("heading", { name: "已处理 8 个文件" })).toBeInTheDocument();
    expect(screen.getByText("发现 6 · 已处理 8 · 错误 2")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "取消扫描" }));
    expect(ipc.cancelCalls).toBe(1);
    expect(screen.getByText("[SCAN INCOMPLETE]")).toBeInTheDocument();
  });

  it("uses a redacted inline error and disables unavailable controls", async () => {
    const ipc = new FakeLibraryIpc();
    ipc.listError = new Error("C:\\private-canary\\track.mp3");
    render(<LibraryView ipc={ipc} disabledReason="本机曲库能力不可用。" />);
    expect(await screen.findByRole("alert")).toHaveTextContent("无法读取本地曲库");
    expect(document.body.textContent).not.toContain("private-canary");
    expect(screen.getByRole("button", { name: "选择目录" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "扫描曲库" })).toBeDisabled();
    expect(screen.getByRole("searchbox", { name: /搜索曲库/ })).toBeDisabled();
    expect(ipc.pickCalls).toBe(0);
    expect(ipc.startCalls).toBe(0);
    expect(vi.isMockFunction(ipc.listTracks)).toBe(false);
  });

  it("has no automatic accessibility violations in the populated state", async () => {
    const ipc = new FakeLibraryIpc();
    const { container } = render(<LibraryView ipc={ipc} />);
    await screen.findByRole("listbox", { name: "曲库曲目" });
    const results = await axe.run(container, {
      rules: { "color-contrast": { enabled: false } },
    });
    expect(results.violations).toEqual([]);
  });
});
