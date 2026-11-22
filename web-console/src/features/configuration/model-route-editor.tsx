import { useState } from "react";
import { Input } from "../../components/ui/field.js";
import { StatusBadge } from "../../components/ui/status-badge.js";
import type { CandidateHealth } from "./route-health.js";

/**
 * 模型候选路线 Chip 编辑器。
 *
 * 从旧版 views/configuration/model-route-editor.ts 平移：视觉上以独立 Chip 展示每个
 * provider:model 值（完整文本、删除、拖动/上移下移排序），候选列表作为受控状态由
 * Agent 草稿持有，保存契约仍是 set_model_route，值语义与旧版完全一致。
 */

/** 归一化候选列表：去首尾空白、过滤空值、按精确匹配去重，保持顺序。 */
export function normalizeCandidates(values: readonly string[]): string[] {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const raw of values) {
    const value = raw.trim();
    if (!value) continue;
    if (seen.has(value)) continue;
    seen.add(value);
    result.push(value);
  }
  return result;
}

/** 候选格式校验：必须包含冒号，且 provider 与 model 均非空。 */
export function isMalformedCandidate(value: string): boolean {
  const trimmed = value.trim();
  if (!trimmed) return true;
  const colon = trimmed.indexOf(":");
  if (colon <= 0 || colon === trimmed.length - 1) return true;
  return false;
}

export interface AddCandidateResult {
  list: string[];
  error: string | null;
}

/** 追加候选到末尾；空值、重复、非法格式返回明确错误。 */
export function addCandidate(list: readonly string[], candidate: string, entityLabel = "模型"): AddCandidateResult {
  const value = candidate.trim();
  if (!value) return { list: [...list], error: `${entityLabel}不能为空` };
  if (isMalformedCandidate(value)) return { list: [...list], error: "格式应为 provider:model" };
  if (list.some((item) => item === value)) return { list: [...list], error: `该${entityLabel}已在路线中` };
  return { list: [...list, value], error: null };
}

/** 删除指定位置的候选。 */
export function removeCandidate(list: readonly string[], index: number): string[] {
  if (index < 0 || index >= list.length) return [...list];
  return [...list.slice(0, index), ...list.slice(index + 1)];
}

/** 将候选从 from 移动到 to（越界时收敛到边界），保持其余顺序。 */
export function moveCandidate(list: readonly string[], from: number, to: number): string[] {
  if (from < 0 || from >= list.length) return [...list];
  const target = Math.max(0, Math.min(to, list.length - 1));
  if (from === target) return [...list];
  const next = [...list];
  const moved = next.splice(from, 1)[0];
  if (moved === undefined) return [...list];
  next.splice(target, 0, moved);
  return next;
}

export interface ModelRouteQuickAdd {
  label: string;
  value: string;
  /** 已在链路中时展示的文案，默认“已添加”。 */
  addedLabel?: string;
}

/** 快捷候选只追加一次；已存在时保持原顺序，避免重复点击产生重复项。 */
export function appendQuickAddCandidate(list: readonly string[], value: string): string[] {
  return list.includes(value) ? [...list] : [...list, value];
}

export interface ModelRouteEditorProps {
  label: string;
  candidates: readonly string[];
  disabled: boolean;
  onChange: (candidates: string[]) => void;
  /** 候选实体的中文名，默认“模型”；转录路由使用“转录模型”。 */
  entityLabel?: string;
  /** 常用候选快捷添加；已存在时按钮禁用，不会重复写入。 */
  quickAdd?: ModelRouteQuickAdd;
  /** 实时诊断单个候选可用性；不可用时在 Chip 上展示警告徽标与原因。 */
  diagnoseCandidate?: (candidate: string) => CandidateHealth;
  /** 该路线当前是否处于活跃业务路径；非活跃路径仅标记单项，不触发整条无可用严重警告。 */
  routeActive?: boolean;
}

