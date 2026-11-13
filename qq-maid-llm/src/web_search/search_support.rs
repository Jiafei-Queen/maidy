//! 独立搜索 API 共用的超时配置校验、查询过滤解析和来源排版。
use super::{WebSearchConfig, WebSearchSource, WebSearchTimeRange, WebSearchTopic};
use crate::error::LlmError;
use qq_maid_common::text::{truncate_chars_with_ellipsis_trimmed, truncate_markdown_prefix};
use qq_maid_common::time_context::RequestTimeContext;

/// 摘要 answer 的整体字符预算。`/查` 会在「【联网查询】\n\n」前缀之后按 1500 字符
/// 展示正文，这里预留头部空间，保证下游不会再把 answer 整体截断，也就不会切断
/// Markdown 参考来源链接。
const ANSWER_TOTAL_MAX_CHARS: usize = 1_400;
/// answer 中单条检索摘要的展示上限。
const ANSWER_SOURCE_SUMMARY_MAX_CHARS: usize = 160;
/// 单条检索摘要的展示下限；放不下下限时从尾部减少展示条数，而不是只留几个字。
const ANSWER_SOURCE_SUMMARY_MIN_CHARS: usize = 30;
/// answer 中单条标题的展示上限；完整标题仍保留在结构化 `sources`。
const ANSWER_SOURCE_TITLE_MAX_CHARS: usize = 60;
const ANSWER_HEADER: &str = "搜索到以下公开网页结果：";
const ANSWER_SOURCE_HEADER: &str = "\n\n参考来源：";

pub(super) fn validate_timeouts(config: &WebSearchConfig) -> Result<(), LlmError> {
    if config.connect_timeout_seconds == 0
        || config.first_response_timeout_seconds == 0
        || config.total_timeout_seconds == 0
    {
        return Err(LlmError::config(
            "tools.web_search timeout values must be greater than zero",
        ));
    }
    if config.connect_timeout_seconds > config.first_response_timeout_seconds {
        return Err(LlmError::config(
            "tools.web_search.connect_timeout_seconds must not exceed first_response_timeout_seconds",
        ));
    }
    if config.first_response_timeout_seconds > config.total_timeout_seconds {
        return Err(LlmError::config(
            "tools.web_search.first_response_timeout_seconds must not exceed total_timeout_seconds",
        ));
    }
    Ok(())
}

/// 解析请求级 topic/time_range，并应用当前新闻启发式。
///
/// 返回 `None` 表示请求没有限定该维度，由各后端使用自己的配置默认值。
/// 请求显式值优先；未显式指定时，只有“最新/今日 + 新闻”才自动切到 news/day，
/// 避免把普通探询误判成新闻检索。Tavily、Exa 和 Firecrawl 共用同一语义，
/// 保证同一句 `/查` 在不同后端下选择的时间窗口一致。
pub(super) fn requested_filters(
    query: &str,
    semantic_text: &str,
    request_topic: Option<&str>,
    request_time_range: Option<&str>,
) -> Result<(Option<WebSearchTopic>, Option<WebSearchTimeRange>), LlmError> {
    let current_news = is_current_news_request(query, semantic_text);
    let topic = match request_topic {
        Some(value) => Some(parse_request_topic(value)?),
        None if current_news => Some(WebSearchTopic::News),
        None => None,
    };
    let time_range = match request_time_range {
        Some(value) => Some(parse_request_time_range(value)?),
        None if current_news => Some(WebSearchTimeRange::Day),
        None => None,
    };
    Ok((topic, time_range))
}

fn parse_request_topic(value: &str) -> Result<WebSearchTopic, LlmError> {
    WebSearchTopic::parse_config(value, "topic").map_err(|_| {
        LlmError::new(
            "bad_request",
            "topic must be general, news, or finance",
            "request",
        )
    })
}

fn parse_request_time_range(value: &str) -> Result<WebSearchTimeRange, LlmError> {
    WebSearchTimeRange::parse_config(value, "time_range").map_err(|_| {
        LlmError::new(
            "bad_request",
            "time_range must be day, week, month, or year",
            "request",
        )
    })
}

