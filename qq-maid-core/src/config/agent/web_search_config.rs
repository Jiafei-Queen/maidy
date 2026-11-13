use std::collections::HashMap;

use qq_maid_llm::web_search::{
    DEFAULT_MAX_RESULTS, ExaCategory, ExaContentsMode, ExaSearchOptions, ExaSearchType,
    FirecrawlCategory, FirecrawlScrapeFormat, FirecrawlSearchOptions, FirecrawlSource,
    MAX_RESULTS_LIMIT, TavilySearchOptions, WebSearchBackend, WebSearchConfig, WebSearchDepth,
    WebSearchTimeRange, WebSearchTopic,
};
use serde::{Deserialize, Serialize};

use crate::error::LlmError;

use super::SearchRouteFile;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(in crate::config) struct ToolsConfigFile {
    #[serde(default)]
    pub(in crate::config) web_search: WebSearchConfigFile,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(in crate::config) struct WebSearchConfigFile {
    #[serde(default = "default_web_search_backend")]
    pub(in crate::config) backend: String,
    /// 每个底层搜索子请求的结果数硬上限；多目标调研对每个目标分别应用。
    #[serde(default = "default_web_search_max_results")]
    pub(in crate::config) max_results: u8,
    /// 旧版 Tavily 顶层搜索深度；仍兼容读取，优先级低于 `[tools.web_search.tavily]`。
    #[serde(default)]
    pub(in crate::config) search_depth: Option<String>,
    /// 旧版 Tavily 顶层主题；仍兼容读取。
    #[serde(default)]
    pub(in crate::config) topic: Option<String>,
    /// 旧版 Tavily 顶层相对时间范围；仍兼容读取。
    #[serde(default)]
    pub(in crate::config) time_range: Option<String>,
    /// Tavily 独立搜索参数；顶层旧字段在新格式中迁移到这里。
    #[serde(default)]
    pub(in crate::config) tavily: TavilyConfigFile,
    /// Exa 独立搜索参数。
    #[serde(default)]
    pub(in crate::config) exa: ExaConfigFile,
    /// Firecrawl 独立搜索参数。
    #[serde(default)]
    pub(in crate::config) firecrawl: FirecrawlConfigFile,
    /// OpenAI/Gemini 等模型原生搜索 route 统一收敛在联网搜索配置下。
    #[serde(default)]
    pub(in crate::config) routes: HashMap<String, SearchRouteFile>,
    #[serde(default = "default_web_search_connect_timeout_seconds")]
    pub(in crate::config) connect_timeout_seconds: u64,
    #[serde(default = "default_web_search_first_response_timeout_seconds")]
    pub(in crate::config) first_response_timeout_seconds: u64,
    #[serde(default = "default_web_search_total_timeout_seconds")]
    pub(in crate::config) total_timeout_seconds: u64,
}

impl Default for WebSearchConfigFile {
    fn default() -> Self {
        Self {
            backend: default_web_search_backend(),
            max_results: default_web_search_max_results(),
            search_depth: None,
            topic: None,
            time_range: None,
            tavily: TavilyConfigFile::default(),
            exa: ExaConfigFile::default(),
            firecrawl: FirecrawlConfigFile::default(),
            routes: HashMap::new(),
            connect_timeout_seconds: default_web_search_connect_timeout_seconds(),
            first_response_timeout_seconds: default_web_search_first_response_timeout_seconds(),
            total_timeout_seconds: default_web_search_total_timeout_seconds(),
        }
    }
}

/// Tavily 独立搜索参数；字段缺省时沿用历史默认值。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(in crate::config) struct TavilyConfigFile {
    #[serde(default)]
    pub(in crate::config) search_depth: Option<String>,
    #[serde(default)]
    pub(in crate::config) topic: Option<String>,
    #[serde(default)]
    pub(in crate::config) time_range: Option<String>,
}

