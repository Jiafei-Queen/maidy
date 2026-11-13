use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};

use super::search_support::{
    query_with_resolved_date, render_source_digest, requested_filters, validate_timeouts,
};
use super::{
    FirecrawlCategory, FirecrawlScrapeFormat, FirecrawlSource, MAX_RESULTS_LIMIT, WebSearchConfig,
    WebSearchExecutor, WebSearchOutcome, WebSearchRequest, WebSearchSource, WebSearchTimeRange,
    WebSearchTopic, truncate_error_detail,
};
use crate::{
    error::{LlmError, LlmErrorKind},
    metrics::duration_ms,
};
use qq_maid_common::time_context::{RequestTimeContext, request_time_context};

const FIRECRAWL_SEARCH_URL: &str = "https://api.firecrawl.dev/v2/search";
const FIRECRAWL_SNIPPET_MAX_CHARS: usize = 1_000;

pub(super) struct MissingFirecrawlWebSearchExecutor;

#[async_trait]
impl WebSearchExecutor for MissingFirecrawlWebSearchExecutor {
    async fn query(&self, _req: WebSearchRequest) -> Result<WebSearchOutcome, LlmError> {
        Err(LlmError::new(
            "web_search_not_configured",
            "FIRECRAWL_API_KEY is required when tools.web_search.backend is firecrawl",
            "firecrawl_search_config",
        ))
    }

    fn provider_name(&self) -> &'static str {
        "firecrawl"
    }
}

/// Firecrawl Search API 执行器。连接、首响应和请求总时长分别受限，避免搜索占满 Agent 预算。
pub(super) struct FirecrawlWebSearchExecutor {
    client: reqwest::Client,
    api_key: String,
    endpoint: String,
    config: WebSearchConfig,
}

impl FirecrawlWebSearchExecutor {
    pub(super) fn new(api_key: String, config: WebSearchConfig) -> Result<Self, LlmError> {
        validate_timeouts(&config)?;
        let client = qq_maid_common::http_client::try_builder()
            .map_err(|err| LlmError::config(format!("failed to configure Firecrawl TLS: {err}")))?
            .connect_timeout(Duration::from_secs(config.connect_timeout_seconds))
            .timeout(Duration::from_secs(config.total_timeout_seconds))
            .build()
            .map_err(|err| {
                LlmError::config(format!("failed to build Firecrawl HTTP client: {err}"))
            })?;
        Ok(Self {
            client,
            api_key,
            endpoint: FIRECRAWL_SEARCH_URL.to_owned(),
            config,
        })
    }

    async fn send(
        &self,
        payload: &FirecrawlSearchRequest<'_>,
    ) -> Result<reqwest::Response, LlmError> {
        let request = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
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
                "Firecrawl did not return response headers before the configured deadline",
                "firecrawl_first_response",
            )
        })?
        .map_err(firecrawl_transport_error)
    }
}

#[async_trait]
impl WebSearchExecutor for FirecrawlWebSearchExecutor {
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
        let options = firecrawl_request_options(&req, &self.config, &time_context)?;
        let payload = FirecrawlSearchRequest {
            query: &options.query,
            limit: options.max_results,
            sources: options
                .sources
                .iter()
                .copied()
                .map(FirecrawlSource::as_str)
                .collect(),
            categories: self
                .config
                .firecrawl
                .categories
                .iter()
                .copied()
                .map(FirecrawlCategory::as_str)
                .collect(),
            tbs: options.tbs,
            country: self.config.firecrawl.country.clone(),
            location: self.config.firecrawl.location.clone(),
            safe: self.config.firecrawl.safe,
            highlights: self.config.firecrawl.highlights,
            scrape_options: (self.config.firecrawl.scrape_format
                == FirecrawlScrapeFormat::Markdown)
                .then(|| FirecrawlScrapeOptions {
                    formats: vec!["markdown"],
                    only_main_content: self.config.firecrawl.only_main_content,
                }),
        };
        let response = self.send(&payload).await?;
        let status = response.status();
        if !status.is_success() {
            return Err(firecrawl_status_error(status, response).await);
        }