/// 独立搜索后端只接收检索词，因此仅补入服务端已解析的相对日期，
/// 避免把聊天上下文或身份信息误传给第三方搜索。
pub(super) fn query_with_resolved_date(
    query: &str,
    semantic_text: &str,
    time_context: &RequestTimeContext,
) -> String {
    let resolved = time_context.resolve_relative_time_text(semantic_text);
    let resolved = if resolved.is_empty() {
        time_context.resolve_relative_time_text(query)
    } else {
        resolved
    };
    let date_hint = resolved
        .first()
        .map(|item| item.value.as_str())
        .or_else(|| {
            (is_current_time_request(query) || is_current_time_request(semantic_text))
                .then_some(time_context.current_date())
        });
    date_hint
        .map(|date| format!("{query} {date}"))
        .unwrap_or_else(|| query.to_owned())
}

fn is_current_news_request(query: &str, semantic_text: &str) -> bool {
    (has_news_intent(query) || has_news_intent(semantic_text))
        && (is_current_time_request(query) || is_current_time_request(semantic_text))
}

fn has_news_intent(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    ["新闻", "资讯", "报道", "news"]
        .iter()
        .any(|term| value.contains(term))
}

fn is_current_time_request(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    ["今天", "今日", "最新", "today", "latest"]
        .iter()
        .any(|term| value.contains(term))
}

/// 为不生成整段回答的独立搜索后端汇总逐条检索摘要和参考来源。
///
/// 预算分配顺序：
/// 1. 先为每条有摘要的结果锁定编号、展示标题和一段最短摘要，前部长结果无法
///    挤占后续结果的展示位；
/// 2. 参考来源链接在剩余预算内按整行优先占据，放不下的来源整条省略，
///    保证链接永远完整且可追溯；
/// 3. 最后把剩下的预算平均补给摘要；预算异常收缩时才从尾部减少展示条数。
///
/// 结构调整只影响 answer 正文，结构化 `sources` 始终保留全部有效来源。
pub(super) fn render_source_digest(sources: &[WebSearchSource]) -> String {
    let entries = summary_entries(sources);
    let digest_floor = entries
        .iter()
        .map(|(prefix, _)| prefix.chars().count())
        .sum::<usize>()
        + entries.len() * ANSWER_SOURCE_SUMMARY_MIN_CHARS;
    let link_budget = ANSWER_TOTAL_MAX_CHARS
        .saturating_sub(ANSWER_HEADER.chars().count())
        .saturating_sub(ANSWER_SOURCE_HEADER.chars().count())
        .saturating_sub(digest_floor);
    let link_section = render_source_link_section(sources, link_budget);
    let entry_budget = ANSWER_TOTAL_MAX_CHARS
        .saturating_sub(ANSWER_HEADER.chars().count())
        .saturating_sub(link_section.chars().count());

    let mut digest = String::from(ANSWER_HEADER);
    digest.push_str(&render_summary_entries(&entries, entry_budget));
    digest.push_str(&link_section);
    digest
}

/// 先把每条有摘要的结果渲染成「编号 + 展示标题 + 分隔符」，标题按展示上限
/// 缩写，完整标题仍保留在结构化 `sources`。
fn summary_entries(sources: &[WebSearchSource]) -> Vec<(String, &WebSearchSource)> {
    sources
        .iter()
        .filter(|source| !source.snippet.trim().is_empty())
        .enumerate()
        .map(|(index, source)| {
            let title = display_title(&source.title, ANSWER_SOURCE_TITLE_MAX_CHARS);
            (format!("\n{}. {title}：", index + 1), source)
        })
        .collect()
}

/// 参考来源链接按整行纳入预算：单条链接必须完整展示，放不下的来源整条省略，
/// 结构化 `sources` 仍保留全部有效来源，因此不影响可追溯性。
fn render_source_link_section(sources: &[WebSearchSource], budget: usize) -> String {
    let header_len = ANSWER_SOURCE_HEADER.chars().count();
    let mut section = String::new();
    let mut used = header_len;
    for source in sources {
        let Some(line) =
            source_link_line(&source.title, &source.url, ANSWER_SOURCE_TITLE_MAX_CHARS)
        else {
            continue;
        };
        let next = used + line.chars().count();
        if next > budget {
            // 整条省略放不下的链接，继续尝试后面更短的来源。
            continue;
        }
        section.push_str(&line);
        used = next;
    }
    if used > header_len {
        format!("{ANSWER_SOURCE_HEADER}{section}")
    } else {
        String::new()
    }
}