/// Exa 独立搜索参数；未配置的字段使用 Exa 官方默认行为。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(in crate::config) struct ExaConfigFile {
    #[serde(default)]
    pub(in crate::config) search_type: Option<String>,
    /// 空字符串或省略表示不限定分类。
    #[serde(default)]
    pub(in crate::config) category: Option<String>,
    #[serde(default)]
    pub(in crate::config) contents: Option<String>,
    #[serde(default)]
    pub(in crate::config) text_max_characters: Option<u32>,
    #[serde(default)]
    pub(in crate::config) max_age_hours: Option<i32>,
    #[serde(default)]
    pub(in crate::config) include_domains: Vec<String>,
    #[serde(default)]
    pub(in crate::config) exclude_domains: Vec<String>,
    /// 两位 ISO 3166-1 国家码。
    #[serde(default)]
    pub(in crate::config) user_location: Option<String>,
    #[serde(default)]
    pub(in crate::config) moderation: bool,
    /// 可选默认时间范围；请求级时间范围优先。
    #[serde(default)]
    pub(in crate::config) time_range: Option<String>,
}

/// Firecrawl 独立搜索参数；未配置的字段使用 Firecrawl 官方默认行为。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(in crate::config) struct FirecrawlConfigFile {
    /// 省略时默认只搜索 web；显式空数组视为配置错误。
    #[serde(default)]
    pub(in crate::config) sources: Option<Vec<String>>,
    #[serde(default)]
    pub(in crate::config) categories: Vec<String>,
    /// 时间过滤，例如 `qdr:w`、`sbd:1` 或 `cdr:...`；请求级时间范围优先。
    #[serde(default)]
    pub(in crate::config) tbs: Option<String>,
    #[serde(default)]
    pub(in crate::config) country: Option<String>,
    #[serde(default)]
    pub(in crate::config) location: Option<String>,
    #[serde(default)]
    pub(in crate::config) scrape_format: Option<String>,
    #[serde(default = "default_true")]
    pub(in crate::config) only_main_content: bool,
    #[serde(default)]
    pub(in crate::config) safe: bool,
    #[serde(default = "default_true")]
    pub(in crate::config) highlights: bool,
}

impl Default for FirecrawlConfigFile {
    fn default() -> Self {
        Self {
            sources: None,
            categories: Vec::new(),
            tbs: None,
            country: None,
            location: None,
            scrape_format: None,
            only_main_content: true,
            safe: false,
            highlights: true,
        }
    }
}

pub(super) fn web_search_from_file(
    file: &WebSearchConfigFile,
) -> Result<WebSearchConfig, LlmError> {
    if !(1..=MAX_RESULTS_LIMIT).contains(&file.max_results) {
        return Err(LlmError::config(format!(
            "tools.web_search.max_results must be between 1 and {MAX_RESULTS_LIMIT}"
        )));
    }
    if file.connect_timeout_seconds == 0
        || file.first_response_timeout_seconds == 0
        || file.total_timeout_seconds == 0
    {
        return Err(LlmError::config(
            "tools.web_search timeout values must be greater than zero",
        ));
    }
    if file.connect_timeout_seconds > file.first_response_timeout_seconds {
        return Err(LlmError::config(
            "tools.web_search.connect_timeout_seconds must not exceed first_response_timeout_seconds",
        ));
    }
    if file.first_response_timeout_seconds > file.total_timeout_seconds {
        return Err(LlmError::config(
            "tools.web_search.first_response_timeout_seconds must not exceed total_timeout_seconds",
        ));
    }
    Ok(WebSearchConfig {
        default_backend: WebSearchBackend::parse_config(&file.backend, "tools.web_search.backend")?,
        // 场景 route 会在请求级覆盖；AppConfig::llm_config 再换成私聊默认 route。
        default_model: "gpt-search".to_owned(),
        max_results: file.max_results,
        tavily: tavily_from_file(file)?,
        exa: exa_from_file(&file.exa)?,
        firecrawl: firecrawl_from_file(&file.firecrawl)?,
        connect_timeout_seconds: file.connect_timeout_seconds,
        first_response_timeout_seconds: file.first_response_timeout_seconds,
        total_timeout_seconds: file.total_timeout_seconds,
    })
}

