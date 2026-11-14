use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};

use super::search_support::{
    query_with_resolved_date, render_source_digest, requested_filters, validate_timeouts,
};
use super::{
    ExaCategory, ExaContentsMode, MAX_RESULTS_LIMIT, WebSearchConfig, WebSearchExecutor,
    WebSearchOutcome, WebSearchRequest, WebSearchSource, WebSearchTimeRange, WebSearchTopic,
    truncate_error_detail,
};
use crate::{
    error::{LlmError, LlmErrorKind},
    metrics::duration_ms,
};
use qq_maid_common::time_context::{RequestTimeContext, request_time_context};

const EXA_SEARCH_URL: &str = "https://api.exa.ai/search";
const EXA_SNIPPET_MAX_CHARS: usize = 1_000;

pub(super) struct MissingExaWebSearchExecutor;

#[async_trait]
impl WebSearchExecutor for MissingExaWebSearchExecutor {
    async fn query(&self, _req: WebSearchRequest) -> Result<WebSearchOutcome, LlmError> {
        Err(LlmError::new(
            "web_search_not_configured",
            "EXA_API_KEY is required when tools.web_search.backend is exa",
            "exa_search_config",
        ))
    }

    fn provider_name(&self) -> &'static str {
        "exa"
    }
}

/// Exa Search API 执行器。连接、首响应和请求总时长分别受限，避免搜索占满 Agent 预算。
pub(super) struct ExaWebSearchExecutor {
    client: reqwest::Client,
    api_key: String,
    endpoint: String,
    config: WebSearchConfig,
}

impl ExaWebSearchExecutor {
    pub(super) fn new(api_key: String, config: WebSearchConfig) -> Result<Self, LlmError> {
        validate_timeouts(&config)?;
        let client = qq_maid_common::http_client::try_builder()
            .map_err(|err| LlmError::config(format!("failed to configure Exa TLS: {err}")))?
            .connect_timeout(Duration::from_secs(config.connect_timeout_seconds))
            .timeout(Duration::from_secs(config.total_timeout_seconds))
            .build()
            .map_err(|err| LlmError::config(format!("failed to build Exa HTTP client: {err}")))?;
        Ok(Self {
            client,
            api_key,
            endpoint: EXA_SEARCH_URL.to_owned(),
            config,
        })
    }

    async fn send(&self, payload: &ExaSearchRequest<'_>) -> Result<reqwest::Response, LlmError> {
        let request = self
            .client
            .post(&self.endpoint)
            .header("x-api-key", &self.api_key)
            .json(payload)
            .send();
        tokio::time::timeout(
            Duration::from_secs(self.config.first_response_timeout_seconds),
            request,
        )
        .await
        .map_err(|_| {
            LlmError::new(
                "timeout",
                "Exa did not return response headers before the configured deadline",
                "exa_first_response",
            )
        })?
        .map_err(exa_transport_error)
    }
}

#[async_trait]
impl WebSearchExecutor for ExaWebSearchExecutor {
    async fn query(&self, req: WebSearchRequest) -> Result<WebSearchOutcome, LlmError> {
        let query = req.query.trim();
        if query.is_empty() {
            return Err(LlmError::new(
                "bad_request",
                "query must not be empty",
                "request",
            ));
        }

        let started = Instant::now();
        let time_context = request_time_context();
        let options = exa_request_options(&req, &self.config, &time_context)?;
        let payload = ExaSearchRequest {
            query: &options.query,
            num_results: options.max_results,
            search_type: self.config.exa.search_type.as_str(),
            category: options.category.map(ExaCategory::as_str),
            contents: exa_contents(&self.config, query),
            include_domains: self.config.exa.include_domains.clone(),
            exclude_domains: self.config.exa.exclude_domains.clone(),
            start_published_date: options
                .time_range
                .filter(|_| {
                    options
                        .category
                        .is_none_or(ExaCategory::supports_published_date)
                })
                .map(|time_range| exa_start_published_date(time_range, &time_context)),
            user_location: self.config.exa.user_location.clone(),
            moderation: self.config.exa.moderation,
        };
        let response = self.send(&payload).await?;
        let status = response.status();
        if !status.is_success() {
            return Err(exa_status_error(status, response).await);
        }

        let body: ExaSearchResponse = response.json().await.map_err(|err| {
            let stage = if err.is_timeout() {
                "exa_total"
            } else {
                "exa_json"
            };
            LlmError::from_error_source(
                &err,
                LlmErrorKind::Other,
                stage,
                "failed to read Exa search JSON",
            )
        })?;
        let sources = exa_sources(body.results, usize::from(payload.num_results));
        if sources.is_empty() {
            // HTTP 与协议都已成功；没有可引用网页是业务空结果，必须让 Core 统一投影为
            // `ok:false + execution_succeeded:true`，不能伪装成可重试的执行故障。
            return Ok(WebSearchOutcome {
                answer: String::new(),
                sources,
                provider: "exa".to_owned(),
                elapsed_ms: duration_ms(started.elapsed()),
            });
        }

        Ok(WebSearchOutcome {
            answer: render_source_digest(&sources),
            sources,
            provider: "exa".to_owned(),
            elapsed_ms: duration_ms(started.elapsed()),
        })
    }

