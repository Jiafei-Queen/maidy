import type {
  CandidateIssueCode,
  ConfigFieldSnapshot,
  ConfigurationSnapshot,
  RouteDiagnostic,
} from "../../types.js";
import { AGENT_ROUTE_LABELS, PLATFORM_STT_CANDIDATE, STT_ROUTE_LABELS } from "./agent-draft.js";
import { configInputValue } from "./configuration-values.js";
import { savedProvidersOf } from "./provider-connection.js";

/** 单个路由候选在前端展示用的实时健康状态。 */
export interface CandidateHealth {
  readonly available: boolean;
  readonly severity: "ok" | "warning" | "error";
  readonly badge: string | null;
  readonly message: string | null;
  readonly issues: readonly CandidateIssueCode[];
}

/** 停用某个供应商时受影响的路由明细。 */
export interface AffectedRouteItem {
  readonly kind: "model_route" | "search_route" | "stt_route";
  readonly routeName: string;
  readonly label: string;
  readonly active: boolean;
  readonly willBecomeEmpty: boolean;
}

export interface ProviderAvailability {
  readonly exists: boolean;
  readonly enabled: boolean;
  readonly configured: boolean;
}

const BUILTIN_IDS = new Set(["openai", "deepseek", "bigmodel", "gemini"]);

const SEARCH_ROUTE_LABELS: Record<string, string> = {
  private_search: "私聊搜索路线",
  group_search: "群聊搜索路线",
};

/** 归一化内置别名，与后端 ModelProvider::parse_prefix 保持一致。 */
export function normalizeProviderPrefix(raw: string): string {
  const lower = raw.trim().toLowerCase();
  if (lower === "zhipu" || lower === "glm") return "bigmodel";
  if (lower === "google") return "gemini";
  return lower;
}

/** 解析 `provider:model` 或历史裸模型字符串。 */
export function parseCandidateProvider(
  candidate: string,
  defaultProvider = "openai",
): { provider: string; model: string } {
  const trimmed = candidate.trim();
  const colon = trimmed.indexOf(":");
  if (colon <= 0) {
    return { provider: defaultProvider, model: trimmed };
  }
  return {
    provider: normalizeProviderPrefix(trimmed.slice(0, colon)),
    model: trimmed.slice(colon + 1).trim(),
  };
}

/** 从配置快照与当前未保存草稿计算所有供应商的实时可用性。 */
export function buildProviderAvailabilityMap(
  snapshot: ConfigurationSnapshot,
  publicDraft: Record<string, string> = {},
  customEnabledOverrides: Record<string, boolean> = {},
): Map<string, ProviderAvailability> {
  const map = new Map<string, ProviderAvailability>();
  const fields: readonly ConfigFieldSnapshot[] = snapshot.fields ?? [];
  const credentials = snapshot.providers?.credentials ?? {};
  const managedProviders = snapshot.providers?.managed_providers === true;

  if (!managedProviders) {
    for (const id of BUILTIN_IDS) {
      const enabledField = fields.find((field) => field.key === `provider.${id}.enabled`);
      const enabledRaw = enabledField ? (publicDraft[enabledField.key] ?? configInputValue(enabledField)) : "true";
      const enabled = enabledRaw !== "false";
      const configured =
        credentials[id]?.configured === true
        || fields.some((field) => field.key === `provider.${id}.api_key` && field.configured);
      map.set(id, { exists: true, enabled, configured });
    }
  }

  const savedCustom = savedProvidersOf(snapshot.agent);
  for (const [rawId, record] of Object.entries(savedCustom)) {
    const id = normalizeProviderPrefix(rawId);
    const override = customEnabledOverrides[rawId] ?? customEnabledOverrides[id];
    const apiKeyEnv = typeof record.api_key_env === "string" ? record.api_key_env : "";
    const legacyEnabledField =
      BUILTIN_IDS.has(id) && apiKeyEnv === `${id.toUpperCase()}_API_KEY`
        ? fields.find((field) => field.key === `provider.${id}.enabled`)
        : undefined;
    const legacyEnabled = legacyEnabledField
      ? (publicDraft[legacyEnabledField.key] ?? configInputValue(legacyEnabledField)) !== "false"
      : true;
    const enabled = override !== undefined ? override : record.enabled !== false && legacyEnabled;
    const configured =
      credentials[rawId]?.configured === true
      || credentials[id]?.configured === true
      || (BUILTIN_IDS.has(id)
        && apiKeyEnv === `${id.toUpperCase()}_API_KEY`
        && fields.some((field) => field.key === `provider.${id}.api_key` && field.configured));
    map.set(id, { exists: true, enabled, configured });
  }

  return map;
}

