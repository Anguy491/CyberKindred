import {
  type FormEvent,
  type KeyboardEvent,
  type RefObject,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import type {
  LibraryIpc,
  LibraryRootView,
  LibraryScanEvent,
  ListTracksRequest,
  TrackAvailabilityFilter,
  TrackMatchStatus,
  TrackSort,
  TrackView,
  TracksPage,
} from "./types";
import "./library.css";

const PAGE_SIZE = 50;

export interface LibraryViewProps {
  readonly ipc: LibraryIpc;
  readonly metadataState?: "online" | "offline" | "disabled";
  readonly disabledReason?: string;
  readonly disabledLabel?: string;
  readonly searchRef?: RefObject<HTMLInputElement | null>;
}

interface ScanViewState {
  readonly operationId: string;
  readonly state: "accepted" | LibraryScanEvent["state"];
  readonly scanned: number;
  readonly discovered: number;
  readonly failed: number;
  readonly safeMessage: string | null;
}

export function LibraryView({
  ipc,
  metadataState = "online",
  disabledReason,
  disabledLabel = "[LIBRARY DISABLED]",
  searchRef,
}: LibraryViewProps) {
  const internalSearchRef = useRef<HTMLInputElement>(null);
  const activeSearchRef = searchRef ?? internalSearchRef;
  const rowRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const selectedRowRef = useRef<HTMLButtonElement | null>(null);
  const requestSerial = useRef(0);
  const [roots, setRoots] = useState<ReadonlyArray<LibraryRootView> | null>(null);
  const [page, setPage] = useState<TracksPage | null>(null);
  const [loading, setLoading] = useState(true);
  const [rootActionPending, setRootActionPending] = useState(false);
  const [error, setError] = useState(false);
  const [draftQuery, setDraftQuery] = useState("");
  const [query, setQuery] = useState("");
  const [sort, setSort] = useState<TrackSort>("title");
  const [availability, setAvailability] = useState<TrackAvailabilityFilter>(null);
  const [matchStatus, setMatchStatus] = useState<TrackMatchStatus | null>(null);
  const [pageCursors, setPageCursors] = useState<ReadonlyArray<string | null>>([null]);
  const [pageIndex, setPageIndex] = useState(0);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [detailTrack, setDetailTrack] = useState<TrackView | null>(null);
  const [scan, setScan] = useState<ScanViewState | null>(null);

  const requestFor = useCallback((cursor: string | null): ListTracksRequest => ({
    cursor,
    limit: PAGE_SIZE,
    query: query.length === 0 ? null : query,
    sort,
    filters: { availability, matchStatus },
  }), [availability, matchStatus, query, sort]);

  const loadPage = useCallback(async (cursor: string | null, index: number) => {
    const serial = requestSerial.current + 1;
    requestSerial.current = serial;
    setLoading(true);
    setError(false);
    try {
      const next = await ipc.listTracks(requestFor(cursor));
      if (requestSerial.current !== serial) return;
      setPage(next);
      setPageIndex(index);
      setSelectedIndex(0);
      setDetailTrack(null);
    } catch {
      if (requestSerial.current === serial) setError(true);
    } finally {
      if (requestSerial.current === serial) setLoading(false);
    }
  }, [ipc, requestFor]);

  const loadRoots = useCallback(async () => {
    try {
      const snapshot = await ipc.listLibraryRoots();
      setRoots(snapshot.roots);
    } catch {
      setRoots([]);
      setError(true);
    }
  }, [ipc]);

  useEffect(() => {
    void loadRoots();
  }, [loadRoots]);

  useEffect(() => {
    setPageCursors([null]);
    void loadPage(null, 0);
  }, [loadPage]);

  useEffect(() => {
    const focusSearch = (event: globalThis.KeyboardEvent) => {
      if (event.ctrlKey && event.key.toLowerCase() === "k") {
        event.preventDefault();
        activeSearchRef.current?.focus();
      }
    };
    window.addEventListener("keydown", focusSearch);
    return () => window.removeEventListener("keydown", focusSearch);
  }, [activeSearchRef]);

  useEffect(() => {
    let active = true;
    let unsubscribe: (() => void) | undefined;
    void ipc.subscribeLibraryScan(
      (event) => {
        if (!active) return;
        setScan((current) => {
          if (current !== null && current.operationId !== event.operationId) return current;
          return {
            operationId: event.operationId,
            state: event.state,
            scanned: event.scanned,
            discovered: event.discovered,
            failed: event.failed,
            safeMessage: event.safeMessage,
          };
        });
      },
      async () => {
        if (!active) return;
        await Promise.all([loadRoots(), loadPage(null, 0)]);
      },
    ).then((stop) => {
      if (active) unsubscribe = stop;
      else stop();
    }).catch(() => {
      if (active) setError(true);
    });
    return () => {
      active = false;
      unsubscribe?.();
    };
  }, [ipc, loadPage, loadRoots]);

  const activeRoots = useMemo(
    () => roots?.filter((root) => root.available) ?? [],
    [roots],
  );
  const scanActive = scan?.state === "accepted" || scan?.state === "running";

  const submitSearch = (event: FormEvent) => {
    event.preventDefault();
    const next = draftQuery.trim();
    setPageCursors([null]);
    if (next === query) void loadPage(null, 0);
    else setQuery(next);
  };

  const pickRoot = async () => {
    if (disabledReason !== undefined || rootActionPending) return;
    setRootActionPending(true);
    setError(false);
    try {
      const result = await ipc.pickAndAddLibraryRoot();
      if (result.root !== null) await loadRoots();
    } catch {
      setError(true);
    } finally {
      setRootActionPending(false);
    }
  };

  const startScan = async () => {
    if (disabledReason !== undefined || scanActive || activeRoots.length === 0) return;
    setError(false);
    try {
      const accepted = await ipc.startLibraryScan(activeRoots.map((root) => root.rootId));
      setScan({
        operationId: accepted.operationId,
        state: "accepted",
        scanned: 0,
        discovered: 0,
        failed: 0,
        safeMessage: null,
      });
    } catch {
      setError(true);
    }
  };

  const cancelScan = async () => {
    if (!scanActive || scan === null) return;
    try {
      const result = await ipc.cancelLibraryScan(scan.operationId);
      if (result.state === "cancelled") {
        setScan((current) => current === null ? null : { ...current, state: "cancelled" });
      }
    } catch {
      setError(true);
    }
  };

  const moveSelection = (index: number) => {
    const itemCount = page?.items.length ?? 0;
    if (itemCount === 0) return;
    const next = Math.max(0, Math.min(index, itemCount - 1));
    setSelectedIndex(next);
    rowRefs.current[next]?.focus();
  };

  const handleRowKey = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      moveSelection(index + 1);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      moveSelection(index - 1);
    } else if (event.key === "Enter") {
      event.preventDefault();
      selectedRowRef.current = event.currentTarget;
      setDetailTrack(page?.items[index] ?? null);
    }
  };

  const closeDetail = () => {
    setDetailTrack(null);
    queueMicrotask(() => selectedRowRef.current?.focus());
  };

  const primary = scanActive
    ? `已处理 ${scan.scanned} 个文件`
    : roots !== null && roots.length === 0
      ? "还没有本地曲库"
      : "你的本地曲库";

  return (
    <section className="library-view page" aria-labelledby="library-view-title" aria-busy={loading}>
      <div className="page-primary" aria-live="polite">
        <p className="instrument-label">
          {scanActive ? "[SCANNING…]" : "LIBRARY"}
        </p>
        <h1 id="library-view-title" className="hero-title">{primary}</h1>
        {scan !== null ? (
          <p className="library-scan-summary">
            发现 {scan.discovered} · 已处理 {scan.scanned} · 错误 {scan.failed}
          </p>
        ) : null}
        {scan?.safeMessage !== null && scan?.safeMessage !== undefined ? (
          <p className={scan.state === "failed" ? "status-error" : "inline-status"}>
            {scan.safeMessage}
          </p>
        ) : null}
      </div>

      <div className="page-secondary library-workbench">
        {metadataState === "offline" ? (
          <p className="inline-notice inline-notice--warning" role="status">
            <strong>[OFFLINE: METADATA PAUSED]</strong>
            <span>本地标签与曲库浏览仍可使用。</span>
          </p>
        ) : null}
        {metadataState === "disabled" ? (
          <p className="inline-notice" role="status">
            <strong>[METADATA DISABLED]</strong>
            <span>仅显示本地标签；可在设置中启用补全。</span>
          </p>
        ) : null}
        {disabledReason !== undefined ? (
          <p className="inline-notice inline-notice--warning" role="status">
            <strong>{disabledLabel}</strong><span>{disabledReason}</span>
          </p>
        ) : null}
        {error ? (
          <div className="inline-error" role="alert">
            <strong>[ERROR: LIBRARY]</strong>
            <span>无法读取本地曲库。已保持静音，请重试。</span>
            <button
              type="button"
              className="text-control"
              onClick={() => {
                void loadRoots();
                void loadPage(null, 0);
              }}
            >
              重新读取
            </button>
          </div>
        ) : null}

        <div className="library-actions" aria-label="曲库与扫描操作">
          <button
            type="button"
            className="control"
            disabled={disabledReason !== undefined || rootActionPending}
            onClick={() => void pickRoot()}
          >
            {rootActionPending ? "[WAITING FOR PICKER…]" : "选择目录"}
          </button>
          <button
            type="button"
            className="control control--primary"
            disabled={disabledReason !== undefined || scanActive || activeRoots.length === 0}
            onClick={() => void startScan()}
          >
            {scan?.state === "cancelled" ? "继续扫描" : "扫描曲库"}
          </button>
          {scanActive ? (
            <button type="button" className="control" onClick={() => void cancelScan()}>
              取消扫描
            </button>
          ) : null}
        </div>

        <form className="library-query" role="search" onSubmit={submitSearch}>
          <label className="search-field" htmlFor="library-feature-search">
            <span className="instrument-label">搜索曲库 / CTRL+K</span>
            <input
              id="library-feature-search"
              ref={activeSearchRef}
              type="search"
              value={draftQuery}
              placeholder="标题、艺术家或专辑"
              disabled={disabledReason !== undefined}
              onChange={(event) => setDraftQuery(event.target.value)}
            />
          </label>
          <button type="submit" className="control" disabled={disabledReason !== undefined}>搜索</button>
          <label>
            <span className="instrument-label">排序</span>
            <select
              value={sort}
              disabled={disabledReason !== undefined}
              onChange={(event) => setSort(event.target.value as TrackSort)}
            >
              <option value="title">标题</option>
              <option value="artist">艺术家</option>
              <option value="album">专辑</option>
              <option value="recent">最近更新</option>
            </select>
          </label>
          <label>
            <span className="instrument-label">文件状态</span>
            <select
              value={availability ?? ""}
              disabled={disabledReason !== undefined}
              onChange={(event) => setAvailability(
                event.target.value === "" ? null : event.target.value as TrackAvailabilityFilter,
              )}
            >
              <option value="">全部</option>
              <option value="playable">可播放</option>
              <option value="missing">缺失</option>
            </select>
          </label>
          <label>
            <span className="instrument-label">匹配状态</span>
            <select
              value={matchStatus ?? ""}
              disabled={disabledReason !== undefined}
              onChange={(event) => setMatchStatus(
                event.target.value === "" ? null : event.target.value as TrackMatchStatus,
              )}
            >
              <option value="">全部</option>
              <option value="matched">已匹配</option>
              <option value="unmatched">未匹配</option>
              <option value="review">待确认</option>
            </select>
          </label>
        </form>

        {loading && page === null ? <LibraryLoading /> : null}
        {!loading && page !== null && page.items.length === 0 ? (
          <p className="empty-detail">
            {query.length > 0 || availability !== null || matchStatus !== null
              ? "没有符合当前搜索和过滤条件的曲目。"
              : "暂无曲目。选择目录后扫描即可建立本地索引。"}
          </p>
        ) : null}
        {page !== null && page.items.length > 0 ? (
          <ul className="track-list" role="listbox" aria-label="曲库曲目">
            {page.items.map((track, index) => (
              <li key={track.trackId} role="none">
                <button
                  ref={(node) => { rowRefs.current[index] = node; }}
                  type="button"
                  role="option"
                  aria-selected={selectedIndex === index}
                  tabIndex={selectedIndex === index ? 0 : -1}
                  className="track-row"
                  onClick={(event) => {
                    setSelectedIndex(index);
                    selectedRowRef.current = event.currentTarget;
                    setDetailTrack(track);
                  }}
                  onKeyDown={(event) => handleRowKey(event, index)}
                >
                  <span className="track-title">{displayTag(track, "title")}</span>
                  <span>{displayTag(track, "artist")} — {displayTag(track, "album")}</span>
                  <span>{formatDuration(track.durationMs)}</span>
                  <span>{sourceLabel(track)} · {matchLabel(track.matchStatus)}</span>
                </button>
              </li>
            ))}
          </ul>
        ) : null}

        {page !== null && page.items.length > 0 ? (
          <nav className="library-pagination" aria-label="曲库分页">
            <button
              type="button"
              className="control"
              disabled={loading || pageIndex === 0}
              onClick={() => void loadPage(pageCursors[pageIndex - 1] ?? null, pageIndex - 1)}
            >
              上一页
            </button>
            <span className="instrument-label">PAGE {pageIndex + 1}</span>
            <button
              type="button"
              className="control"
              disabled={loading || page.nextCursor === null}
              onClick={() => {
                const cursor = page.nextCursor;
                if (cursor === null) return;
                const nextIndex = pageIndex + 1;
                setPageCursors((current) => [...current.slice(0, nextIndex), cursor]);
                void loadPage(cursor, nextIndex);
              }}
            >
              下一页
            </button>
          </nav>
        ) : null}
      </div>

      <aside className="page-tertiary library-detail" aria-label="曲库状态与曲目详情">
        {detailTrack === null ? (
          <dl className="instrument-list">
            <div><dt>ROOTS</dt><dd>{roots?.length ?? "…"}</dd></div>
            <div><dt>VISIBLE</dt><dd>{page?.items.length ?? "…"}</dd></div>
            <div><dt>SCAN</dt><dd>{scanLabel(scan)}</dd></div>
            <div><dt>METADATA</dt><dd>{metadataLabel(metadataState)}</dd></div>
          </dl>
        ) : (
          <TrackDetails track={detailTrack} onClose={closeDetail} />
        )}
      </aside>
    </section>
  );
}

