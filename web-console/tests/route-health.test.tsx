import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ModelRouteEditor } from "../src/features/configuration/model-route-editor.js";
import {
  affectedRoutesForProvider,
  buildProviderAvailabilityMap,
  diagnoseCandidateHealth,
  summarizeRouteHealth,
} from "../src/features/configuration/route-health.js";
import type { ConfigFieldSnapshot, ConfigurationSnapshot } from "../src/types.js";

function field(overrides: Partial<ConfigFieldSnapshot>): ConfigFieldSnapshot {
  return {
    key: "provider.openai.enabled",
    module: "core.provider",
    valueType: "boolean",
    source: "default",
    overridden: false,
    editable: true,
    configured: true,
    valid: true,
    revision: null,
    sensitivity: "public",
    applyMode: "restart",
    savedValue: true,
    effectiveValue: true,
    runningValue: true,
    pendingRestart: false,
    ...overrides,
  };
}

function makeSnapshot(overrides: Partial<ConfigurationSnapshot> = {}): ConfigurationSnapshot {
  return {
    revision: "r1",
    fileExists: true,
    providers: {
      presets: [],
      adapters: ["openai_compatible", "openai_responses"],
      credentials: {
        mimo: { configured: true, editable: true, revision: "s1", pending_restart: false },
      },
    },
    fields: [
      field({ key: "provider.openai.enabled", savedValue: false, effectiveValue: false }),
      field({ key: "provider.openai.api_key", sensitivity: "secret", configured: true, savedValue: null, effectiveValue: null }),
      field({ key: "provider.deepseek.enabled", savedValue: true, effectiveValue: true }),
      field({ key: "provider.deepseek.api_key", sensitivity: "secret", configured: true, savedValue: null, effectiveValue: null }),
      field({ key: "provider.gemini.enabled", savedValue: true, effectiveValue: true }),
      field({ key: "provider.gemini.api_key", sensitivity: "secret", configured: false, savedValue: null, effectiveValue: null }),
    ],
    agent: {
      revision: "a1",
      fileExists: true,
      source: "agent_toml",
      editable: true,
      readOnly: false,
      pendingRestart: false,
      savedValue: {
        providers: {
          mimo: { enabled: true, kind: "openai_compatible", base_url: "https://api.xiaomimimo.com/v1" },
        },
        model_routes: {
          private_main: { candidates: ["openai:gpt-5.6-luna", "deepseek:deepseek-v4-flash"] },
          group_main: { candidates: ["openai:gpt-5.6-luna"] },
        },
        tools: {
          web_search: {
            backend: "tavily",
            routes: { private_search: { model: "openai:gpt-5.6-luna" } },
          },
        },
        stt: {
          enabled: false,
          routes: { default: ["platform:qq", "openai:whisper-1"], private: [], group: [] },
        },
      },
      runningValue: {},
    },
    routeDiagnostics: [
      {
        kind: "model_route",
        routeName: "private_main",
        active: true,
        hasAvailableCandidate: true,
        candidates: [
          {
            index: 0,
            candidate: "openai:gpt-5.6-luna",
            provider: "openai",
            model: "gpt-5.6-luna",
            available: false,
            issues: ["provider_disabled"],
            message: "供应商 openai 已停用，运行时将跳过",
          },
          {
            index: 1,
            candidate: "deepseek:deepseek-v4-flash",
            provider: "deepseek",
            model: "deepseek-v4-flash",
            available: true,
            issues: [],
            message: null,
          },
        ],
      },
      {
        kind: "model_route",
        routeName: "group_main",
        active: true,
        hasAvailableCandidate: false,
        candidates: [
          {
            index: 0,
            candidate: "openai:gpt-5.6-luna",
            provider: "openai",
            model: "gpt-5.6-luna",
            available: false,
            issues: ["provider_disabled"],
            message: "供应商 openai 已停用，运行时将跳过",
          },
        ],
      },
    ],
    registeredTools: [],
    restartAvailable: false,
    ...overrides,
  };
}

describe("route-health 诊断与停用影响分析", () => {
  it("准确诊断停用、缺 Key、未配置与可用候选", () => {
    const snapshot = makeSnapshot();
    const map = buildProviderAvailabilityMap(snapshot);

    const disabled = diagnoseCandidateHealth("openai:gpt-5.6-luna", map);
    expect(disabled.available).toBe(false);
    expect(disabled.badge).toBe("供应商已停用");
    expect(disabled.message).toContain("已停用");

    const ok = diagnoseCandidateHealth("deepseek:deepseek-v4-flash", map);
    expect(ok.available).toBe(true);
    expect(ok.badge).toBeNull();

    const noKey = diagnoseCandidateHealth("gemini:gemini-2.5-flash", map);
    expect(noKey.available).toBe(false);
    expect(noKey.badge).toBe("未配 Key");

    const missing = diagnoseCandidateHealth("ghost:model-x", map);
    expect(missing.available).toBe(false);
    expect(missing.severity).toBe("error");
    expect(missing.badge).toBe("未配置");

    const platformStt = diagnoseCandidateHealth("platform:qq", map, { isStt: true });
    expect(platformStt.available).toBe(true);
  });

  it("停用某供应商时识别受影响路由，并区分是否会导致路由无剩余可用候选", () => {
    const snapshot = makeSnapshot();
    const affected = affectedRoutesForProvider(snapshot, "openai");
    const privateMain = affected.find((item) => item.routeName === "private_main");
    const groupMain = affected.find((item) => item.routeName === "group_main");
    const privateSearch = affected.find((item) => item.routeName === "private_search");

    expect(privateMain).toMatchObject({ active: true, willBecomeEmpty: false });
    expect(groupMain).toMatchObject({ active: true, willBecomeEmpty: true });
    // 非 provider_native 时搜索路线标记非活跃
    expect(privateSearch).toMatchObject({ active: false, willBecomeEmpty: false });
  });

  it("汇总顶部配置摘要的路由健康计数", () => {
    const summary = summarizeRouteHealth(makeSnapshot());
    expect(summary).toEqual({
      emptyActiveRoutes: 1,
      disabledCandidateCount: 2,
      unavailableCandidateCount: 2,
    });
  });

  it("ModelRouteEditor 对不可用候选渲染徽标并在全部不可用时显示严重告警", () => {
    const map = buildProviderAvailabilityMap(makeSnapshot());
    const { rerender } = render(
      <ModelRouteEditor
        label="私聊主路线"
        candidates={["openai:gpt-5.6-luna", "deepseek:deepseek-v4-flash"]}
        disabled={false}
        diagnoseCandidate={(candidate) => diagnoseCandidateHealth(candidate, map)}
        onChange={() => undefined}
      />,
    );
    expect(screen.getByText("供应商已停用")).toBeInTheDocument();
    expect(screen.getByText("含 1 个不可用候选")).toBeInTheDocument();
    expect(screen.queryByText("无可用候选")).not.toBeInTheDocument();

    rerender(
      <ModelRouteEditor
        label="群聊主路线"
        candidates={["openai:gpt-5.6-luna"]}
        disabled={false}
        diagnoseCandidate={(candidate) => diagnoseCandidateHealth(candidate, map)}
        onChange={() => undefined}
      />,
    );
    expect(screen.getByText("无可用候选")).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("当前路线所有候选均不可用");
  });
});