/** 根据后端路由诊断与实时供应商状态，评估单个候选的可用性与告警文案。 */
export function diagnoseCandidateHealth(
  candidate: string,
  availability: ReadonlyMap<string, ProviderAvailability>,
  options: {
    defaultProvider?: string;
    isStt?: boolean;
    serverDiagnostic?: RouteDiagnostic | undefined;
  } = {},
): CandidateHealth {
  const trimmed = candidate.trim();
  if (!trimmed) {
    return { available: true, severity: "ok", badge: null, message: null, issues: [] };
  }
  if (options.isStt && trimmed.toLowerCase() === PLATFORM_STT_CANDIDATE) {
    return { available: true, severity: "ok", badge: null, message: null, issues: [] };
  }

  const { provider, model } = parseCandidateProvider(trimmed, options.defaultProvider ?? "openai");
  const state = availability.get(provider);
  const issues: CandidateIssueCode[] = [];
  const reasons: string[] = [];

  if (!state || !state.exists) {
    issues.push("provider_missing");
    reasons.push(`供应商 ${provider} 未配置`);
  } else {
    if (!state.enabled) {
      issues.push("provider_disabled");
      reasons.push(`供应商 ${provider} 已停用，运行时将跳过`);
    }
    if (!state.configured) {
      issues.push("missing_credential");
      reasons.push(`供应商 ${provider} 未配置 API Key`);
    }
  }

  // 合并服务端基于 models.json 判定出的 model_disabled 告警。
  const serverCandidate = options.serverDiagnostic?.candidates.find(
    (item) => item.provider === provider && item.model === model,
  );
  if (serverCandidate?.issues.includes("model_disabled")) {
    issues.push("model_disabled");
    reasons.push(`模型 ${provider}:${model} 已禁用`);
  }

  if (issues.length === 0) {
    return { available: true, severity: "ok", badge: null, message: null, issues };
  }

  const isError = issues.includes("provider_missing") || issues.includes("model_disabled");
  const badge = issues.includes("provider_missing")
    ? "未配置"
    : issues.includes("provider_disabled")
      ? "供应商已停用"
      : issues.includes("model_disabled")
        ? "模型已禁用"
        : "未配 Key";

  return {
    available: false,
    severity: isError ? "error" : "warning",
    badge,
    message: reasons.join("；"),
    issues,
  };
}

