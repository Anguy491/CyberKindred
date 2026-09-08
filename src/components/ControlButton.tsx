import { forwardRef, useId, type ButtonHTMLAttributes, type ReactNode } from "react";

interface ControlButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  readonly children: ReactNode;
  readonly disabledReason?: string;
  readonly tone?: "primary" | "secondary" | "danger" | "ghost";
}

export const ControlButton = forwardRef<HTMLButtonElement, ControlButtonProps>(function ControlButton({
  children,
  disabledReason,
  tone = "secondary",
  disabled,
  onClick,
  ...buttonProps
}: ControlButtonProps, ref) {
  const unavailable = disabledReason !== undefined;
  const reasonId = useId();
  const describedBy = unavailable
    ? [buttonProps["aria-describedby"], reasonId].filter(Boolean).join(" ")
    : buttonProps["aria-describedby"];
  return (
    <span className="control-with-reason">
      <button
        ref={ref}
        {...buttonProps}
        type="button"
        className={`control control--${tone}`}
        disabled={disabled}
        aria-disabled={unavailable || undefined}
        aria-describedby={describedBy || undefined}
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
      {unavailable ? <span id={reasonId} className="control-reason">{disabledReason}</span> : null}
    </span>
  );
});
