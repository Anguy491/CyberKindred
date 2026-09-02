import type { RefObject } from "react";

import { LibraryView, type LibraryIpc } from "../features/library";

interface LibraryPageProps {
  readonly disabledReason?: string;
  readonly ipc: LibraryIpc;
  readonly metadataState: "online" | "offline" | "disabled";
  readonly searchRef: RefObject<HTMLInputElement | null>;
}

export function LibraryPage({
  disabledReason,
  ipc,
  metadataState,
  searchRef,
}: LibraryPageProps) {
  return (
    <LibraryView
      disabledLabel="[ERROR: PERMISSION DENIED]"
      {...(disabledReason === undefined ? {} : { disabledReason })}
      ipc={ipc}
      metadataState={metadataState}
      searchRef={searchRef}
    />
  );
}