        let body: FirecrawlSearchResponse = response.json().await.map_err(|err| {
            let stage = if err.is_timeout() {
                "firecrawl_total"
            } else {
                "firecrawl_json"
            };
            LlmError::from_error_source(
                &err,
                LlmErrorKind::Other,
                stage,
                "failed to read Firecrawl search JSON",
            )
        })?;
        if body.success == Some(false) {
            // HTTP 200 但 success=false 表示上游明确拒绝或抓取失败，不能投影成空结果。
            let detail = body
                .error
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("Firecrawl reported an unsuccessful search");
            return Err(LlmError::provider(
                truncate_error_detail(detail, 300),
                "firecrawl_response",
            ));
        }
        let sources = body
            .data
            .map(|data| firecrawl_sources(data, usize::from(payload.limit)))
            .unwrap_or_default();
        if sources.is_empty() {
            // HTTP 与协议都已成功；没有可引用网页是业务空结果，必须让 Core 统一投影为
            // `ok:false + execution_succeeded:true`，不能伪装成可重试的执行故障。
            return Ok(WebSearchOutcome {
                answer: String::new(),
                sources,
                provider: "firecrawl".to_owned(),
                elapsed_ms: duration_ms(started.elapsed()),
            });
        }

        Ok(WebSearchOutcome {
            answer: render_source_digest(&sources),
            sources,
            provider: "firecrawl".to_owned(),
            elapsed_ms: duration_ms(started.elapsed()),
        })
    }

    fn provider_name(&self) -> &'static str {
        "firecrawl"
    }
}

struct FirecrawlRequestOptions {
    query: String,
    max_results: u8,
    sources: Vec<FirecrawlSource>,
    tbs: Option<String>,
}

fn firecrawl_request_options(
    req: &WebSearchRequest,
    config: &WebSearchConfig,
    time_context: &RequestTimeContext,
) -> Result<FirecrawlRequestOptions, LlmError> {
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
    // 新闻请求优先使用 news 来源；其他请求保留配置来源，避免区分类别时误改用户配置。
    let sources = match topic {
        Some(WebSearchTopic::News) => vec![FirecrawlSource::News],
        _ => config.firecrawl.sources.clone(),
    };
    Ok(FirecrawlRequestOptions {
        query: query_with_resolved_date(query, semantic_text, time_context),
        max_results: req
            .max_results
            .unwrap_or(config.max_results)
            .clamp(1, MAX_RESULTS_LIMIT),
        sources,
        tbs: time_range
            .map(|range| firecrawl_tbs(range).to_owned())
            .or_else(|| config.firecrawl.tbs.clone()),
    })
}

fn firecrawl_tbs(time_range: WebSearchTimeRange) -> &'static str {
    match time_range {
        WebSearchTimeRange::Day => "qdr:d",
        WebSearchTimeRange::Week => "qdr:w",
        WebSearchTimeRange::Month => "qdr:m",
        WebSearchTimeRange::Year => "qdr:y",
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FirecrawlSearchRequest<'a> {
    query: &'a str,
    limit: u8,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    sources: Vec<&'static str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    categories: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tbs: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    location: Option<String>,
    safe: bool,
    highlights: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    scrape_options: Option<FirecrawlScrapeOptions>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FirecrawlScrapeOptions {
    formats: Vec<&'static str>,
    only_main_content: bool,
}

#[derive(Deserialize)]
struct FirecrawlSearchResponse {
    #[serde(default)]
    success: Option<bool>,
    #[serde(default)]
    data: Option<FirecrawlSearchData>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Deserialize, Default)]
struct FirecrawlSearchData {
    #[serde(default)]
    web: Vec<FirecrawlWebResult>,
    #[serde(default)]
    news: Vec<FirecrawlNewsResult>,
}

#[derive(Deserialize)]
struct FirecrawlWebResult {
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    markdown: Option<String>,
}

#[derive(Deserialize)]
struct FirecrawlNewsResult {
    #[serde(default)]
    title: String,
    #[serde(default)]
    snippet: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    markdown: Option<String>,
}