    fn provider_name(&self) -> &'static str {
        "exa"
    }
}

struct ExaRequestOptions {
    query: String,
    max_results: u8,
    category: Option<ExaCategory>,
    time_range: Option<WebSearchTimeRange>,
}

fn exa_request_options(
    req: &WebSearchRequest,
    config: &WebSearchConfig,
    time_context: &RequestTimeContext,
) -> Result<ExaRequestOptions, LlmError> {
    let query = req.query.trim();
    let semantic_text = req
        .raw_question
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(query);
    let (topic, time_range) = requested_filters(
        query,
        semantic_text,
        req.topic.as_deref(),
        req.time_range.as_deref(),
    )?;
    // 显式 general 表示不限制分类；未指定时沿用配置分类。
    let category = match topic {
        Some(WebSearchTopic::News) => Some(ExaCategory::News),
        Some(WebSearchTopic::Finance) => Some(ExaCategory::FinancialReport),
        Some(WebSearchTopic::General) => None,
        None => config.exa.category,
    };
    let time_range = time_range.or(config.exa.time_range);
    // company/people 不支持发布日期过滤；不发送不支持的参数，但保留诊断线索，
    // 避免用户误以为请求级时间范围已经生效。
    if let Some(category) = category
        && !category.supports_published_date()
        && time_range.is_some()
    {
        tracing::debug!(
            category = category.as_str(),
            "Exa 分类不支持发布日期过滤，已忽略时间范围"
        );
    }
    Ok(ExaRequestOptions {
        query: query_with_resolved_date(query, semantic_text, time_context),
        max_results: req
            .max_results
            .unwrap_or(config.max_results)
            .clamp(1, MAX_RESULTS_LIMIT),
        category,
        time_range,
    })
}

fn exa_contents(config: &WebSearchConfig, query: &str) -> ExaContentsRequest {
    let max_age_hours = config.exa.max_age_hours;
    match config.exa.contents {
        ExaContentsMode::Highlights => ExaContentsRequest {
            text: None,
            highlights: true,
            summary: None,
            max_age_hours,
        },
        ExaContentsMode::Summary => ExaContentsRequest {
            text: None,
            highlights: false,
            summary: Some(ExaSummaryOptions {
                query: query.to_owned(),
            }),
            max_age_hours,
        },
        ExaContentsMode::Text => ExaContentsRequest {
            text: Some(match config.exa.text_max_characters {
                Some(max_characters) => ExaTextSetting::Options(ExaTextOptions {
                    max_characters: Some(max_characters),
                }),
                None => ExaTextSetting::Enabled(true),
            }),
            highlights: false,
            summary: None,
            max_age_hours,
        },
    }
}

