/**
 * Agent 联网搜索配置的读取与保存投影。
 *
 * 只识别统一的 tools.web_search：共享字段、Tavily 顶层兼容字段、Exa 和
 * Firecrawl 子表。保存契约拆为 set_web_search（共享 + Tavily）、
 * set_web_search_exa、set_web_search_firecrawl 三个完整替换操作，不携带
 * route 或 secret，搜索路线单独走 set_search_route。
 */

export type AgentWebSearchBackend =
  | "provider_native"
  | "tavily"
  | "deepseek"
  | "exa"
  | "firecrawl"
  | "disabled";

export type AgentWebSearchTimeRange = "day" | "week" | "month" | "year";
export type ExaSearchType = "auto" | "lite" | "deep-lite" | "deep";
export type ExaCategory =
  | ""
  | "company"
  | "publication"
  | "news"
  | "personal site"
  | "financial report"
  | "people";
export type ExaContentsMode = "highlights" | "summary" | "text";
export type FirecrawlSource = "web" | "news";
export type FirecrawlCategory = "research" | "pdf" | "developer" | "gov";
export type FirecrawlScrapeFormat = "none" | "markdown";

export interface AgentExaWebSearchConfig {
  searchType: ExaSearchType;
  category: ExaCategory;
  contents: ExaContentsMode;
  textMaxCharacters: number | null;
  maxAgeHours: number | null;
  /** 页面用英文逗号分隔，保存时拆成数组。 */
  includeDomains: string;
  excludeDomains: string;
  userLocation: string;
  moderation: boolean;
  timeRange: AgentWebSearchTimeRange | null;
}

export interface AgentFirecrawlWebSearchConfig {
  sources: FirecrawlSource[];
  categories: FirecrawlCategory[];
  tbs: string;
  country: string;
  location: string;
  scrapeFormat: FirecrawlScrapeFormat;
  onlyMainContent: boolean;
  safe: boolean;
  highlights: boolean;
}

export interface AgentWebSearchConfig {
  backend: AgentWebSearchBackend;
  maxResults: number;
  searchDepth: "basic" | "advanced";
  topic: "general" | "news" | "finance";
  timeRange: AgentWebSearchTimeRange | null;
  connectTimeoutSeconds: number;
  firstResponseTimeoutSeconds: number;
  totalTimeoutSeconds: number;
  routes: Record<string, string>;
  exa: AgentExaWebSearchConfig;
  firecrawl: AgentFirecrawlWebSearchConfig;
}

export const DEFAULT_EXA_CONFIG: AgentExaWebSearchConfig = {
  searchType: "auto",
  category: "",
  contents: "highlights",
  textMaxCharacters: null,
  maxAgeHours: null,
  includeDomains: "",
  excludeDomains: "",
  userLocation: "",
  moderation: false,
  timeRange: null,
};

export const DEFAULT_FIRECRAWL_CONFIG: AgentFirecrawlWebSearchConfig = {
  sources: ["web"],
  categories: [],
  tbs: "",
  country: "",
  location: "",
  scrapeFormat: "none",
  onlyMainContent: true,
  safe: false,
  highlights: true,
};

export const DEFAULT_WEB_SEARCH_CONFIG: AgentWebSearchConfig = {
  backend: "provider_native",
  maxResults: 5,
  searchDepth: "basic",
  topic: "general",
  timeRange: null,
  connectTimeoutSeconds: 10,
  firstResponseTimeoutSeconds: 30,
  totalTimeoutSeconds: 60,
  routes: {},
  exa: DEFAULT_EXA_CONFIG,
  firecrawl: DEFAULT_FIRECRAWL_CONFIG,
};

/** 联网搜索受管凭据；字段 key 与 core 的 managed_config_fields 保持一致。 */
export const WEB_SEARCH_CREDENTIALS = [
  { backend: "tavily", label: "Tavily", env: "TAVILY_API_KEY", key: "tools.web_search.tavily.api_key" },
  { backend: "exa", label: "Exa", env: "EXA_API_KEY", key: "tools.web_search.exa.api_key" },
  { backend: "firecrawl", label: "Firecrawl", env: "FIRECRAWL_API_KEY", key: "tools.web_search.firecrawl.api_key" },
] as const;

function isRecord(value: unknown): value is Readonly<Record<string, unknown>> {
  return typeof value === "object" && value !== null;
}

function record(value: unknown): Record<string, unknown> {
  return isRecord(value) ? { ...value } : {};
}