/** 查询停用某供应商时所有受影响的路由及是否会导致该路由无可用候选。 */
export function affectedRoutesForProvider(
  snapshot: ConfigurationSnapshot,
  providerId: string,
  publicDraft: Record<string, string> = {},
): AffectedRouteItem[] {
  const canonical = normalizeProviderPrefix(providerId);
  const baseAvailability = buildProviderAvailabilityMap(snapshot, publicDraft);
  const simulated = new Map(baseAvailability);
  const current = simulated.get(canonical);
  simulated.set(canonical, {
    exists: current?.exists ?? true,
    enabled: false,
    configured: current?.configured ?? false,
  });

  const result: AffectedRouteItem[] = [];
  const savedAgent =
    typeof snapshot.agent?.savedValue === "object" && snapshot.agent.savedValue !== null
      ? (snapshot.agent.savedValue as Record<string, unknown>)
      : {};

  // 1. 文本模型路由
  const modelRoutes =
    typeof savedAgent.model_routes === "object" && savedAgent.model_routes !== null && !Array.isArray(savedAgent.model_routes)
      ? (savedAgent.model_routes as Record<string, unknown>)
      : {};
  for (const [routeName, routeValue] of Object.entries(modelRoutes)) {
    const candidates =
      typeof routeValue === "object" && routeValue !== null && Array.isArray((routeValue as Record<string, unknown>).candidates)
        ? ((routeValue as Record<string, unknown>).candidates as unknown[]).filter((item): item is string => typeof item === "string")
        : [];
    const matched = candidates.some((item) => parseCandidateProvider(item).provider === canonical);
    if (!matched) continue;
    const hasRemaining = candidates.some((item) => diagnoseCandidateHealth(item, simulated).available);
    result.push({
      kind: "model_route",
      routeName,
      label: AGENT_ROUTE_LABELS[routeName] ?? routeName,
      active: true,
      willBecomeEmpty: !hasRemaining,
    });
  }

  // 2. 联网搜索路线
  const tools =
    typeof savedAgent.tools === "object" && savedAgent.tools !== null
      ? (savedAgent.tools as Record<string, unknown>)
      : {};
  const webSearch =
    typeof tools.web_search === "object" && tools.web_search !== null
      ? (tools.web_search as Record<string, unknown>)
      : {};
  const searchBackend = typeof webSearch.backend === "string" ? webSearch.backend.trim().toLowerCase() : "provider_native";
  const searchActive = searchBackend === "" || searchBackend === "provider_native";
  const searchRoutes =
    typeof webSearch.routes === "object" && webSearch.routes !== null && !Array.isArray(webSearch.routes)
      ? (webSearch.routes as Record<string, unknown>)
      : {};
  for (const [routeName, routeValue] of Object.entries(searchRoutes)) {
    const model =
      typeof routeValue === "object" && routeValue !== null && typeof (routeValue as Record<string, unknown>).model === "string"
        ? ((routeValue as Record<string, unknown>).model as string)
        : "";
    if (!model || parseCandidateProvider(model).provider !== canonical) continue;
    result.push({
      kind: "search_route",
      routeName,
      label: SEARCH_ROUTE_LABELS[routeName] ?? routeName,
      active: searchActive,
      willBecomeEmpty: searchActive,
    });
  }

  // 3. STT 转录路线
  const stt =
    typeof savedAgent.stt === "object" && savedAgent.stt !== null
      ? (savedAgent.stt as Record<string, unknown>)
      : {};
  const sttActive = stt.enabled === true;
  const sttRoutes =
    typeof stt.routes === "object" && stt.routes !== null && !Array.isArray(stt.routes)
      ? (stt.routes as Record<string, unknown>)
      : {};
  for (const routeName of ["default", "private", "group"] as const) {
    const candidates = Array.isArray(sttRoutes[routeName])
      ? (sttRoutes[routeName] as unknown[]).filter((item): item is string => typeof item === "string")
      : [];
    const matched = candidates.some(
      (item) => item.trim().toLowerCase() !== PLATFORM_STT_CANDIDATE && parseCandidateProvider(item).provider === canonical,
    );
    if (!matched) continue;
    const hasRemaining = candidates.some((item) => diagnoseCandidateHealth(item, simulated, { isStt: true }).available);
    result.push({
      kind: "stt_route",
      routeName,
      label: `转录 ${STT_ROUTE_LABELS[routeName]}`,
      active: sttActive,
      willBecomeEmpty: sttActive && !hasRemaining,
    });
  }

  return result;
}

/** 汇总配置中心顶部摘要所需的路由健康指标。 */
export function summarizeRouteHealth(snapshot: ConfigurationSnapshot): {
  emptyActiveRoutes: number;
  disabledCandidateCount: number;
  unavailableCandidateCount: number;
} {
  const diagnostics = snapshot.routeDiagnostics ?? [];
  let emptyActiveRoutes = 0;
  let disabledCandidateCount = 0;
  let unavailableCandidateCount = 0;
  for (const route of diagnostics) {
    if (!route.active) continue;
    if (!route.hasAvailableCandidate) emptyActiveRoutes += 1;
    for (const candidate of route.candidates) {
      if (!candidate.available) {
        unavailableCandidateCount += 1;
        if (candidate.issues.includes("provider_disabled") || candidate.issues.includes("model_disabled")) {
          disabledCandidateCount += 1;
        }
      }
    }
  }
  return { emptyActiveRoutes, disabledCandidateCount, unavailableCandidateCount };
}
