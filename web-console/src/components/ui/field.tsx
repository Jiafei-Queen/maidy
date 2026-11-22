import type { InputHTMLAttributes, ReactNode, SelectHTMLAttributes, TextareaHTMLAttributes } from "react";
import { cn } from "../../lib/utils.js";

type FieldControlProps = { id: string; "aria-describedby"?: string | undefined; "aria-invalid"?: true | undefined };

type FieldProps = {
  label: string;
  /** label 的 for 绑定;不传 id 时必须传。 */
  id: string;
  hint?: ReactNode | undefined;
  error?: string | null | undefined;
  /** stacked：标签在上；row：桌面设置面板的“左标签说明 / 右控件”行结构，窄屏自动回落为上下排列。 */
  layout?: "stacked" | "row";
  /** row 布局右侧的附加操作（如“恢复默认”），与控件同列显示。 */
  aside?: ReactNode;
  children: (props: FieldControlProps) => ReactNode;
};

/**
 * 表单字段包装：label + 控件 + hint/错误文本，
 * 通过 render prop 把 aria 关联注入控件，保持无障碍契约。
 */
export function Field({ label, id, hint, error, layout = "stacked", aside, children }: FieldProps) {
  const hintId = hint !== undefined && hint !== null && hint !== "" ? `${id}-hint` : undefined;
  const errorId = error ? `${id}-error` : undefined;
  const describedBy = [hintId, errorId].filter(Boolean).join(" ") || undefined;
  const control = children({ id, "aria-describedby": describedBy, "aria-invalid": error ? true : undefined });
  const errorText = error ? (
    <p id={errorId} role="alert" className="m-0 text-xs font-medium text-error">
      {error}
    </p>
  ) : null;

  if (layout === "row") {
    return (
      <div className="grid gap-x-6 gap-y-2 py-3.5 md:grid-cols-[minmax(0,15rem)_minmax(0,1fr)] md:items-start">
        <div className="min-w-0 md:pt-1.5">
          <label htmlFor={id} className="text-[13px] font-semibold text-ink">
            {label}
          </label>
          {hintId ? (
            <p id={hintId} className="m-0 mt-0.5 text-xs leading-relaxed text-muted">
              {hint}
            </p>
          ) : null}
        </div>
        <div className="flex min-w-0 flex-col gap-1.5">
          {control}
          {aside}
          {errorText}
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-1.5">
      <label htmlFor={id} className="text-[13px] font-semibold text-ink">
        {label}
      </label>
      {control}
      {hintId ? (
        <p id={hintId} className="m-0 text-xs leading-relaxed text-muted">
          {hint}
        </p>
      ) : null}
      {errorText}
    </div>
  );
}

/** 输入类控件共享的几何与焦点反馈：L3 内容面、2px 圆角、1px 发丝线。 */
export const CONTROL_CLASS = cn(
  "min-h-8.5 rounded-console-sm border border-line bg-input px-2.5 py-1.5 text-[13px] text-ink outline-none",
  "placeholder:text-muted transition-colors duration-[120ms] ease-out",
  "hover:border-[color-mix(in_srgb,var(--console-text-secondary)_40%,transparent)] focus-visible:border-accent",
  "disabled:cursor-not-allowed disabled:opacity-60",
);

type InputProps = InputHTMLAttributes<HTMLInputElement>;

/** 文本输入：错误态红色边框。 */
export function Input({ className, ...props }: InputProps) {
  return <input className={cn(CONTROL_CLASS, props["aria-invalid"] && "border-error", className)} {...props} />;
}

/** 原生下拉：保留原生可访问性，只统一外观。 */
export function Select({ className, ...props }: SelectHTMLAttributes<HTMLSelectElement>) {
  return <select className={cn("console-select text-[13px] disabled:cursor-not-allowed disabled:opacity-60 focus-visible:border-accent", className)} {...props} />;
}

/** 多行文本：等宽场景由调用方追加 font-mono。 */
export function Textarea({ className, ...props }: TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return <textarea className={cn(CONTROL_CLASS, "min-h-24 leading-relaxed", props["aria-invalid"] && "border-error", className)} {...props} />;
}
