import type { InputHTMLAttributes } from "react";
import { cn } from "../../lib/utils.js";

type SwitchProps = Omit<InputHTMLAttributes<HTMLInputElement>, "type"> & {
  className?: string;
};

/** 轻量开关：原生 checkbox + peer 样式；120ms 静默过渡，disabled 与 aria 语义与原生一致。 */
export function Switch({ className, ...props }: SwitchProps) {
  return (
    <label
      className={cn(
        "relative inline-flex h-5 w-9 shrink-0 items-center",
        props.disabled ? "cursor-not-allowed opacity-55" : "cursor-pointer",
        className,
      )}
    >
      <input type="checkbox" role="switch" className="peer sr-only" {...props} />
      <span
        aria-hidden
        className="pointer-events-none absolute inset-0 rounded-full border border-line bg-input transition-colors duration-[120ms] ease-out peer-checked:border-accent peer-checked:bg-accent"
      />
      <span
        aria-hidden
        className="pointer-events-none absolute top-0.5 left-0.5 h-3.5 w-3.5 rounded-full bg-muted transition-transform duration-[120ms] ease-out peer-checked:translate-x-4 peer-checked:bg-accent-contrast"
      />
    </label>
  );
}