function string(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function positiveNumber(value: unknown, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? value : fallback;
}

function optionalInteger(value: unknown, fallback: number | null): number | null {
  return typeof value === "number" && Number.isInteger(value) ? value : fallback;
}

function stringList(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
}

function splitDomains(value: unknown): string[] {
  return stringList(value).map((item) => item.trim()).filter((item) => item.length > 0);
}

/** 只识别统一的 tools.web_search；旧顶层 search_routes 不参与页面读取。 */
export function readAgentWebSearchConfig(documentValue: unknown): AgentWebSearchConfig {
  const webSearch = record(record(record(documentValue).tools).web_search);
  const routes = Object.fromEntries(
    Object.entries(record(webSearch.routes))
      .map(([name, value]) => [name, string(record(value).model)] as const)
      .filter(([, model]) => model.length > 0),
  );
  const backend = string(webSearch.backend);
  const tavily = readTavilyFields(webSearch);
  const searchDepth = string(tavily.searchDepth);
  const topic = string(tavily.topic);
  const timeRange = string(tavily.timeRange);
  return {
    backend: isWebSearchBackend(backend) ? backend : DEFAULT_WEB_SEARCH_CONFIG.backend,
    maxResults: positiveNumber(webSearch.max_results, DEFAULT_WEB_SEARCH_CONFIG.maxResults),
    searchDepth: searchDepth === "advanced" ? "advanced" : "basic",
    topic: topic === "news" || topic === "finance" ? topic : "general",
    timeRange: isWebSearchTimeRange(timeRange) ? timeRange : null,
    connectTimeoutSeconds: positiveNumber(webSearch.connect_timeout_seconds, DEFAULT_WEB_SEARCH_CONFIG.connectTimeoutSeconds),
    firstResponseTimeoutSeconds: positiveNumber(webSearch.first_response_timeout_seconds, DEFAULT_WEB_SEARCH_CONFIG.firstResponseTimeoutSeconds),
    totalTimeoutSeconds: positiveNumber(webSearch.total_timeout_seconds, DEFAULT_WEB_SEARCH_CONFIG.totalTimeoutSeconds),
    routes,
    exa: readExaConfig(webSearch),
    firecrawl: readFirecrawlConfig(webSearch),
  };
}

/** Tavily 参数兼容读取旧顶层字段；新配置优先使用 [tools.web_search.tavily]。 */
function readTavilyFields(webSearch: Record<string, unknown>): {
  searchDepth: unknown;
  topic: unknown;
  timeRange: unknown;
} {
  const tavily = record(webSearch.tavily);
  return {
    searchDepth: tavily.search_depth ?? webSearch.search_depth,
    topic: tavily.topic ?? webSearch.topic,
    timeRange: tavily.time_range ?? webSearch.time_range,
  };
}

function readExaConfig(webSearch: Record<string, unknown>): AgentExaWebSearchConfig {
  const exa = record(webSearch.exa);
  const searchType = string(exa.search_type);
  // 手写配置可能使用 personal_site / financial_report，统一归一到 Exa 的展示值。
  const category = string(exa.category).trim().toLowerCase().replace(/_/g, " ");
  const contents = string(exa.contents);
  const timeRange = string(exa.time_range);
  return {
    searchType: isExaSearchType(searchType) ? searchType : DEFAULT_EXA_CONFIG.searchType,
    category: isExaCategory(category) ? category : "",
    contents: isExaContentsMode(contents) ? contents : DEFAULT_EXA_CONFIG.contents,
    textMaxCharacters: optionalInteger(exa.text_max_characters, null),
    maxAgeHours: optionalInteger(exa.max_age_hours, null),
    includeDomains: splitDomains(exa.include_domains).join(", "),
    excludeDomains: splitDomains(exa.exclude_domains).join(", "),
    userLocation: string(exa.user_location),
    moderation: exa.moderation === true,
    timeRange: isWebSearchTimeRange(timeRange) ? timeRange : null,
  };
}

function readFirecrawlConfig(webSearch: Record<string, unknown>): AgentFirecrawlWebSearchConfig {
  const firecrawl = record(webSearch.firecrawl);
  const sources = stringList(firecrawl.sources)
    .map((item) => item.trim().toLowerCase())
    .filter(isFirecrawlSource);
  const scrapeFormat = string(firecrawl.scrape_format).trim().toLowerCase();
  return {
    sources: sources.length > 0 ? sources : [...DEFAULT_FIRECRAWL_CONFIG.sources],
    categories: stringList(firecrawl.categories)
      .map((item) => item.trim().toLowerCase())
      .filter(isFirecrawlCategory),
    tbs: string(firecrawl.tbs),
    country: string(firecrawl.country),
    location: string(firecrawl.location),
    scrapeFormat: isFirecrawlScrapeFormat(scrapeFormat) ? scrapeFormat : "none",
    onlyMainContent: firecrawl.only_main_content !== false,
    safe: firecrawl.safe === true,
    highlights: firecrawl.highlights !== false,
  };
}

/** 保存前在页面侧完成结果数边界与超时顺序校验，错误必须阻塞提交而不是等后端拒绝。 */
export function webSearchConfigChange(config: AgentWebSearchConfig): Record<string, unknown> {
  if (!Number.isInteger(config.maxResults) || config.maxResults < 1 || config.maxResults > 10) {
    throw new Error("搜索结果数必须是 1 到 10 之间的整数");
  }
  for (const [label, value] of [
    ["连接超时", config.connectTimeoutSeconds],
    ["首响应超时", config.firstResponseTimeoutSeconds],
    ["总超时", config.totalTimeoutSeconds],
  ] as const) {
    if (!Number.isInteger(value) || value < 1) throw new Error(`${label}必须是大于 0 的整数秒数`);
  }
  if (config.connectTimeoutSeconds > config.firstResponseTimeoutSeconds) {
    throw new Error("连接超时不能大于首响应超时");
  }
  if (config.firstResponseTimeoutSeconds > config.totalTimeoutSeconds) {
    throw new Error("首响应超时不能大于总超时");
  }
  return {
    action: "set_web_search",
    backend: config.backend,
    max_results: config.maxResults,
    search_depth: config.searchDepth,
    topic: config.topic,
    time_range: config.timeRange,
    connect_timeout_seconds: config.connectTimeoutSeconds,
    first_response_timeout_seconds: config.firstResponseTimeoutSeconds,
    total_timeout_seconds: config.totalTimeoutSeconds,
  };
}

export function exaWebSearchConfigChange(config: AgentWebSearchConfig): Record<string, unknown> {
  const exa = config.exa;
  const includeDomains = splitDomainInput(exa.includeDomains);
  const excludeDomains = splitDomainInput(exa.excludeDomains);
  if (includeDomains.length > 0 && excludeDomains.length > 0) {
    throw new Error("Exa 限定域名和排除域名不能同时填写");
  }
  if (exa.searchType === "lite" && exa.contents === "summary") {
    throw new Error("Exa lite 搜索模式不支持 summary 内容提取");
  }
  if ((exa.category === "company" || exa.category === "people") && exa.timeRange !== null) {
    throw new Error("Exa company / people 分类不支持时间范围过滤");
  }
  if (exa.textMaxCharacters !== null
    && (!Number.isInteger(exa.textMaxCharacters) || exa.textMaxCharacters < 1 || exa.textMaxCharacters > 1_000_000)) {
    throw new Error("Exa 正文最大字符数必须是 1 到 1000000 之间的整数");
  }
  if (exa.maxAgeHours !== null
    && (!Number.isInteger(exa.maxAgeHours) || exa.maxAgeHours < -1 || exa.maxAgeHours > 720)) {
    throw new Error("Exa 缓存时效必须是 -1 到 720 之间的整数小时");
  }
  const userLocation = exa.userLocation.trim();
  if (userLocation !== "" && !/^[A-Za-z]{2}$/.test(userLocation)) {
    throw new Error("Exa 用户地区必须是两位国家码");
  }
  return {
    action: "set_web_search_exa",
    options: {
      search_type: exa.searchType,
      category: exa.category === "" ? null : exa.category,
      contents: exa.contents,
      text_max_characters: exa.contents === "text" ? exa.textMaxCharacters : null,
      max_age_hours: exa.maxAgeHours,
      include_domains: includeDomains,
      exclude_domains: excludeDomains,
      user_location: userLocation === "" ? null : userLocation.toUpperCase(),
      moderation: exa.moderation,
      time_range: exa.timeRange,
    },
  };
}

export function firecrawlWebSearchConfigChange(config: AgentWebSearchConfig): Record<string, unknown> {
  const firecrawl = config.firecrawl;
  if (firecrawl.sources.length === 0) {
    throw new Error("Firecrawl 至少选择一个搜索来源");
  }
  if (firecrawl.categories.length > 1
    && firecrawl.categories.some((category) => category === "developer" || category === "gov")) {
    throw new Error("Firecrawl developer / gov 分类必须单独使用");
  }
  const tbs = firecrawl.tbs.trim();
  if (tbs !== "" && !["qdr:", "cdr:", "sbd:"].some((prefix) => tbs.startsWith(prefix))) {
    throw new Error("Firecrawl 时间过滤必须以 qdr:、cdr: 或 sbd: 开头");
  }
  const country = firecrawl.country.trim();
  if (country !== "" && !/^[A-Za-z]{2}$/.test(country)) {
    throw new Error("Firecrawl 国家必须是两位国家码");
  }
  const location = firecrawl.location.trim();
  if (location.length > 200) {
    throw new Error("Firecrawl 地区过长");
  }
  return {
    action: "set_web_search_firecrawl",
    options: {
      sources: [...firecrawl.sources],
      categories: [...firecrawl.categories],
      tbs: tbs === "" ? null : tbs,
      country: country === "" ? null : country.toUpperCase(),
      location: location === "" ? null : location,
      scrape_format: firecrawl.scrapeFormat,
      only_main_content: firecrawl.onlyMainContent,
      safe: firecrawl.safe,
      highlights: firecrawl.highlights,
    },
  };
}

function splitDomainInput(value: string): string[] {
  return value
    .split(/[,，\n]/)
    .map((item) => item.trim())
    .filter((item) => item.length > 0);
}

export function webSearchCredentialNotice(
  backend: AgentWebSearchBackend,
  configured: Partial<Record<AgentWebSearchBackend, boolean>>,
): string {
  const credential = WEB_SEARCH_CREDENTIALS.find((item) => item.backend === backend);
  if (!credential || configured[backend] === true) return "";
  return `已选择 ${credential.label}，但 ${credential.env} 尚未配置。请先在“联网搜索凭据”中保存 Key，重启后搜索才可用。`;
}

/** 兼容旧调用方：Tavily 凭据提示单独保留。 */
export function tavilyCredentialNotice(backend: AgentWebSearchBackend, configured: boolean): string {
  return backend === "tavily" && !configured
    ? "已选择 Tavily，但 Tavily API Key 尚未配置。请先在“联网与工具”中保存 Key，重启后搜索才可用。"
    : "";
}

export function webSearchRouteChanges(
  savedRoutes: Record<string, string>,
  formRoutes: Record<string, string>,
): Array<Record<string, unknown>> {
  const changes: Array<Record<string, unknown>> = [];
  for (const name of ["private_search", "group_search"]) {
    const model = (formRoutes[name] ?? "").trim();
    // 后端切换只更新联网搜索参数；空输入或未改动路线都保留当前 agent.toml 内容。
    if (model.length > 0 && model !== (savedRoutes[name] ?? "")) {
      changes.push({ action: "set_search_route", name, model });
    }
  }
  return changes;
}

export function isWebSearchBackend(value: string): value is AgentWebSearchBackend {
  return value === "provider_native"
    || value === "tavily"
    || value === "deepseek"
    || value === "exa"
    || value === "firecrawl"
    || value === "disabled";
}

export function isWebSearchTimeRange(value: string): value is AgentWebSearchTimeRange {
  return value === "day" || value === "week" || value === "month" || value === "year";
}

function isExaSearchType(value: string): value is ExaSearchType {
  return value === "auto" || value === "lite" || value === "deep-lite" || value === "deep";
}

function isExaCategory(value: string): value is ExaCategory {
  return value === ""
    || value === "company"
    || value === "publication"
    || value === "news"
    || value === "personal site"
    || value === "financial report"
    || value === "people";
}

function isExaContentsMode(value: string): value is ExaContentsMode {
  return value === "highlights" || value === "summary" || value === "text";
}

function isFirecrawlSource(value: string): value is FirecrawlSource {
  return value === "web" || value === "news";
}

function isFirecrawlCategory(value: string): value is FirecrawlCategory {
  return value === "research" || value === "pdf" || value === "developer" || value === "gov";
}

function isFirecrawlScrapeFormat(value: string): value is FirecrawlScrapeFormat {
  return value === "none" || value === "markdown";
}

export function webSearchBackendLabel(value: AgentWebSearchBackend): string {
  return ({
    provider_native: "Provider 原生搜索",
    tavily: "Tavily",
    deepseek: "DeepSeek Search",
    exa: "Exa",
    firecrawl: "Firecrawl",
    disabled: "已关闭",
  } as const)[value];
}
