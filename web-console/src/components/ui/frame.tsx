import type { HTMLAttributes, ReactNode } from "react";
import { cn } from "../../lib/utils.js";

type FrameProps = HTMLAttributes<HTMLElement> & {
  /** panel：L2 雾面区块容器（6px 圆角）；card：L3 高对比核心内容面（4px 圆角）；compact：轻量 L3 卡片。 */
  variant?: "panel" | "card" | "compact";
};

/** 材质容器：通过 L2 雾面面板与 L3 核心内容面建立“外透内实”层级，1px 发丝边框、无投影。 */
export function Frame({ variant = "card", className, ...props }: FrameProps) {
  const Tag = variant === "panel" ? "section" : "article";
  return (
    <Tag
      className={cn(
        "text-ink",
        variant === "panel" && "console-panel p-5 sm:p-6",
        variant === "card" && "console-paper p-4 sm:p-5",
        variant === "compact" && "console-paper p-3.5",
        className,
      )}
      {...props}
    />
  );
}

export type PageMetric = {
  readonly label: string;
  readonly value: ReactNode;
  readonly unit?: string;
};

type PageHeaderProps = {
  title: string;
  subtitle?: string | undefined;
  /** 右侧 1–2 个与当前页强相关的核心指标：数值用大号衬线体，单位用等宽字体。 */
  metrics?: readonly PageMetric[] | undefined;
  actions?: ReactNode | undefined;
  className?: string | undefined;
};

/** 杂志感页眉（Editorial Page Header）：左侧衬线主标题 + 一行中文副标题，右侧核心指标或操作。 */
export function PageHeader({ title, subtitle, metrics, actions, className }: PageHeaderProps) {
  return (
    <header className={cn("flex flex-wrap items-end justify-between gap-x-6 gap-y-4 border-b border-line pb-5", className)}>
      <div className="min-w-0">
        <h1 className="m-0 font-display text-[28px] leading-[1.2] font-semibold tracking-[-0.01em] text-ink sm:text-[32px]">
          {title}
        </h1>
        {subtitle ? (
          <p className="m-0 mt-1.5 max-w-[72ch] text-sm leading-[1.55] text-muted">{subtitle}</p>
        ) : null}
      </div>
      {(metrics && metrics.length > 0) || actions ? (
        <div className="flex flex-wrap items-end gap-6">
          {metrics?.map((metric) => (
            <div key={metric.label} className="flex flex-col items-end">
              <span className="text-xs font-medium text-muted">{metric.label}</span>
              <div className="mt-0.5 flex items-baseline gap-1.5">
                <span className="font-display text-[28px] leading-[1.08] font-semibold text-ink sm:text-[32px]">
                  {metric.value}
                </span>
                {metric.unit ? <span className="font-mono text-xs font-medium text-muted">{metric.unit}</span> : null}
              </div>
            </div>
          ))}
          {actions ? <div className="flex flex-wrap items-center gap-2">{actions}</div> : null}
        </div>
      ) : null}
    </header>
  );
}

type SectionHeadingProps = {
  id?: string;
  title: string;
  description?: string;
  aside?: ReactNode;
  className?: string;
};

/** 区块标题（h2 衬线体，18px–20px，不加装饰性英文眉题）。 */
export function SectionHeading({ id, title, description, aside, className }: SectionHeadingProps) {
  return (
    <div className={cn("flex flex-wrap items-baseline justify-between gap-3", className)}>
      <div className="min-w-0">
        <h2 id={id} className="m-0 font-display text-[19px] leading-[1.3] font-semibold text-ink">
          {title}
        </h2>
        {description ? <p className="m-0 mt-1 max-w-[72ch] text-xs leading-relaxed text-muted">{description}</p> : null}
      </div>
      {aside ? <div className="flex shrink-0 items-center gap-2">{aside}</div> : null}
    </div>
  );
}
