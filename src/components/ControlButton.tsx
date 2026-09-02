import type { ButtonHTMLAttributes, ReactNode } from "react";

interface ControlButtonProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "disabled"> {
  readonly children: ReactNode;
  readonly disabledReason?: string;
  readonly tone?: "primary" | "secondary" | "danger" | "ghost";
}

export function ControlButton({
  children,
  disabledReason,
  tone = "secondary",
  onClick,
  ...buttonProps
}: ControlButtonProps) {
  const unavailable = disabledReason !== undefined;
  return (
    <span className="control-with-reason">
      <button
        {...buttonProps}
        type="button"
        className={`control control--${tone}`}
        aria-disabled={unavailable || undefined}
        onClick={(event) => {
          if (unavailable) {
            event.preventDefault();
            return;
          }
          onClick?.(event);
        }}
      >
        {children}
      </button>
      {unavailable ? <span className="control-reason">{disabledReason}</span> : null}
    </span>
  );
}
