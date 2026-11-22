import { cn } from "../../lib/utils.js";
import { stateLabel } from "../../lib/format.js";

export type SignalTone = "success" | "warning" | "error" | "neutral";

/** 语义状态 → 几何信号（成功方形、警告菱形、中性圆形、错误三角形），禁止只靠颜色区分。 */
const TONE_SHAPES: Readonly<Record<SignalTone, string>> = {
  success: "■",
  warning: "◆",
  neutral: "●",
  error: "▲",
};

const TONE_CLASSES: Readonly<Record<SignalTone, string>> = {
  success: "border-success/35 bg-success-soft text-success",
  warning: "border-warning/35 bg-warning-soft text-warning",
  neutral: "border-line bg-card text-muted",
  error: "border-error/35 bg-error-soft text-error",
};

type StatusBadgeProps = {
  tone: SignalTone;
  /** 状态文本；不传时由 value 查中文标签。 */
  label?: string;
  /** 原始英文状态值（用于未知值回显），或直接传 label。 */
  value?: string;
  className?: string;
};

/** 状态徽章：2px 微圆角 + 1px 发丝线 + 几何形状与文字标签。 */
export function StatusBadge({ tone, label, value, className }: StatusBadgeProps) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-console-sm border px-2 py-0.5 font-mono text-xs leading-[1.35] font-semibold whitespace-nowrap",
        TONE_CLASSES[tone],
        className,
      )}
    >
      <span aria-hidden="true" className="text-[9px] leading-none">
        {TONE_SHAPES[tone]}
      </span>
      <span>{label ?? stateLabel(value ?? "unknown")}</span>
    </span>
  );
}

/** 已知语义状态值 → tone 的保守映射；未知值一律中性，避免误导。 */
export function toneForState(value: string): SignalTone {
  switch (value) {
    case "online":
    case "available":
    case "supported":
    case "ready":
      return "success";
    case "warning":
    case "unverified":
    case "not_configured":
    case "degraded":
      return "warning";
    case "offline":
    case "error":
    case "unsupported":
    case "not_found":
    case "permission_denied":
    case "invalid_path":
    case "invalid_path_type":
    case "unsupported_path_type":
    case "io_error":
      return "error";
    default:
      return "neutral";
  }
}