/// `[tools.web_search.tavily]` 优先，其次兼容旧顶层字段，最后使用历史默认值。
fn tavily_from_file(file: &WebSearchConfigFile) -> Result<TavilySearchOptions, LlmError> {
    let search_depth = file
        .tavily
        .search_depth
        .as_deref()
        .or(file.search_depth.as_deref())
        .unwrap_or("basic");
    let topic = file
        .tavily
        .topic
        .as_deref()
        .or(file.topic.as_deref())
        .unwrap_or("general");
    let time_range = file
        .tavily
        .time_range
        .as_deref()
        .or(file.time_range.as_deref());
    Ok(TavilySearchOptions {
        search_depth: WebSearchDepth::parse_config(
            search_depth,
            "tools.web_search.tavily.search_depth",
        )?,
        topic: WebSearchTopic::parse_config(topic, "tools.web_search.tavily.topic")?,
        time_range: time_range
            .map(|value| {
                WebSearchTimeRange::parse_config(value, "tools.web_search.tavily.time_range")
            })
            .transpose()?,
    })
}

fn exa_from_file(file: &ExaConfigFile) -> Result<ExaSearchOptions, LlmError> {
    let search_type = ExaSearchType::parse_config(
        file.search_type.as_deref().unwrap_or("auto"),
        "tools.web_search.exa.search_type",
    )?;
    let category = file
        .category
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| ExaCategory::parse_config(value, "tools.web_search.exa.category"))
        .transpose()?;
    let contents = ExaContentsMode::parse_config(
        file.contents.as_deref().unwrap_or("highlights"),
        "tools.web_search.exa.contents",
    )?;
    let time_range = file
        .time_range
        .as_deref()
        .map(|value| WebSearchTimeRange::parse_config(value, "tools.web_search.exa.time_range"))
        .transpose()?;
    if contents == ExaContentsMode::Summary && !search_type.supports_summary() {
        return Err(LlmError::config(
            "tools.web_search.exa.contents must be highlights or text when search_type is lite",
        ));
    }
    if let Some(value) = file.text_max_characters
        && !(1..=1_000_000).contains(&value)
    {
        return Err(LlmError::config(
            "tools.web_search.exa.text_max_characters must be between 1 and 1000000",
        ));
    }
    if let Some(value) = file.max_age_hours
        && !(-1..=720).contains(&value)
    {
        return Err(LlmError::config(
            "tools.web_search.exa.max_age_hours must be between -1 and 720",
        ));
    }
    let include_domains = normalize_domains(
        &file.include_domains,
        "tools.web_search.exa.include_domains",
    )?;
    let exclude_domains = normalize_domains(
        &file.exclude_domains,
        "tools.web_search.exa.exclude_domains",
    )?;
    if !include_domains.is_empty() && !exclude_domains.is_empty() {
        return Err(LlmError::config(
            "tools.web_search.exa.include_domains and exclude_domains are mutually exclusive",
        ));
    }
    let user_location = file
        .user_location
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            if value.len() != 2 || !value.chars().all(|ch| ch.is_ascii_alphabetic()) {
                return Err(LlmError::config(
                    "tools.web_search.exa.user_location must be a two-letter country code",
                ));
            }
            Ok(value.to_ascii_uppercase())
        })
        .transpose()?;
    if category.is_some_and(|category| !category.supports_published_date()) && time_range.is_some()
    {
        return Err(LlmError::config(
            "tools.web_search.exa.time_range is not supported with the company or people category",
        ));
    }
    Ok(ExaSearchOptions {
        search_type,
        category,
        contents,
        text_max_characters: file.text_max_characters,
        max_age_hours: file.max_age_hours,
        include_domains,
        exclude_domains,
        user_location,
        moderation: file.moderation,
        time_range,
    })
}