function LibraryLoading() {
  return (
    <div className="library-loading" role="status">
      <p className="instrument-label">[LOADING…]</p>
      <p>正在读取本地曲库索引。</p>
    </div>
  );
}

function TrackDetails({ track, onClose }: { readonly track: TrackView; readonly onClose: () => void }) {
  useEffect(() => {
    const closeOnEscape = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [onClose]);
  return (
    <div className="track-details">
      <button type="button" className="text-control" onClick={onClose}>返回曲库</button>
      <TagSection label="LOCAL TAGS" tags={track.original} />
      {track.enriched === null ? (
        <p className="empty-detail">MUSICBRAINZ MATCH / 尚未获得</p>
      ) : (
        <section>
          <p className="instrument-label">MUSICBRAINZ MATCH</p>
          <TagSection tags={track.enriched} />
          <p className={track.matchStatus === "review" ? "status-warning" : "inline-status"}>
            {matchLabel(track.matchStatus)} · {Math.round(track.enriched.confidence * 100)}% · {formatFetchedAt(track.enriched.fetchedAt)}
          </p>
        </section>
      )}
      <p className="inline-status">{availabilityLabel(track.availability)} · {formatDuration(track.durationMs)}</p>
    </div>
  );
}

function TagSection({
  label,
  tags,
}: {
  readonly label?: string;
  readonly tags: { readonly title: string | null; readonly artist: string | null; readonly album: string | null };
}) {
  return (
    <section>
      {label === undefined ? null : <p className="instrument-label">{label}</p>}
      <dl className="instrument-list">
        <div><dt>标题</dt><dd>{tags.title ?? "尚未获得"}</dd></div>
        <div><dt>艺术家</dt><dd>{tags.artist ?? "尚未获得"}</dd></div>
        <div><dt>专辑</dt><dd>{tags.album ?? "尚未获得"}</dd></div>
      </dl>
    </section>
  );
}

function displayTag(track: TrackView, field: keyof TrackView["original"]): string {
  return track.enriched?.[field] ?? track.original[field] ?? "尚未获得";
}

function sourceLabel(track: TrackView): string {
  return track.enriched === null ? "LOCAL TAGS" : "MUSICBRAINZ";
}

function matchLabel(status: TrackMatchStatus): string {
  if (status === "matched") return "[MATCHED]";
  if (status === "review") return "[REVIEW]";
  return "[UNMATCHED]";
}

function availabilityLabel(status: TrackView["availability"]): string {
  if (status === "playable") return "[PLAYABLE]";
  if (status === "missing") return "[MISSING]";
  if (status === "corrupt") return "[CORRUPT]";
  return "[UNSUPPORTED]";
}

function scanLabel(scan: ScanViewState | null): string {
  if (scan === null) return "[NOT STARTED]";
  if (scan.state === "accepted") return "[ACCEPTED]";
  if (scan.state === "running") return "[SCANNING…]";
  if (scan.state === "completed") return "[COMPLETE]";
  if (scan.state === "cancelled") return "[SCAN INCOMPLETE]";
  return "[FAILED]";
}

function metadataLabel(state: NonNullable<LibraryViewProps["metadataState"]>): string {
  if (state === "online") return "[READY]";
  if (state === "offline") return "[OFFLINE]";
  return "[DISABLED]";
}

function formatDuration(durationMs: number): string {
  const totalSeconds = Math.max(0, Math.floor(durationMs / 1_000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

function formatFetchedAt(value: string): string {
  const parsed = new Date(value);
  if (Number.isNaN(parsed.valueOf())) return "来源时间尚未获得";
  return `来源 ${parsed.toLocaleDateString("zh-CN")}`;
}
