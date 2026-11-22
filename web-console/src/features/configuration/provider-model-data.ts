/** Connection 发现列表是实际暴露集合；目录与本地覆盖只补充展示，不验证运行能力。
 * 合并语义：发现优先（自动回退到服务端持久化发现缓存）、未知私有模型保留、仅精确 ID 补充目录来源。 */

export interface ManagedModel {
  id: string;
  metadata: Record<string, unknown>;
  sources: string[];
  override: Record<string, unknown>;
}

export interface ModelSelectionState {
  configured: boolean;
  selectedIds: string[];
}

export interface ModelSeriesGroup {
  series: string;
  items: ManagedModel[];
}

function recordOf(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function entriesOf(value: unknown): Array<[string, unknown]> {
  const object = recordOf(value);
  return object ? Object.entries(object) : [];
}

function modelsOf(value: unknown): Record<string, unknown>[] {
  const object = recordOf(value);
  if (!object || !Array.isArray(object.models)) return [];
  return (object.models as unknown[]).filter(
    (item): item is Record<string, unknown> => recordOf(item) !== null,
  );
}

/** 读取服务端持久化的最近一次成功模型发现缓存。 */
export function persistedDiscoveryOf(metadata: Record<string, unknown>): Record<string, unknown> | null {
  return recordOf(metadata.discovered);
}

/** 优先使用当前会话实时同步结果；尚未同步时回退到持久化缓存。 */
export function effectiveDiscovery(
  liveDiscovery: Record<string, unknown> | null,
  metadata: Record<string, unknown>,
): Record<string, unknown> | null {
  return liveDiscovery ?? persistedDiscoveryOf(metadata);
}

/** 提取供应商的模型精选白名单配置状态。 */
export function selectionStateOf(metadata: Record<string, unknown>): ModelSelectionState {
  const configured = metadata.selection_configured === true;
  const selectedIds = Array.isArray(metadata.selected_models)
    ? metadata.selected_models.filter((item): item is string => typeof item === "string" && item.trim().length > 0)
    : [];
  return { configured, selectedIds };
}

/** 本地 metadata（目录条目 + 本地覆盖）与 discovery 结果（含持久化缓存）合并为展示行。 */
export function mergeModels(discovery: Record<string, unknown> | null, metadata: Record<string, unknown>): ManagedModel[] {
  const rows = new Map<string, ManagedModel>();
  const overrides = modelsOf({ models: metadata.overrides });
  for (const model of modelsOf(metadata)) {
    const id = typeof model.id === "string" ? model.id : "";
    if (!id) continue;
    const local = overrides.find((item) => item.id === id) ?? {};
    const fromCatalog = entriesOf(model.provenance).some(([, value]) => value === "catalog" || value === "official_patch");
    const sources = fromCatalog ? ["catalog"] : [];
    if (Object.keys(local).length) sources.push("local override");
    rows.set(id, { id, metadata: model, sources, override: local });
  }
  // 仅 success 携带真实模型列表；failed 的 models 可能是历史残留，不并入展示。
  // 当实时同步失败但磁盘已有上次成功缓存时，继续保留上次成功列表，避免网络抖动冲掉候选池。
  const persisted = persistedDiscoveryOf(metadata);
  const discoverySource = discovery?.state === "success"
    ? discovery
    : (discovery === null || discovery.state === "failed") && persisted?.state === "success"
      ? persisted
      : null;
  if (discoverySource?.state === "success") {
    for (const model of modelsOf(discoverySource)) {
      const id = typeof model.id === "string" ? model.id : "";
      if (!id) continue;
      const row = rows.get(id) ?? { id, metadata: {}, sources: [], override: {} };
      if (!row.sources.includes("connection discovery")) {
        row.sources.unshift("connection discovery");
      }
      rows.set(id, row);
    }
  }
  return [...rows.values()].sort(
    (a, b) => Number(b.sources.includes("connection discovery")) - Number(a.sources.includes("connection discovery")) || a.id.localeCompare(b.id),
  );
}

/** 应用精选模型白名单：未配置时原样返回全部；已配置时仅保留所选模型（顺序保持一致）。 */
export function applyCuratedSelection(
  allRows: readonly ManagedModel[],
  configured: boolean,
  selectedIds: readonly string[],
): ManagedModel[] {
  if (!configured) return [...allRows];
  const index = new Map(allRows.map((row) => [row.id, row]));
  const result: ManagedModel[] = [];
  const seen = new Set<string>();
  for (const id of selectedIds) {
    if (seen.has(id)) continue;
    seen.add(id);
    const existing = index.get(id);
    if (existing) {
      result.push(existing);
    } else {
      result.push({ id, metadata: {}, sources: [], override: {} });
    }
  }
  return result;
}

/** 格式化发现缓存的 RFC 3339 同步时间为简洁本地可读文本。 */
export function formatSyncedAt(value: unknown): string {
  if (typeof value !== "string" || !value.trim()) return "";
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value;
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${parsed.getFullYear()}-${pad(parsed.getMonth() + 1)}-${pad(parsed.getDate())} ${pad(parsed.getHours())}:${pad(parsed.getMinutes())}`;
}

/** 获取状态文案：unsupported / failed / unknown / success（含空列表）互不混淆，支持持久化同步时间。 */
export function discoveryLabel(result: Record<string, unknown> | null): string {
  if (!result) return "尚未获取模型（unknown）";
  const models = Array.isArray(result.models) ? result.models : [];
  const labels: Record<string, string> = {
    success: models.length ? `获取成功：${models.length} 个模型` : "获取成功：Connection 返回空列表",
    unsupported: "不支持模型发现（unsupported）",
    failed: "获取失败（failed）",
    unknown: "无法确认模型列表（unknown）",
  };
  const label = labels[typeof result.state === "string" ? result.state : ""] ?? labels.unknown;
  const category = typeof result.category === "string" && result.category ? ` · ${result.category}` : "";
  const syncedAt = formatSyncedAt(result.synced_at);
  const syncedSuffix = syncedAt ? ` · 上次同步 ${syncedAt}` : "";
  return `${label}${category}${syncedSuffix}`;
}

/** 搜索 + 状态过滤；disabled 只匹配本地显式禁用，active/beta/deprecated 按目录状态。 */
export function filterModels(rows: ManagedModel[], query: string, status: string): ManagedModel[] {
  const term = query.trim().toLowerCase();
  return rows.filter((row) => {
    const displayName = typeof row.metadata.display_name === "string" ? row.metadata.display_name : "";
    if (!`${row.id} ${displayName}`.toLowerCase().includes(term)) return false;
    if (status === "all") return true;
    if (status === "disabled") return row.metadata.enabled === false;
    if (status === "enabled") return row.metadata.enabled !== false;
    return row.metadata.status === status;
  });
}

/** 按模型 ID 推导易读系列名称，便于在批量选择弹窗中快速分组勾选。 */
export function modelSeriesOf(modelId: string): string {
  const trimmed = modelId.trim();
  if (!trimmed) return "其他";
  const slashIndex = trimmed.indexOf("/");
  if (slashIndex > 0) {
    return trimmed.slice(0, slashIndex);
  }
  const lower = trimmed.toLowerCase();
  if (/^gpt-5/.test(lower)) return "GPT-5";
  if (/^gpt-4/.test(lower)) return "GPT-4";
  if (/^gpt-3/.test(lower)) return "GPT-3.5";
  if (/^o[134](?:$|-)/.test(lower)) return "o-series";
  if (/^gemini-2\.5/.test(lower)) return "Gemini 2.5";
  if (/^gemini-2/.test(lower)) return "Gemini 2.0";
  if (/^gemini/.test(lower)) return "Gemini";
  if (/^deepseek/.test(lower)) return "DeepSeek";
  if (/^glm-5/.test(lower)) return "GLM-5";
  if (/^glm-4/.test(lower)) return "GLM-4";
  if (/^glm/.test(lower)) return "GLM";
  if (/^mimo/.test(lower)) return "MiMo";
  if (/^claude/.test(lower)) return "Claude";
  if (/^qwen/.test(lower)) return "Qwen";
  if (/^(whisper|tts|dall-e|gpt-image|text-embedding|embedding|omni-moderation)/.test(lower)) {
    return "音频 / 图像 / 向量";
  }
  const firstSegment = trimmed.split(/[-_:]/)[0];
  return firstSegment && firstSegment.length <= 20 ? firstSegment : "其他";
}

/** 将候选模型按系列分组，保持组内顺序，并将“其他”固定在最后。 */
export function groupModelsBySeries(rows: readonly ManagedModel[]): ModelSeriesGroup[] {
  const buckets = new Map<string, ManagedModel[]>();
  for (const row of rows) {
    const series = modelSeriesOf(row.id);
    const list = buckets.get(series);
    if (list) list.push(row);
    else buckets.set(series, [row]);
  }
  return [...buckets.entries()]
    .sort(([a], [b]) => {
      if (a === "其他") return 1;
      if (b === "其他") return -1;
      return a.localeCompare(b);
    })
    .map(([series, items]) => ({ series, items }));
}
