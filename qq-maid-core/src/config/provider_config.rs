//! Agent Provider 配置到 LLM crate 配置的窄转换层。
//!
//! Provider 元数据保留在 `agent.toml`，凭证只按声明的环境变量名从受管环境解析，
//! 避免把 API Key 写回 Agent 文档。

use qq_maid_llm::config::{
    HttpAuthConfig, OpenAiCompatibleProviderConfig, OpenAiResponsesProviderConfig,
};
use qq_maid_llm::provider::types::ModelProvider;

use super::{
    agent::{AgentProviderKind, AgentRuntimeConfig},
    env_optional,
};

pub(super) fn builtin_api_key(prefix: &str) -> Result<Option<String>, crate::error::LlmError> {
    Ok(if super::env_bool(&format!("{prefix}_ENABLED"), true)? {
        env_optional(&format!("{prefix}_API_KEY"))
    } else {
        None
    })
}

/// 结合统一 `[providers.*]` 与旧环境变量解析知名品牌连接的启停、密钥与地址。
///
/// - 当 `agent.managed_providers() == true` 时，`[providers.*]` 是唯一来源：被删除的连接绝不
///   从旧环境变量复活；仍存在的连接优先采用自身配置，仅在仍使用默认 URL/密钥引用时兼容旧 env。
/// - 当 `agent.managed_providers() == false`（旧手工 TOML）时，完全沿用原环境变量行为。
pub(super) fn resolve_standard_provider(
    agent: &AgentRuntimeConfig,
    provider: ModelProvider,
    env_prefix: &str,
    default_base_url: &str,
    env_base_url: Option<String>,
) -> Result<(Option<String>, String, bool), crate::error::LlmError> {
    let env_enabled = super::env_bool(&format!("{env_prefix}_ENABLED"), true)?;
    if let Some(configured) = agent.provider_config(&provider) {
        let uses_legacy_slot = configured.api_key_env == format!("{env_prefix}_API_KEY");
        let enabled = configured.enabled && (!uses_legacy_slot || env_enabled);
        let api_key = if enabled {
            env_optional(&configured.api_key_env)
        } else {
            None
        };
        // 若用户未修改默认地址且部署环境显式设置了旧 BASE_URL，保留旧部署兼容。
        let base_url = if configured.base_url == default_base_url {
            env_base_url.unwrap_or_else(|| configured.base_url.clone())
        } else {
            configured.base_url.clone()
        };
        return Ok((api_key, base_url, !enabled));
    }
    if agent.managed_providers() {
        return Ok((None, default_base_url.to_owned(), true));
    }
    let api_key = builtin_api_key(env_prefix)?;
    let base_url = env_base_url.unwrap_or_else(|| default_base_url.to_owned());
    Ok((api_key, base_url, !env_enabled))
}

pub(super) fn llm_provider_configs(
    agent_config: &AgentRuntimeConfig,
) -> (
    Vec<OpenAiCompatibleProviderConfig>,
    Vec<OpenAiResponsesProviderConfig>,
    Vec<ModelProvider>,
) {
    let mut compatible = Vec::new();
    let mut responses = Vec::new();
    let mut disabled = Vec::new();
    for provider in agent_config.provider_configs() {
        if !provider.enabled {
            disabled.push(provider.id);
            continue;
        }
        // 标准品牌变体在 Commit 5 之前由顶层 LlmConfig 字段承载，避免在 custom 列表重复声明。
        if !matches!(provider.id, ModelProvider::Custom(_)) {
            continue;
        }
        let auth = HttpAuthConfig {
            header: provider.auth_header,
            scheme: provider.auth_scheme,
        };
        let api_key = env_optional(&provider.api_key_env);
        match provider.kind {
            AgentProviderKind::OpenAiCompatible => {
                compatible.push(OpenAiCompatibleProviderConfig {
                    id: provider.id,
                    base_url: provider.base_url,
                    api_key_env: provider.api_key_env,
                    api_key,
                    auth,
                    request_timeout_seconds: provider.request_timeout_seconds,
                });
            }
            AgentProviderKind::OpenAiResponses => {
                responses.push(OpenAiResponsesProviderConfig {
                    id: provider.id,
                    base_url: provider.base_url,
                    api_key_env: provider.api_key_env,
                    api_key,
                    auth,
                    request_timeout_seconds: provider.request_timeout_seconds,
                    chat_fallback: provider.chat_fallback,
                });
            }
        }
    }
    (compatible, responses, disabled)
}