/** Chip 编辑器：排序即时生效（上移/下移/拖动），删除与添加同样即时；错误经 aria-live 播报。 */
export function ModelRouteEditor({
  label,
  candidates,
  disabled,
  onChange,
  entityLabel = "模型",
  quickAdd,
  diagnoseCandidate,
  routeActive = true,
}: ModelRouteEditorProps) {
  const [addValue, setAddValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [dragFrom, setDragFrom] = useState<number | null>(null);
  const [dragOverIndex, setDragOverIndex] = useState<number | null>(null);

  const healthList = diagnoseCandidate ? candidates.map((candidate) => diagnoseCandidate(candidate)) : null;
  const unavailableCount = healthList ? healthList.filter((item) => !item.available).length : 0;
  const allUnavailable = routeActive && candidates.length > 0 && healthList !== null && unavailableCount === candidates.length;

  const tryAdd = (): void => {
    const result = addCandidate(candidates, addValue, entityLabel);
    if (result.error) {
      setError(result.error);
      return;
    }
    setError(null);
    setAddValue("");
    onChange(result.list);
  };

  const quickAddPresent = quickAdd ? candidates.includes(quickAdd.value) : false;
  const tryQuickAdd = (): void => {
    if (!quickAdd || quickAddPresent) return;
    setError(null);
    onChange(appendQuickAddCandidate(candidates, quickAdd.value));
  };

  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex flex-wrap items-center gap-2">
        <p className="m-0 text-sm font-semibold text-ink">{label}</p>
        {allUnavailable ? (
          <StatusBadge tone="error" label="无可用候选" />
        ) : unavailableCount > 0 ? (
          <StatusBadge tone="warning" label={`含 ${unavailableCount} 个不可用候选`} />
        ) : null}
      </div>
      <div role="list" aria-label={`${label}候选${entityLabel}`} className="flex flex-wrap gap-1.5">
        {candidates.map((candidate, index) => {
          const health = healthList?.[index];
          const chipToneClass =
            health && !health.available
              ? health.severity === "error"
                ? "border-error/60 bg-error-soft/60 text-ink"
                : "border-warning/60 bg-warning-soft/60 text-ink"
              : "border-line bg-glass-muted text-ink";
          return (
            <span
              key={candidate}
              role="listitem"
              aria-label={
                health?.message
                  ? `候选 ${candidate}，${health.message}，拖动或使用上移/下移调整顺序`
                  : `候选 ${candidate}，拖动或使用上移/下移调整顺序`
              }
              title={health?.message ?? undefined}
              draggable={!disabled}
              onDragStart={() => setDragFrom(index)}
              onDragEnd={() => {
                setDragFrom(null);
                setDragOverIndex(null);
              }}
              onDragOver={(event) => {
                event.preventDefault();
                setDragOverIndex(index);
              }}
              onDragLeave={() => setDragOverIndex((current) => (current === index ? null : current))}
              onDrop={(event) => {
                event.preventDefault();
                setDragOverIndex(null);
                if (dragFrom !== null && dragFrom !== index) {
                  onChange(moveCandidate(candidates, dragFrom, index));
                }
                setDragFrom(null);
              }}
              className={`flex items-center gap-1 border px-2 py-1 text-xs font-mono ${chipToneClass} ${
                dragOverIndex === index && dragFrom !== null && dragFrom !== index ? "border-accent" : ""
              } ${disabled ? "opacity-70" : ""}`}
            >
              <span aria-hidden="true" className={disabled ? "hidden" : "text-muted"}>≡</span>
              <span>{candidate}</span>
              {health?.badge ? (
                <span
                  className={`rounded-console-sm px-1 py-0.25 font-sans text-[10px] font-semibold ${
                    health.severity === "error" ? "bg-error/15 text-error" : "bg-warning/15 text-warning"
                  }`}
                >
                  {health.badge}
                </span>
              ) : null}
              {disabled ? null : (
                <>
                  <button
                    type="button"
                    aria-label={`上移 ${candidate}`}
                    onClick={() => onChange(moveCandidate(candidates, index, index - 1))}
                    className="px-0.5 text-muted hover:text-accent"
                  >
                    ↑
                  </button>
                  <button
                    type="button"
                    aria-label={`下移 ${candidate}`}
                    onClick={() => onChange(moveCandidate(candidates, index, index + 1))}
                    className="px-0.5 text-muted hover:text-accent"
                  >
                    ↓
                  </button>
                  <button
                    type="button"
                    aria-label={`删除 ${candidate}`}
                    onClick={() => onChange(removeCandidate(candidates, index))}
                    className="px-0.5 text-muted hover:text-error"
                  >
                    ×
                  </button>
                </>
              )}
            </span>
          );
        })}
        {candidates.length === 0 ? (
          <span className="text-xs text-muted">暂无候选{entityLabel}</span>
        ) : null}
      </div>
      {allUnavailable ? (
        <p role="alert" className="m-0 text-xs font-medium text-error">
          当前路线所有候选均不可用，运行时无法调用模型；请启用对应供应商、配置 API Key 或添加可用模型。
        </p>
      ) : unavailableCount > 0 ? (
        <p className="m-0 text-xs text-warning">
          标黄/红的候选在运行时会被自动跳过，悬停可查看具体原因。
        </p>
      ) : null}
      <div className="flex flex-wrap gap-2">
        <Input
          type="text"
          placeholder="provider:model"
          disabled={disabled}
          aria-label={`为 ${label} 添加候选${entityLabel}`}
          value={addValue}
          onChange={(event) => setAddValue(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              tryAdd();
            }
          }}
          className="max-w-64 flex-1 py-1.5 text-sm"
        />
        <button
          type="button"
          disabled={disabled}
          onClick={tryAdd}
          className="rounded-console border border-line bg-glass px-3 py-1.5 text-xs font-bold text-ink transition-colors hover:bg-accent-soft disabled:opacity-50"
        >
          添加
        </button>
        {quickAdd ? (
          <button
            type="button"
            disabled={disabled || quickAddPresent}
            onClick={tryQuickAdd}
            title={quickAddPresent ? `${quickAdd.value} 已在当前链路中` : `追加 ${quickAdd.value}`}
            aria-label={
              quickAddPresent
                ? `${quickAdd.label}已添加，无需重复添加`
                : `${quickAdd.label}，追加 ${quickAdd.value}`
            }
            className="rounded-console border border-line bg-glass-muted px-3 py-1.5 text-xs font-bold text-ink transition-colors hover:bg-accent-soft disabled:opacity-50"
          >
            {quickAddPresent ? quickAdd.addedLabel ?? "已添加" : quickAdd.label}
          </button>
        ) : null}
      </div>
      <p role="status" aria-live="polite" className="m-0 min-h-4 text-xs text-error">
        {error}
      </p>
    </div>
  );
}