/// 只发送服务端已解析的日期边界；月/年按 30/365 天近似，精确区间用查询词运算符表达。
fn exa_start_published_date(
    time_range: WebSearchTimeRange,
    time_context: &RequestTimeContext,
) -> String {
    let days = match time_range {
        WebSearchTimeRange::Day => 1,
        WebSearchTimeRange::Week => 7,
        WebSearchTimeRange::Month => 30,
        WebSearchTimeRange::Year => 365,
    };
    let date = time_context.local_date() - chrono::Duration::days(days);
    format!("{}T00:00:00.000Z", date.format("%Y-%m-%d"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExaSearchRequest<'a> {
    query: &'a str,
    num_results: u8,
    #[serde(rename = "type")]
    search_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<&'static str>,
    contents: ExaContentsRequest,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    include_domains: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    exclude_domains: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    start_published_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_location: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    moderation: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExaContentsRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<ExaTextSetting>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    highlights: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<ExaSummaryOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_age_hours: Option<i32>,
}

/// Exa 允许 `text: true` 或带提取选项的对象；未配置字符上限时使用简单布尔形式。
#[derive(Serialize)]
#[serde(untagged)]
enum ExaTextSetting {
    Enabled(bool),
    Options(ExaTextOptions),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExaTextOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    max_characters: Option<u32>,
}

#[derive(Serialize)]
struct ExaSummaryOptions {
    query: String,
}

#[derive(Deserialize)]
struct ExaSearchResponse {
    #[serde(default)]
    results: Vec<ExaSearchResult>,
}

#[derive(Deserialize)]
struct ExaSearchResult {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    highlights: Vec<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

fn exa_sources(results: Vec<ExaSearchResult>, limit: usize) -> Vec<WebSearchSource> {
    results
        .into_iter()
        .filter_map(|result| {
            let parsed_url = Url::parse(result.url.trim()).ok()?;
            if !matches!(parsed_url.scheme(), "http" | "https") {
                return None;
            }
            let title = result.title.trim();
            Some(WebSearchSource {
                title: if title.is_empty() {
                    parsed_url
                        .host_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| "未命名来源".to_owned())
                } else {
                    truncate_chars(title, 200)
                },
                url: parsed_url.to_string(),
                snippet: exa_snippet(&result),
            })
        })
        .take(limit)
        .collect()
}

/// 优先使用与查询相关的 highlights；没有时退回 summary 或正文前缀。
fn exa_snippet(result: &ExaSearchResult) -> String {
    let highlights = result
        .highlights
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let text = if !highlights.is_empty() {
        highlights
    } else if let Some(summary) = result.summary.as_deref() {
        summary.trim().to_owned()
    } else {
        result.text.as_deref().unwrap_or_default().trim().to_owned()
    };
    truncate_chars(&text, EXA_SNIPPET_MAX_CHARS)
}

fn truncate_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_owned();
    }
    value.chars().take(limit).collect()
}

fn exa_transport_error(err: reqwest::Error) -> LlmError {
    let stage = if err.is_timeout() && err.is_connect() {
        "exa_connect"
    } else if err.is_timeout() {
        "exa_total"
    } else {
        "exa_request"
    };
    LlmError::from_error_source(
        &err,
        LlmErrorKind::Network,
        stage,
        "Exa search request failed",
    )
}

