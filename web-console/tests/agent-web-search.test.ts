import { describe, expect, it } from "vitest";
import {
  DEFAULT_EXA_CONFIG,
  DEFAULT_FIRECRAWL_CONFIG,
  exaWebSearchConfigChange,
  firecrawlWebSearchConfigChange,
  readAgentWebSearchConfig,
  tavilyCredentialNotice,
  webSearchBackendLabel,
  webSearchConfigChange,
  webSearchCredentialNotice,
  webSearchRouteChanges,
} from "../src/features/configuration/agent-web-search.js";

const baseConfig = {
  backend: "tavily" as const,
  maxResults: 8,
  searchDepth: "advanced" as const,
  topic: "news" as const,
  timeRange: "week" as const,
  connectTimeoutSeconds: 5,
  firstResponseTimeoutSeconds: 15,
  totalTimeoutSeconds: 45,
  routes: {
    private_search: "openai:gpt-search",
    group_search: "gemini:gemini-2.5-flash",
  },
  exa: DEFAULT_EXA_CONFIG,
  firecrawl: DEFAULT_FIRECRAWL_CONFIG,
};

describe("联网搜索配置读取", () => {
  it("只读取 tools.web_search 及其 routes，旧顶层 search_routes 被忽略", () => {
    const parsed = readAgentWebSearchConfig({
      search_routes: { private_search: { model: "legacy-must-be-ignored" } },
      tools: {
        web_search: {
          backend: "tavily",
          max_results: 8,
          search_depth: "advanced",
          topic: "news",
          time_range: "week",
          connect_timeout_seconds: 5,
          first_response_timeout_seconds: 15,
          total_timeout_seconds: 45,
          routes: {
            private_search: { model: "openai:gpt-search" },
            group_search: { model: "gemini:gemini-2.5-flash" },
          },
        },
      },
    });

    expect(parsed).toEqual(baseConfig);
    expect(Object.values(parsed.routes)).not.toContain("legacy-must-be-ignored");
  });

  it("优先读取 tavily 子表，并解析 Exa / Firecrawl 选项", () => {
    const parsed = readAgentWebSearchConfig({
      tools: {
        web_search: {
          backend: "firecrawl",
          max_results: 4,
          search_depth: "basic",
          topic: "general",
          tavily: { search_depth: "advanced", topic: "finance", time_range: "month" },
          exa: {
            search_type: "deep",
            category: "publication",
            contents: "text",
            text_max_characters: 2000,
            max_age_hours: 24,
            include_domains: ["arxiv.org"],
            exclude_domains: [],
            user_location: "cn",
            moderation: true,
            time_range: "day",
          },
          firecrawl: {
            sources: ["web", "news"],
            categories: ["pdf"],
            tbs: "qdr:w",
            country: "jp",
            location: "Tokyo",
            scrape_format: "markdown",
            only_main_content: false,
            safe: true,
            highlights: false,
          },
        },
      },
    });

    expect(parsed.backend).toBe("firecrawl");
    expect(parsed.searchDepth).toBe("advanced");
    expect(parsed.topic).toBe("finance");
    expect(parsed.timeRange).toBe("month");
    expect(parsed.exa).toEqual({
      searchType: "deep",
      category: "publication",
      contents: "text",
      textMaxCharacters: 2000,
      maxAgeHours: 24,
      includeDomains: "arxiv.org",
      excludeDomains: "",
      userLocation: "cn",
      moderation: true,
      timeRange: "day",
    });
    expect(parsed.firecrawl).toEqual({
      sources: ["web", "news"],
      categories: ["pdf"],
      tbs: "qdr:w",
      country: "jp",
      location: "Tokyo",
      scrapeFormat: "markdown",
      onlyMainContent: false,
      safe: true,
      highlights: false,
    });
  });

  it("缺失或非法字段回落默认值", () => {
    const parsed = readAgentWebSearchConfig({});
    expect(parsed.backend).toBe("provider_native");
    expect(parsed.maxResults).toBe(5);
    expect(parsed.searchDepth).toBe("basic");
    expect(parsed.topic).toBe("general");
    expect(parsed.timeRange).toBeNull();
    expect(parsed.routes).toEqual({});
    expect(parsed.exa).toEqual(DEFAULT_EXA_CONFIG);
    expect(parsed.firecrawl).toEqual(DEFAULT_FIRECRAWL_CONFIG);
  });
});