fn firecrawl_sources(data: FirecrawlSearchData, limit: usize) -> Vec<WebSearchSource> {
    let mut seen = HashSet::new();
    let mut sources = Vec::new();
    for result in data.web {
        push_firecrawl_source(
            &mut sources,
            &mut seen,
            result.title,
            result.url,
            result.description,
            result.markdown,
        );
    }
    for result in data.news {
        push_firecrawl_source(
            &mut sources,
            &mut seen,
            result.title,
            result.url,
            result.snippet,
            result.markdown,
        );
    }
    sources.truncate(limit);
    sources
}

fn push_firecrawl_source(
    sources: &mut Vec<WebSearchSource>,
    seen: &mut HashSet<String>,
    title: String,
    url: String,
    description: String,
    markdown: Option<String>,
) {
    let Ok(parsed_url) = Url::parse(url.trim()) else {
        return;
    };
    if !matches!(parsed_url.scheme(), "http" | "https") {
        return;
    }
    let normalized_url = parsed_url.to_string();
    if !seen.insert(normalized_url.clone()) {
        return;
    }
    let title = title.trim();
    let snippet = if !description.trim().is_empty() {
        description.trim().to_owned()
    } else {
        markdown
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or_default()
            .to_owned()
    };
    sources.push(WebSearchSource {
        title: if title.is_empty() {
            parsed_url
                .host_str()
                .map(str::to_owned)
                .unwrap_or_else(|| "未命名来源".to_owned())
        } else {
            truncate_chars(title, 200)
        },
        url: normalized_url,
        snippet: truncate_chars(&snippet, FIRECRAWL_SNIPPET_MAX_CHARS),
    });
}

fn truncate_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_owned();
    }
    value.chars().take(limit).collect()
}

fn firecrawl_transport_error(err: reqwest::Error) -> LlmError {
    let stage = if err.is_timeout() && err.is_connect() {
        "firecrawl_connect"
    } else if err.is_timeout() {
        "firecrawl_total"
    } else {
        "firecrawl_request"
    };
    LlmError::from_error_source(
        &err,
        LlmErrorKind::Network,
        stage,
        "Firecrawl search request failed",
    )
}

async fn firecrawl_status_error(status: StatusCode, response: reqwest::Response) -> LlmError {
    let detail = response.text().await.unwrap_or_default();
    let detail = truncate_error_detail(&detail, 300);
    let error = match status.as_u16() {
        400 => LlmError::new(
            "upstream_bad_request",
            "Firecrawl rejected the search request",
            "firecrawl_http",
        ),
        401 => LlmError::new(
            "authentication_failed",
            "Firecrawl rejected the configured API key",
            "firecrawl_http",
        ),
        402 => LlmError::new(
            "quota_exhausted",
            "Firecrawl credits are exhausted",
            "firecrawl_http",
        ),
        403 => LlmError::new(
            "permission_denied",
            "Firecrawl denied access to the search API",
            "firecrawl_http",
        ),
        408 | 504 => LlmError::new(
            "timeout",
            "Firecrawl search timed out upstream",
            "firecrawl_http",
        ),
        429 => LlmError::new(
            "rate_limited",
            "Firecrawl rate limit exceeded",
            "firecrawl_http",
        ),
        500..=599 => LlmError::new(
            "upstream_unavailable",
            format!("Firecrawl search returned HTTP {status}: {detail}"),
            "firecrawl_http",
        ),
        _ => LlmError::provider(
            format!("Firecrawl search returned HTTP {status}: {detail}"),
            "firecrawl_http",
        ),
    };
    error.with_upstream_status(status.as_u16())
}

