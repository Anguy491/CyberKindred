import type { RefObject } from "react";

import { RadioView, type RadioIpc } from "../features/radio";

interface RadioPageProps {
  readonly ipc: RadioIpc;
  readonly startButtonRef?: RefObject<HTMLButtonElement | null>;
  readonly autoFocusStart?: boolean;
  readonly onStartFocused?: () => void;
}

export function RadioPage({ ipc, startButtonRef, autoFocusStart, onStartFocused }: RadioPageProps) {
  return <RadioView ipc={ipc}
    {...(startButtonRef === undefined ? {} : { startButtonRef })}
    {...(autoFocusStart === undefined ? {} : { autoFocusStart })}
    {...(onStartFocused === undefined ? {} : { onStartFocused })} />;
}