describe("联网搜索保存契约", () => {
  it("为全部后端生成 set_web_search 且不携带 route 或 secret", () => {
    for (const backend of ["provider_native", "tavily", "deepseek", "exa", "firecrawl", "disabled"] as const) {
      const change = webSearchConfigChange({ ...baseConfig, backend });
      expect(change.action).toBe("set_web_search");
      expect(change.backend).toBe(backend);
      expect(change.max_results).toBe(8);
      expect(change.time_range).toBe("week");
      expect(change).not.toHaveProperty("routes");
      expect(change).not.toHaveProperty("api_key");
    }
  });

  it("Exa 选项生成独立完整替换动作并规范化国家码", () => {
    const change = exaWebSearchConfigChange({
      ...baseConfig,
      exa: {
        ...DEFAULT_EXA_CONFIG,
        searchType: "deep",
        category: "publication",
        contents: "text",
        textMaxCharacters: 2000,
        maxAgeHours: 24,
        includeDomains: "arxiv.org, nature.com",
        userLocation: "cn",
        moderation: true,
        timeRange: "month",
      },
    });

    expect(change).toEqual({
      action: "set_web_search_exa",
      options: {
        search_type: "deep",
        category: "publication",
        contents: "text",
        text_max_characters: 2000,
        max_age_hours: 24,
        include_domains: ["arxiv.org", "nature.com"],
        exclude_domains: [],
        user_location: "CN",
        moderation: true,
        time_range: "month",
      },
    });
  });

  it("Firecrawl 选项生成独立完整替换动作并规范化国家码", () => {
    const change = firecrawlWebSearchConfigChange({
      ...baseConfig,
      firecrawl: {
        sources: ["web", "news"],
        categories: ["pdf"],
        tbs: "qdr:w",
        country: "jp",
        location: "Tokyo",
        scrapeFormat: "markdown",
        onlyMainContent: false,
        safe: true,
        highlights: false,
      },
    });

    expect(change).toEqual({
      action: "set_web_search_firecrawl",
      options: {
        sources: ["web", "news"],
        categories: ["pdf"],
        tbs: "qdr:w",
        country: "JP",
        location: "Tokyo",
        scrape_format: "markdown",
        only_main_content: false,
        safe: true,
        highlights: false,
      },
    });
  });

  it("提交前拒绝冲突的 Exa / Firecrawl 选项", () => {
    expect(() => exaWebSearchConfigChange({
      ...baseConfig,
      exa: { ...DEFAULT_EXA_CONFIG, includeDomains: "a.com", excludeDomains: "b.com" },
    })).toThrow(/互斥|不能同时/);
    expect(() => exaWebSearchConfigChange({
      ...baseConfig,
      exa: { ...DEFAULT_EXA_CONFIG, searchType: "lite", contents: "summary" },
    })).toThrow(/lite/);
    expect(() => exaWebSearchConfigChange({
      ...baseConfig,
      exa: { ...DEFAULT_EXA_CONFIG, category: "company", timeRange: "week" },
    })).toThrow(/company/);
    expect(() => firecrawlWebSearchConfigChange({
      ...baseConfig,
      firecrawl: { ...DEFAULT_FIRECRAWL_CONFIG, sources: [] },
    })).toThrow(/至少选择/);
    expect(() => firecrawlWebSearchConfigChange({
      ...baseConfig,
      firecrawl: { ...DEFAULT_FIRECRAWL_CONFIG, categories: ["developer", "pdf"] },
    })).toThrow(/单独使用/);
    expect(() => firecrawlWebSearchConfigChange({
      ...baseConfig,
      firecrawl: { ...DEFAULT_FIRECRAWL_CONFIG, tbs: "week" },
    })).toThrow(/qdr:/);
    expect(() => firecrawlWebSearchConfigChange({
      ...baseConfig,
      firecrawl: { ...DEFAULT_FIRECRAWL_CONFIG, country: "中国" },
    })).toThrow(/两位国家码/);
  });

  it("切换后端时未修改或留空的搜索路线不会生成删除操作", () => {
    expect(webSearchRouteChanges(baseConfig.routes, {
      private_search: baseConfig.routes.private_search,
      group_search: "",
    })).toEqual([]);
    expect(webSearchRouteChanges(baseConfig.routes, {
      private_search: "openai:gpt-search-new",
      group_search: baseConfig.routes.group_search,
    })).toEqual([{
      action: "set_search_route",
      name: "private_search",
      model: "openai:gpt-search-new",
    }]);
  });

  it("提交前校验结果数和超时顺序", () => {
    expect(() => webSearchConfigChange({ ...baseConfig, maxResults: 11 })).toThrow(/1 到 10/);
    expect(() => webSearchConfigChange({ ...baseConfig, connectTimeoutSeconds: 20 })).toThrow(/连接超时/);
    expect(() => webSearchConfigChange({ ...baseConfig, totalTimeoutSeconds: 10 })).toThrow(/总超时/);
  });
});

describe("联网搜索凭据提示", () => {
  it("选择对应后端且未配置 Key 时显示明确提示", () => {
    expect(webSearchCredentialNotice("tavily", { tavily: false })).toMatch(/TAVILY_API_KEY/);
    expect(webSearchCredentialNotice("exa", { exa: false })).toMatch(/EXA_API_KEY/);
    expect(webSearchCredentialNotice("firecrawl", { firecrawl: false })).toMatch(/FIRECRAWL_API_KEY/);
    expect(webSearchCredentialNotice("exa", { exa: true })).toBe("");
    expect(webSearchCredentialNotice("provider_native", {})).toBe("");
  });

  it("Tavily 旧提示保持兼容", () => {
    expect(tavilyCredentialNotice("tavily", false)).toMatch(/Tavily API Key 尚未配置/);
    expect(tavilyCredentialNotice("tavily", true)).toBe("");
    expect(tavilyCredentialNotice("provider_native", false)).toBe("");
    expect(tavilyCredentialNotice("disabled", false)).toBe("");
  });

  it("后端显示名覆盖新增搜索后端", () => {
    expect(webSearchBackendLabel("tavily")).toBe("Tavily");
    expect(webSearchBackendLabel("exa")).toBe("Exa");
    expect(webSearchBackendLabel("firecrawl")).toBe("Firecrawl");
  });
});
