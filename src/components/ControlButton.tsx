import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from "react";

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
  return (
    <span className="control-with-reason">
      <button
        ref={ref}
        {...buttonProps}
        type="button"
        className={`control control--${tone}`}
        disabled={disabled}
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
});
