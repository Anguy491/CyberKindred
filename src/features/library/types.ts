import type {
  CancelLibraryScanResponse,
  EnrichedTrackTagView as IpcEnrichedTrackTagView,
  LibraryRoot,
  LibraryRootsResponse,
  ListTracksRequest as IpcListTracksRequest,
  OperationAccepted,
  PickLibraryRootResponse,
  TrackAvailabilityFilter as IpcTrackAvailabilityFilter,
  TrackFilters as IpcTrackFilters,
  TrackMatchStatus as IpcTrackMatchStatus,
  TrackSort as IpcTrackSort,
  TrackTagView as IpcTrackTagView,
  TrackView as IpcTrackView,
  TracksPage as IpcTracksPage,
} from "../../ipc";

export type TrackSort = IpcTrackSort;
export type TrackAvailabilityFilter = IpcTrackAvailabilityFilter | null;
export type TrackMatchStatus = IpcTrackMatchStatus;
export type TrackAvailability = IpcTrackView["availability"];
export type TrackFilters = IpcTrackFilters;
export type ListTracksRequest = IpcListTracksRequest;
export type TrackTagView = IpcTrackTagView;
export type EnrichedTrackTagView = IpcEnrichedTrackTagView;
export type TrackView = IpcTrackView;
export type TracksPage = IpcTracksPage;
export type LibraryRootView = LibraryRoot;
export type LibraryRootsView = LibraryRootsResponse;
export type PickLibraryRootView = PickLibraryRootResponse;
export type OperationAcceptedView = OperationAccepted;
export type CancelLibraryScanView = CancelLibraryScanResponse;

export type LibraryScanState = "running" | "completed" | "cancelled" | "failed";

export interface LibraryScanEvent {
  readonly schemaVersion: "1.0.0";
  readonly sequence: number;
  readonly occurredAt: string;
  readonly operationId: string;
  readonly state: LibraryScanState;
  readonly scanned: number;
  readonly discovered: number;
  readonly failed: number;
  readonly safeMessage: string | null;
}

/** The feature receives this capability; it never imports Tauri or reads files directly. */
export interface LibraryIpc {
  listLibraryRoots(): Promise<LibraryRootsView>;
  pickAndAddLibraryRoot(): Promise<PickLibraryRootView>;
  listTracks(request: ListTracksRequest): Promise<TracksPage>;
  startLibraryScan(rootIds: ReadonlyArray<string>): Promise<OperationAcceptedView>;
  cancelLibraryScan(operationId: string): Promise<CancelLibraryScanView>;
  subscribeLibraryScan(
    handler: (event: LibraryScanEvent) => void,
    refreshSnapshot: () => Promise<void>,
  ): Promise<() => void>;
}
