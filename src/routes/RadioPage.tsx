import type { RefObject } from "react";

import { RadioView, type RadioIpc } from "../features/radio";

interface RadioPageProps {
  readonly ipc: RadioIpc;
  readonly startButtonRef?: RefObject<HTMLButtonElement | null>;
  readonly messageInputRef?: RefObject<HTMLTextAreaElement | null>;
  readonly autoFocusStart?: boolean;
  readonly onStartFocused?: () => void;
}

export function RadioPage({ ipc, startButtonRef, messageInputRef, autoFocusStart, onStartFocused }: RadioPageProps) {
  return <RadioView ipc={ipc}
    {...(startButtonRef === undefined ? {} : { startButtonRef })}
    {...(messageInputRef === undefined ? {} : { messageInputRef })}
    {...(autoFocusStart === undefined ? {} : { autoFocusStart })}
    {...(onStartFocused === undefined ? {} : { onStartFocused })} />;
}