#[cfg(test)]
mod tests {
    use super::*;
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
        authorization: Option<String>,
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
        state.authorization = headers
            .get("authorization")
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
    ) -> (FirecrawlWebSearchExecutor, Arc<Mutex<MockState>>) {
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
            FirecrawlWebSearchExecutor::new("test-key".to_owned(), WebSearchConfig::default())
                .unwrap();
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
    async fn query_maps_firecrawl_results_and_request_options() {
        let (executor, state) = test_executor(
            200,
            json!({
                "success": true,
                "data": {
                    "web": [{
                        "title": "Rust Web",
                        "url": "https://example.com/rust",
                        "description": "网页摘要"
                    }],
                    "news": [{
                        "title": "Rust News",
                        "url": "https://example.com/news",
                        "snippet": "新闻摘要"
                    }, {
                        "title": "重复网页",
                        "url": "https://example.com/rust",
                        "snippet": "应被去重"
                    }]
                }
            }),
        )
        .await;

        let outcome = executor.query(request()).await.unwrap();

        assert_eq!(outcome.provider, "firecrawl");
        assert_eq!(outcome.sources.len(), 2);
        assert_eq!(outcome.sources[0].snippet, "网页摘要");
        assert_eq!(outcome.sources[1].snippet, "新闻摘要");
        assert!(outcome.answer.contains("https://example.com/rust"));
        let state = state.lock().await;
        assert_eq!(state.authorization.as_deref(), Some("Bearer test-key"));
        assert_eq!(state.requests[0]["limit"], 3);
        assert_eq!(state.requests[0]["sources"], json!(["news"]));
        assert_eq!(state.requests[0]["tbs"], "qdr:w");
        assert!(state.requests[0].get("scrapeOptions").is_none());
    }

    #[tokio::test]
    async fn markdown_scrape_options_and_categories_are_sent_when_configured() {
        let (executor, state) = test_executor(
            200,
            json!({
                "success": true,
                "data": {
                    "web": [{
                        "title": "PDF",
                        "url": "https://example.com/paper.pdf",
                        "markdown": "# 正文"
                    }]
                }
            }),
        )
        .await;
        let mut executor = executor;
        executor.config.firecrawl.scrape_format = FirecrawlScrapeFormat::Markdown;
        executor.config.firecrawl.categories = vec![FirecrawlCategory::Pdf];
        executor.config.firecrawl.sources = vec![FirecrawlSource::Web];

        let outcome = executor
            .query(WebSearchRequest {
                topic: None,
                time_range: None,
                ..request()
            })
            .await
            .unwrap();

        assert_eq!(outcome.sources[0].snippet, "# 正文");
        let state = state.lock().await;
        assert_eq!(state.requests[0]["categories"], json!(["pdf"]));
        assert_eq!(
            state.requests[0]["scrapeOptions"]["formats"],
            json!(["markdown"])
        );
        assert_eq!(state.requests[0]["scrapeOptions"]["onlyMainContent"], true);
    }

    #[tokio::test]
    async fn query_classifies_auth_quota_and_unsuccessful_payloads() {
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

        let (executor, _) =
            test_executor(200, json!({"success": false, "error": "upstream rejected"})).await;
        assert_eq!(
            executor.query(request()).await.unwrap_err().code,
            "provider_error"
        );

        let (executor, _) = test_executor(200, json!({"success": true, "data": {"web": []}})).await;
        let outcome = executor.query(request()).await.unwrap();
        assert!(outcome.answer.is_empty());
        assert!(outcome.sources.is_empty());
    }

    #[tokio::test]
    async fn query_classifies_first_response_timeout() {
        let (mut executor, state) = test_executor(200, json!({"success": true})).await;
        executor.config.first_response_timeout_seconds = 1;
        state.lock().await.delay_ms = 1_100;

        let error = executor.query(request()).await.unwrap_err();

        assert_eq!(error.code, "timeout");
        assert_eq!(error.stage, "firecrawl_first_response");
    }

    #[test]
    fn current_news_query_uses_news_source_and_day_filter() {
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

        let options =
            firecrawl_request_options(&request, &WebSearchConfig::default(), &context).unwrap();

        assert!(options.query.contains("2026-07-25"));
        assert_eq!(options.sources, vec![FirecrawlSource::News]);
        assert_eq!(options.tbs.as_deref(), Some("qdr:d"));
    }

    #[test]
    fn timeout_order_is_validated() {
        let config = WebSearchConfig {
            connect_timeout_seconds: 31,
            first_response_timeout_seconds: 30,
            ..WebSearchConfig::default()
        };
        assert!(FirecrawlWebSearchExecutor::new("key".to_owned(), config).is_err());
    }
}