/// 把摘要区预算先按每条下限分配，剩余额度均分补给各条（单条不超过展示上限）。
fn render_summary_entries(entries: &[(String, &WebSearchSource)], budget: usize) -> String {
    // 预算已为每条结果锁定了下限，这里是常量被调小时的兜底：后续条目整体不展示。
    let mut shown = 0usize;
    let mut fixed = 0usize;
    for (prefix, _) in entries {
        let next_fixed = fixed + prefix.chars().count();
        if next_fixed + (shown + 1) * ANSWER_SOURCE_SUMMARY_MIN_CHARS > budget {
            break;
        }
        fixed = next_fixed;
        shown += 1;
    }
    if shown == 0 {
        return String::new();
    }

    let spare = budget - fixed - shown * ANSWER_SOURCE_SUMMARY_MIN_CHARS;
    let summary_budget =
        (ANSWER_SOURCE_SUMMARY_MIN_CHARS + spare / shown).min(ANSWER_SOURCE_SUMMARY_MAX_CHARS);
    let mut digest = String::new();
    for (prefix, source) in entries.iter().take(shown) {
        digest.push_str(prefix);
        digest.push_str(&truncate_summary(&source.snippet, summary_budget));
    }
    digest
}

/// 摘要按展示预算截断，但不能切断摘要文本自带的 Markdown 行内链接。
///
/// 摘要整体就是一条放不下的链接时安全前缀为空，此时宁可只输出省略号，也不
/// 输出残缺的 `[标签](URL)`；完整摘要在结构化 `sources` 中仍可追溯。
fn truncate_summary(summary: &str, budget: usize) -> String {
    let summary = summary.trim();
    if summary.chars().count() <= budget {
        return summary.to_owned();
    }
    let (prefix, _) = truncate_markdown_prefix(summary, budget);
    if prefix.is_empty() {
        return "…".to_owned();
    }
    format!("{}…", prefix)
}

pub(super) fn render_source_links(answer: Option<&str>, sources: &[WebSearchSource]) -> String {
    let answer = answer.map(str::trim).filter(|value| !value.is_empty());
    let mut output = answer
        .map(str::to_owned)
        .unwrap_or_else(|| ANSWER_HEADER.to_owned());
    output.push_str("\n\n参考来源：");
    for source in sources {
        push_escaped_source_link(
            &mut output,
            &escape_markdown_label(&source.title),
            &source.url,
        );
    }
    output
}

/// 追加一条 `- [已转义标签](URL)` 参考来源行。
///
/// 展示层会按字符预算裁剪正文，链接必须整行渲染：残缺的 `[标题](URL)` 既不可点击
/// 也会污染 Markdown 结构，因此所有来源链接排版统一从这里输出。标签必须由调用方
/// 转义，避免重复转义破坏标题。
fn push_escaped_source_link(output: &mut String, escaped_label: &str, url: &str) {
    output.push_str("\n- [");
    output.push_str(escaped_label);
    output.push_str("](");
    output.push_str(url);
    output.push(')');
}

/// 渲染一行带上限展示标题的参考来源链接（含前导换行），URL 必须完整保留。
///
/// 标题过长时只截断链接标签，完整标题仍保留在结构化 `sources`；URL 为空时返回
/// `None`，调用方应整条省略该来源。
pub(super) fn source_link_line(title: &str, url: &str, title_max_chars: usize) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    let label = display_title(title, title_max_chars);
    // 标题为空或全被截断时退回使用 URL 作为标签，保证链接始终可点击。
    let label = if label.is_empty() {
        escape_markdown_label(url)
    } else {
        label
    };
    let mut line = String::new();
    push_escaped_source_link(&mut line, &label, url);
    Some(line)
}

/// answer 摘要中的展示标题：转义后再按展示上限截断。
pub(super) fn display_title(title: &str, max_chars: usize) -> String {
    truncate_chars_with_ellipsis_trimmed(&escape_markdown_label(title.trim()), max_chars)
}

/// 转义 Markdown 链接标签中的方括号和反斜杠，供来源链接与 answer 摘要共用。
pub(super) fn escape_markdown_label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]")
}