fn firecrawl_from_file(file: &FirecrawlConfigFile) -> Result<FirecrawlSearchOptions, LlmError> {
    let sources = match &file.sources {
        Some(values) => {
            if values.is_empty() {
                return Err(LlmError::config(
                    "tools.web_search.firecrawl.sources must contain at least one source",
                ));
            }
            values
                .iter()
                .map(|value| {
                    FirecrawlSource::parse_config(value, "tools.web_search.firecrawl.sources")
                })
                .collect::<Result<Vec<_>, _>>()?
        }
        None => vec![FirecrawlSource::Web],
    };
    let categories = file
        .categories
        .iter()
        .map(|value| {
            FirecrawlCategory::parse_config(value, "tools.web_search.firecrawl.categories")
        })
        .collect::<Result<Vec<_>, _>>()?;
    if categories.len() > 1
        && categories.iter().any(|category| {
            matches!(
                category,
                FirecrawlCategory::Developer | FirecrawlCategory::Gov
            )
        })
    {
        return Err(LlmError::config(
            "tools.web_search.firecrawl.categories must use developer or gov alone",
        ));
    }
    let tbs = file
        .tbs
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            if !["qdr:", "cdr:", "sbd:"]
                .iter()
                .any(|prefix| value.starts_with(prefix))
                || value.chars().any(char::is_control)
            {
                return Err(LlmError::config(
                    "tools.web_search.firecrawl.tbs must start with qdr:, cdr:, or sbd:",
                ));
            }
            Ok(value.to_owned())
        })
        .transpose()?;
    let country = file
        .country
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            if value.len() != 2 || !value.chars().all(|ch| ch.is_ascii_alphabetic()) {
                return Err(LlmError::config(
                    "tools.web_search.firecrawl.country must be a two-letter country code",
                ));
            }
            Ok(value.to_ascii_uppercase())
        })
        .transpose()?;
    let location = file
        .location
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            if value.chars().count() > 200 {
                return Err(LlmError::config(
                    "tools.web_search.firecrawl.location is too long",
                ));
            }
            Ok(value.to_owned())
        })
        .transpose()?;
    Ok(FirecrawlSearchOptions {
        sources,
        categories,
        tbs,
        country,
        location,
        scrape_format: FirecrawlScrapeFormat::parse_config(
            file.scrape_format.as_deref().unwrap_or("none"),
            "tools.web_search.firecrawl.scrape_format",
        )?,
        only_main_content: file.only_main_content,
        safe: file.safe,
        highlights: file.highlights,
    })
}

/// 域名过滤只保留非空白项；Exa 允许 hostname 和路径前缀，这里不裁剪合法 path。
fn normalize_domains(values: &[String], name: &str) -> Result<Vec<String>, LlmError> {
    if values.len() > 1_200 {
        return Err(LlmError::config(format!(
            "{name} must not contain more than 1200 items"
        )));
    }
    values
        .iter()
        .map(|value| {
            let value = value.trim();
            if value.is_empty() || value.chars().any(char::is_control) {
                return Err(LlmError::config(format!(
                    "{name} must not contain empty or control characters"
                )));
            }
            Ok(value.to_owned())
        })
        .collect()
}

fn default_true() -> bool {
    true
}

fn default_web_search_backend() -> String {
    "provider_native".to_owned()
}

const fn default_web_search_max_results() -> u8 {
    DEFAULT_MAX_RESULTS
}

const fn default_web_search_connect_timeout_seconds() -> u64 {
    10
}

const fn default_web_search_first_response_timeout_seconds() -> u64 {
    30
}

const fn default_web_search_total_timeout_seconds() -> u64 {
    60
}
