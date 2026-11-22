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

/// 收集通过 `<PROVIDER>_ENABLED=false` 显式停用的内置供应商。
///
/// 停用的供应商不再阻断同路由中其他可用候选，但必须透传给 LLM 层，
/// 确保其即使保留旧 API Key 也不会被初始化，同时在配置中心输出路由告警。
pub(super) fn disabled_builtin_providers() -> Result<Vec<ModelProvider>, crate::error::LlmError> {
    let mut disabled = Vec::new();
    for (prefix, provider) in [
        ("OPENAI", ModelProvider::OpenAi),
        ("DEEPSEEK", ModelProvider::DeepSeek),
        ("BIGMODEL", ModelProvider::BigModel),
        ("GEMINI", ModelProvider::Gemini),
    ] {
        if !super::env_bool(&format!("{prefix}_ENABLED"), true)? {
            disabled.push(provider);
        }
    }
    Ok(disabled)
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