async fn exa_status_error(status: StatusCode, response: reqwest::Response) -> LlmError {
    let detail = response.text().await.unwrap_or_default();
    let detail = truncate_error_detail(&detail, 300);
    let error = match status.as_u16() {
        400 => LlmError::new(
            "upstream_bad_request",
            "Exa rejected the search request",
            "exa_http",
        ),
        401 => LlmError::new(
            "authentication_failed",
            "Exa rejected the configured API key",
            "exa_http",
        ),
        402 => LlmError::new("quota_exhausted", "Exa credits are exhausted", "exa_http"),
        403 => LlmError::new(
            "permission_denied",
            "Exa denied access to the search API",
            "exa_http",
        ),
        408 | 504 => LlmError::new("timeout", "Exa search timed out upstream", "exa_http"),
        429 => LlmError::new("rate_limited", "Exa rate limit exceeded", "exa_http"),
        500..=599 => LlmError::new(
            "upstream_unavailable",
            format!("Exa search returned HTTP {status}: {detail}"),
            "exa_http",
        ),
        _ => LlmError::provider(
            format!("Exa search returned HTTP {status}: {detail}"),
            "exa_http",
        ),
    };
    error.with_upstream_status(status.as_u16())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web_search::ExaSearchType;
    use axum::{
        Json, Router,
        extract::State,
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::post,
    };
    use chrono::{FixedOffset, TimeZone};
    use serde_json::{Value, json};
    use std::sync::Arc;
    use tokio::{net::TcpListener, sync::Mutex};

    #[derive(Default)]
    struct MockState {
        requests: Vec<Value>,
        api_key: Option<String>,
        status: u16,
        response: Value,
        delay_ms: u64,
    }

    async fn mock_search(
        State(state): State<Arc<Mutex<MockState>>>,
        headers: HeaderMap,
        Json(payload): Json<Value>,
    ) -> impl IntoResponse {
        let delay_ms = state.lock().await.delay_ms;
        if delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }
        let mut state = state.lock().await;
        state.requests.push(payload);
        state.api_key = headers
            .get("x-api-key")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        (
            StatusCode::from_u16(state.status).unwrap(),
            Json(state.response.clone()),
        )
    }

    async fn test_executor(
        status: u16,
        response: Value,
    ) -> (ExaWebSearchExecutor, Arc<Mutex<MockState>>) {
        let state = Arc::new(Mutex::new(MockState {
            status,
            response,
            ..MockState::default()
        }));
        let app = Router::new()
            .route("/search", post(mock_search))
            .with_state(state.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let mut executor =
            ExaWebSearchExecutor::new("test-key".to_owned(), WebSearchConfig::default()).unwrap();
        executor.endpoint = format!("http://{addr}/search");
        (executor, state)
    }

    fn request() -> WebSearchRequest {
        WebSearchRequest {
            query: "Rust 2026".to_owned(),
            raw_question: None,
            max_results: Some(3),
            context_size: None,
            topic: Some("news".to_owned()),
            time_range: Some("week".to_owned()),
            backend_override: None,
            model_override: None,
        }
    }

    #[tokio::test]
    async fn query_maps_exa_results_and_request_options() {
        let (mut executor, state) = test_executor(
            200,
            json!({
                "results": [{
                    "title": "Rust News",
                    "url": "https://example.com/rust",
                    "highlights": ["重点一", "重点二"],
                }]
            }),
        )
        .await;
        executor.config.exa.search_type = ExaSearchType::Deep;
        executor.config.exa.contents = ExaContentsMode::Text;
        executor.config.exa.text_max_characters = Some(4_000);
        executor.config.exa.max_age_hours = Some(24);
        executor.config.exa.include_domains = vec!["example.com".to_owned()];
        executor.config.exa.user_location = Some("CN".to_owned());
        executor.config.exa.moderation = true;

        let outcome = executor.query(request()).await.unwrap();

        assert_eq!(outcome.provider, "exa");
        assert_eq!(outcome.sources[0].snippet, "重点一\n重点二");
        assert!(outcome.answer.contains("https://example.com/rust"));
        let state = state.lock().await;
        assert_eq!(state.api_key.as_deref(), Some("test-key"));
        assert_eq!(state.requests[0]["numResults"], 3);
        assert_eq!(state.requests[0]["type"], "deep");
        assert_eq!(state.requests[0]["category"], "news");
        assert_eq!(state.requests[0]["contents"]["text"]["maxCharacters"], 4000);
        assert_eq!(state.requests[0]["contents"]["maxAgeHours"], 24);
        assert_eq!(state.requests[0]["includeDomains"], json!(["example.com"]));
        assert_eq!(state.requests[0]["userLocation"], "CN");
        assert_eq!(state.requests[0]["moderation"], true);
        assert!(state.requests[0].get("maxAgeHours").is_none());
        assert!(
            state.requests[0]["startPublishedDate"]
                .as_str()
                .unwrap()
                .ends_with('Z')
        );
    }

    #[tokio::test]
    async fn company_category_skips_published_date_and_filters_invalid_urls() {
        let (executor, state) = test_executor(
            200,
            json!({
                "results": [
                    {"title": "Company", "url": "https://example.com/company", "summary": "公司摘要"},
                    {"title": "Bad", "url": "javascript:alert(1)", "text": "不应出现"}
                ]
            }),
        )
        .await;
        let mut executor = executor;
        executor.config.exa.category = Some(ExaCategory::Company);
        executor.config.exa.contents = ExaContentsMode::Summary;
        executor.config.exa.exclude_domains = vec!["blocked.example".to_owned()];
        executor.config.exa.max_age_hours = Some(24);

        let outcome = executor
            .query(WebSearchRequest {
                topic: None,
                time_range: Some("week".to_owned()),
                ..request()
            })
            .await
            .unwrap();

        assert_eq!(outcome.sources.len(), 1);
        assert_eq!(outcome.sources[0].snippet, "公司摘要");
        let state = state.lock().await;
        assert_eq!(state.requests[0]["category"], "company");
        assert_eq!(
            state.requests[0]["excludeDomains"],
            json!(["blocked.example"])
        );
        assert_eq!(state.requests[0]["contents"]["maxAgeHours"], 24);
        assert!(state.requests[0].get("startPublishedDate").is_none());
        assert!(state.requests[0]["contents"]["summary"].is_object());
    }

    #[tokio::test]
    async fn query_classifies_auth_quota_and_keeps_empty_results_completed() {
        let (executor, _) = test_executor(401, json!({"error": "invalid key"})).await;
        assert_eq!(
            executor.query(request()).await.unwrap_err().code,
            "authentication_failed"
        );

        let (executor, _) = test_executor(402, json!({"error": "no credits"})).await;
        assert_eq!(
            executor.query(request()).await.unwrap_err().code,
            "quota_exhausted"
        );

        let (executor, _) = test_executor(429, json!({"error": "rate limit"})).await;
        assert_eq!(
            executor.query(request()).await.unwrap_err().code,
            "rate_limited"
        );

        let (executor, _) = test_executor(200, json!({"results": []})).await;
        let outcome = executor.query(request()).await.unwrap();
        assert!(outcome.answer.is_empty());
        assert!(outcome.sources.is_empty());
    }

    #[tokio::test]
    async fn query_classifies_first_response_timeout() {
        let (mut executor, state) = test_executor(200, json!({"results": []})).await;
        executor.config.first_response_timeout_seconds = 1;
        state.lock().await.delay_ms = 1_100;

        let error = executor.query(request()).await.unwrap_err();

        assert_eq!(error.code, "timeout");
        assert_eq!(error.stage, "exa_first_response");
    }

    #[test]
    fn text_mode_serializes_boolean_or_character_limit() {
        let config = WebSearchConfig {
            exa: crate::web_search::ExaSearchOptions {
                contents: ExaContentsMode::Text,
                ..Default::default()
            },
            ..WebSearchConfig::default()
        };
        assert_eq!(
            serde_json::to_value(exa_contents(&config, "q")).unwrap(),
            json!({"text": true})
        );

        let mut config = config;
        config.exa.text_max_characters = Some(2_000);
        assert_eq!(
            serde_json::to_value(exa_contents(&config, "q")).unwrap(),
            json!({"text": {"maxCharacters": 2000}})
        );

        config.exa.max_age_hours = Some(24);
        assert_eq!(
            serde_json::to_value(exa_contents(&config, "q")).unwrap(),
            json!({"text": {"maxCharacters": 2000}, "maxAgeHours": 24})
        );
    }

    #[test]
    fn timeout_order_is_validated() {
        let config = WebSearchConfig {
            connect_timeout_seconds: 31,
            first_response_timeout_seconds: 30,
            ..WebSearchConfig::default()
        };
        assert!(ExaWebSearchExecutor::new("key".to_owned(), config).is_err());
    }

    #[test]
    fn current_news_query_is_date_resolved_with_news_defaults() {
        let offset = FixedOffset::east_opt(8 * 60 * 60).unwrap();
        let context = RequestTimeContext::from_datetime(
            offset.with_ymd_and_hms(2026, 7, 25, 9, 30, 0).unwrap(),
        );
        let request = WebSearchRequest {
            query: "今日 AI 新闻".to_owned(),
            raw_question: Some("请查今日 AI 新闻".to_owned()),
            max_results: None,
            context_size: None,
            topic: None,
            time_range: None,
            backend_override: None,
            model_override: None,
        };

        let options = exa_request_options(&request, &WebSearchConfig::default(), &context).unwrap();

        assert!(options.query.contains("2026-07-25"));
        assert_eq!(options.category, Some(ExaCategory::News));
        assert_eq!(options.time_range, Some(WebSearchTimeRange::Day));
    }
}
