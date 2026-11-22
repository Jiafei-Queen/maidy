import { useMemo, useState } from "react";
import { Button } from "../../components/ui/button.js";
import { Input } from "../../components/ui/field.js";
import { FormDialog } from "../../components/ui/form-dialog.js";
import { CAPABILITY_KEYS, CAPABILITY_LABELS } from "./model-override-form.js";
import {
  discoveryLabel,
  filterModels,
  groupModelsBySeries,
  type ManagedModel,
} from "./provider-model-data.js";

const SOURCE_LABELS: Record<string, string> = {
  "connection discovery": "已发现",
  catalog: "目录",
  "local override": "本地",
};

type ModelSelectionDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  connection: string;
  allRows: readonly ManagedModel[];
  selectionConfigured: boolean;
  initialSelectedIds: readonly string[];
  activeDiscovery: Record<string, unknown> | null;
  providerEnabled: boolean;
  syncPending: boolean;
  savePending: boolean;
  saveError: string;
  onSync: () => void;
  onSave: (selectedIds: string[]) => void;
  onResetToAll: () => void;
};

/** 供应商模型批量选择弹窗：
 * 集中展示上游同步模型 + 目录参考模型 + 本地自定义模型，按系列分组支持一键勾选与批量保存精选白名单。 */
export function ModelSelectionDialog({
  open,
  onOpenChange,
  connection,
  allRows,
  selectionConfigured,
  initialSelectedIds,
  activeDiscovery,
  providerEnabled,
  syncPending,
  savePending,
  saveError,
  onSync,
  onSave,
  onResetToAll,
}: ModelSelectionDialogProps) {
  const [search, setSearch] = useState("");
  const [seriesFilter, setSeriesFilter] = useState("all");
  const [draftSelected, setDraftSelected] = useState<Set<string>>(() => new Set(
    selectionConfigured
      ? initialSelectedIds
      : allRows.filter((row) => row.metadata.enabled !== false).map((row) => row.id),
  ));

  const allGroups = useMemo(() => groupModelsBySeries(allRows), [allRows]);
  const filteredRows = useMemo(() => {
    const matched = filterModels([...allRows], search, "all");
    if (seriesFilter === "all") return matched;
    const targetGroup = allGroups.find((group) => group.series === seriesFilter);
    if (!targetGroup) return matched;
    const allowed = new Set(targetGroup.items.map((item) => item.id));
    return matched.filter((row) => allowed.has(row.id));
  }, [allGroups, allRows, search, seriesFilter]);

  const filteredGroups = useMemo(() => groupModelsBySeries(filteredRows), [filteredRows]);
  const discoveredIds = useMemo(
    () => allRows.filter((row) => row.sources.includes("connection discovery")).map((row) => row.id),
    [allRows],
  );

  const toggleOne = (id: string) => {
    setDraftSelected((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const selectFiltered = () => {
    setDraftSelected((current) => {
      const next = new Set(current);
      for (const row of filteredRows) next.add(row.id);
      return next;
    });
  };

  const deselectFiltered = () => {
    setDraftSelected((current) => {
      const next = new Set(current);
      for (const row of filteredRows) next.delete(row.id);
      return next;
    });
  };

  const toggleGroup = (items: readonly ManagedModel[]) => {
    const allChecked = items.every((item) => draftSelected.has(item.id));
    setDraftSelected((current) => {
      const next = new Set(current);
      for (const item of items) {
        if (allChecked) next.delete(item.id);
        else next.add(item.id);
      }
      return next;
    });
  };

  const orderedSelection = (): string[] => {
    // 优先按当前候选列表排序输出，保留此前手动选择的额外 ID（例如尚未同步出来的私有模型）。
    const ordered: string[] = [];
    const included = new Set<string>();
    for (const row of allRows) {
      if (draftSelected.has(row.id)) {
        ordered.push(row.id);
        included.add(row.id);
      }
    }
    for (const id of draftSelected) {
      if (!included.has(id)) ordered.push(id);
    }
    return ordered;
  };

  return (
    <FormDialog
      open={open}
      onOpenChange={onOpenChange}
      title={`管理模型 · ${connection}`}
      description="勾选你希望保留在主列表和路由快捷选项中的模型。保存精选列表不会删除或中断既有路由配置，可随时重新勾选调整。"
      wide
      busy={savePending}
      footer={(
        <div className="flex flex-wrap items-center gap-2">
          <Button
            disabled={savePending}
            onClick={() => onSave(orderedSelection())}
          >
            {savePending ? "保存中…" : `保存精选模型（${draftSelected.size}）`}
          </Button>
          {selectionConfigured ? (
            <Button
              variant="secondary"
              disabled={savePending}
              onClick={onResetToAll}
            >
              恢复显示全部
            </Button>
          ) : null}
          {saveError ? <span role="alert" className="text-xs font-semibold text-error">{saveError}</span> : null}
        </div>
      )}
    >
      <div className="flex flex-wrap items-center gap-2">
        <Input
          aria-label="筛选候选模型"
          placeholder="搜索模型 ID / 显示名称…"
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          className="min-w-0 flex-1 py-1.5 text-sm"
        />
        <select
          aria-label="按系列筛选"
          value={seriesFilter}
          onChange={(event) => setSeriesFilter(event.target.value)}
          className="rounded-console border border-line bg-input px-2.5 py-1.5 text-sm text-ink outline-none"
        >
          <option value="all">全部系列（{allRows.length}）</option>
          {allGroups.map((group) => (
            <option key={group.series} value={group.series}>
              {group.series}（{group.items.length}）
            </option>
          ))}
        </select>
        <Button
          variant="secondary"
          className="px-2.5 py-1.5 text-xs"
          disabled={!providerEnabled || syncPending}
          onClick={onSync}
        >
          {syncPending ? "正在同步…" : "同步上游模型"}
        </Button>
      </div>

      <div className="flex flex-wrap items-center justify-between gap-2 rounded-console border border-line-inner bg-input/40 px-3 py-2 text-xs">
        <span className="font-mono text-muted">
          已选 <strong className="text-ink">{draftSelected.size}</strong> / 候选共 {allRows.length} 个
          {activeDiscovery ? ` · ${discoveryLabel(activeDiscovery)}` : ""}
        </span>
        <div className="flex flex-wrap gap-1.5">
          <Button variant="secondary" className="px-2 py-0.5 text-xs" disabled={filteredRows.length === 0} onClick={selectFiltered}>
            全选当前筛选
          </Button>
          <Button variant="secondary" className="px-2 py-0.5 text-xs" disabled={filteredRows.length === 0} onClick={deselectFiltered}>
            取消当前筛选
          </Button>
          <Button
            variant="secondary"
            className="px-2 py-0.5 text-xs"
            disabled={discoveredIds.length === 0}
            onClick={() => setDraftSelected(new Set(discoveredIds))}
          >
            仅保留已发现（{discoveredIds.length}）
          </Button>
          <Button variant="secondary" className="px-2 py-0.5 text-xs" disabled={draftSelected.size === 0} onClick={() => setDraftSelected(new Set())}>
            清空已选
          </Button>
        </div>
      </div>

      <div className="flex max-h-[48vh] flex-col gap-3 overflow-y-auto pr-1">
        {filteredGroups.length === 0 ? (
          <p className="m-0 rounded-console border border-line px-3 py-6 text-center text-sm text-muted">
            没有匹配的候选模型；可尝试清空搜索词或点击「同步上游模型」。
          </p>
        ) : (
          filteredGroups.map((group) => {
            const selectedInGroup = group.items.filter((item) => draftSelected.has(item.id)).length;
            const allInGroupChecked = selectedInGroup === group.items.length;
            return (
              <div key={group.series} className="overflow-hidden rounded-console border border-line">
                <div className="flex items-center justify-between gap-2 border-b border-line-inner bg-input/60 px-3 py-1.5">
                  <span className="text-xs font-bold text-ink">
                    {group.series}
                    <span className="ml-1.5 font-mono font-normal text-muted">
                      {selectedInGroup}/{group.items.length}
                    </span>
                  </span>
                  <button
                    type="button"
                    className="cursor-pointer border-0 bg-transparent p-0 text-xs font-semibold text-accent-strong hover:underline"
                    onClick={() => toggleGroup(group.items)}
                  >
                    {allInGroupChecked ? "取消本组" : "全选本组"}
                  </button>
                </div>
                <div className="divide-y divide-line-inner">
                  {group.items.map((row) => {
                    const checked = draftSelected.has(row.id);
                    const displayName = typeof row.metadata.display_name === "string" ? row.metadata.display_name : "";
                    const capabilities = typeof row.metadata.capabilities === "object" && row.metadata.capabilities !== null
                      ? row.metadata.capabilities as Record<string, unknown>
                      : {};
                    const declared = CAPABILITY_KEYS.filter((key) => {
                      const claim = capabilities[key];
                      return typeof claim === "object" && claim !== null && (claim as Record<string, unknown>).advertised === true;
                    });
                    return (
                      <label
                        key={row.id}
                        className={`flex cursor-pointer items-center gap-2.5 px-3 py-2 text-xs transition-colors ${checked ? "bg-accent-soft/30" : "hover:bg-input/40"}`}
                      >
                        <input
                          type="checkbox"
                          checked={checked}
                          aria-label={`选择模型 ${row.id}`}
                          onChange={() => toggleOne(row.id)}
                        />
                        <div className="min-w-0 flex-1">
                          <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
                            <span className="font-mono text-xs font-bold break-all text-ink">{row.id}</span>
                            {displayName && displayName !== row.id ? (
                              <span className="text-[0.72rem] text-muted">{displayName}</span>
                            ) : null}
                            {row.metadata.enabled === false ? (
                              <span className="border border-line px-1 py-0.2 text-[0.64rem] text-warning">已禁用</span>
                            ) : null}
                          </div>
                        </div>
                        <div className="flex shrink-0 flex-wrap items-center gap-1">
                          {declared.map((key) => (
                            <span key={key} className="border border-line bg-accent-soft px-1.5 py-0.5 text-[0.62rem] font-semibold text-accent-strong">
                              {CAPABILITY_LABELS[key]}
                            </span>
                          ))}
                          {row.sources.map((source) => (
                            <span key={source} className="border border-line px-1.5 py-0.5 font-mono text-[0.62rem] text-muted">
                              {SOURCE_LABELS[source] ?? source}
                            </span>
                          ))}
                        </div>
                      </label>
                    );
                  })}
                </div>
              </div>
            );
          })
        )}
      </div>
    </FormDialog>
  );
}
