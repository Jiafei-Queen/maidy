import { describe, expect, it } from "vitest";
import {
  applyCuratedSelection,
  discoveryLabel,
  effectiveDiscovery,
  filterModels,
  groupModelsBySeries,
  mergeModels,
  modelSeriesOf,
  selectionStateOf,
} from "../src/features/configuration/provider-model-data.js";

describe("模型 discovery 与本地 metadata 合并", () => {
  const metadata = {
    revision: "models-r1",
    provider: "openai",
    models: [
      {
        id: "known",
        display_name: "本地显示名称",
        enabled: false,
        status: "beta",
        provenance: { display_name: "local_override", context_window: "catalog" },
      },
      { id: "catalog-only", display_name: "参考条目", enabled: true, status: "deprecated", provenance: { display_name: "catalog" } },
    ],
    overrides: [{ provider: "openai", id: "known", display_name: "本地显示名称", enabled: false }],
  };

  it("发现模型优先，未知私有模型保留，只有精确 ID 补充目录和字段来源", () => {
    const rows = mergeModels({ state: "success", models: [{ id: "private/unknown" }, { id: "known" }] }, metadata);
    expect(rows.map((row) => row.id)).toEqual(["known", "private/unknown", "catalog-only"]);
    expect(rows[0]?.sources).toEqual(["connection discovery", "catalog", "local override"]);
    expect(rows[1]?.metadata).toEqual({});
  });

  it("unsupported、failed、unknown 与 success 空列表不混淆", () => {
    const states = ["unsupported", "failed", "unknown", "success"];
    const labels = states.map((state) => discoveryLabel({ state, models: [], category: "test" }));
    expect(new Set(labels).size).toBe(4);
    expect(labels[3]).toMatch(/获取成功.*空列表/);
    for (const label of labels.slice(0, 3)) expect(label).not.toMatch(/暂无模型|空列表/);
    expect(discoveryLabel(null)).toMatch(/尚未获取/);
    // failed 的 models 可能是历史残留，不得并入展示
    expect(mergeModels({ state: "failed", models: [{ id: "stale" }] }, {})).toEqual([]);
  });

  it("filterModels 支持搜索与状态过滤", () => {
    const rows = mergeModels({ state: "success", models: [{ id: "private/unknown" }, { id: "known" }] }, metadata);
    expect(filterModels(rows, "本地显示", "disabled")[0]?.id).toBe("known");
    expect(filterModels(rows, "PRIVATE/", "enabled")[0]?.id).toBe("private/unknown");
    expect(filterModels(rows, "", "deprecated")[0]?.id).toBe("catalog-only");
    expect(filterModels(rows, "", "beta")[0]?.id).toBe("known");
    expect(filterModels(rows, "不存在", "all")).toEqual([]);
  });

  it("未触发实时同步时自动回退到持久化发现缓存，并展示上次同步时间", () => {
    const withCache = {
      ...metadata,
      discovered: {
        state: "success",
        category: "ok",
        synced_at: "2026-11-22T08:30:00Z",
        models: [{ id: "cached-alpha", source: "connection_discovery" }],
      },
    };
    const rows = mergeModels(null, withCache);
    expect(rows[0]?.id).toBe("cached-alpha");
    expect(rows[0]?.sources).toEqual(["connection discovery"]);
    const active = effectiveDiscovery(null, withCache);
    expect(discoveryLabel(active)).toMatch(/获取成功：1 个模型.*上次同步/);
  });

  it("applyCuratedSelection 与 groupModelsBySeries 按精选白名单和系列分组展示", () => {
    const rows = mergeModels(
      {
        state: "success",
        models: [
          { id: "gpt-5-mini" },
          { id: "gpt-4o" },
          { id: "deepseek-chat" },
          { id: "openai/gpt-5-nano" },
        ],
      },
      {
        ...metadata,
        selection_configured: true,
        selected_models: ["gpt-5-mini", "missing-but-selected"],
      },
    );
    const sel = selectionStateOf({
      selection_configured: true,
      selected_models: ["gpt-5-mini", "missing-but-selected"],
    });
    expect(sel).toEqual({ configured: true, selectedIds: ["gpt-5-mini", "missing-but-selected"] });

    const curated = applyCuratedSelection(rows, sel.configured, sel.selectedIds);
    expect(curated.map((row) => row.id)).toEqual(["gpt-5-mini", "missing-but-selected"]);
    expect(applyCuratedSelection(rows, false, [])).toHaveLength(rows.length);

    expect(modelSeriesOf("gpt-5-mini")).toBe("GPT-5");
    expect(modelSeriesOf("gpt-4o")).toBe("GPT-4");
    expect(modelSeriesOf("deepseek-chat")).toBe("DeepSeek");
    expect(modelSeriesOf("openai/gpt-5-nano")).toBe("openai");

    const groups = groupModelsBySeries(rows);
    expect(groups.map((g) => g.series)).toContain("GPT-5");
    expect(groups.map((g) => g.series)).toContain("DeepSeek");
  });
});
