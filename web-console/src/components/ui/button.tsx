import type { ButtonHTMLAttributes } from "react";
import { cn } from "../../lib/utils.js";

type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";
type ButtonSize = "sm" | "md";

type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: ButtonVariant;
  size?: ButtonSize;
};

const VARIANT_CLASSES: Readonly<Record<ButtonVariant, string>> = {
  primary: "border-transparent bg-accent text-accent-contrast hover:bg-accent-strong",
  secondary: "border-line bg-card text-ink hover:border-accent/40 hover:bg-accent-soft",
  ghost: "border-transparent bg-transparent text-muted hover:bg-accent-soft hover:text-ink",
  // 危险操作保持描边形态，避免大面积红色；悬停时才填充。
  danger: "border-error/45 bg-transparent text-error hover:bg-error hover:text-error-contrast",
};

const SIZE_CLASSES: Readonly<Record<ButtonSize, string>> = {
  sm: "h-7 px-2.5 text-xs",
  md: "h-8.5 px-3.5 text-[13px]",
};

/** 控制台按钮：2px 微圆角、1px 发丝边框，仅做 120ms 色值过渡；disabled 表示不可用或请求进行中。 */
export function Button({ variant = "primary", size = "md", className, type = "button", ...props }: ButtonProps) {
  return (
    <button
      type={type}
      className={cn(
        "inline-flex shrink-0 cursor-pointer items-center justify-center gap-1.5 rounded-console-sm border font-semibold whitespace-nowrap",
        "transition-colors duration-[120ms] ease-out disabled:cursor-not-allowed disabled:opacity-50",
        SIZE_CLASSES[size],
        VARIANT_CLASSES[variant],
        className,
      )}
      {...props}
    />
  );
}
