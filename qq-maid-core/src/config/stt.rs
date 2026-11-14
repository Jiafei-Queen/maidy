//! STT 运行配置：把 `[stt]` 候选链解析为 Gateway 可直接消费的 Provider 端点。
//!
//! 语音音频只在 Gateway 流转，因此这里只暴露 OpenAI 兼容转录所需的 Base URL、
//! 认证方式和密钥；不复制模型路由或 Provider 管理状态。

use std::time::Duration;

use qq_maid_llm::provider::types::{ModelId, ModelProvider};

use super::{AppConfig, ChatScene, SttPlatformAsrMode, agent::AgentProviderKind, env_optional};

const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";

/// STT 链整体状态；缺少凭证不阻止启动，只降级为平台 ASR。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SttRuntimeStatus {
    Disabled,
    Available,
    MissingCredentials,
}

impl SttRuntimeStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Available => "available",
            Self::MissingCredentials => "missing_credentials",
        }
    }
}

/// 单个已解析转录候选。
#[derive(Clone, PartialEq, Eq)]
pub struct SttCandidate {
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub auth_header: String,
    pub auth_scheme: Option<String>,
    pub request_timeout_seconds: Option<u64>,
}

impl std::fmt::Debug for SttCandidate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SttCandidate")
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("base_url", &self.base_url)
            .field("api_key_configured", &self.api_key.is_some())
            .field("auth_header", &self.auth_header)
            .field("auth_scheme", &self.auth_scheme)
            .field("request_timeout_seconds", &self.request_timeout_seconds)
            .finish()
    }
}

impl SttCandidate {
    /// 构造 OpenAI 兼容 `/audio/transcriptions` 端点；允许配置直接写完整端点。
    pub fn endpoint(&self) -> String {
        let base = self.base_url.trim().trim_end_matches('/');
        if base.ends_with("/audio/transcriptions") {
            base.to_owned()
        } else {
            format!("{base}/audio/transcriptions")
        }
    }

    pub fn request_timeout(&self, fallback: Duration) -> Duration {
        self.request_timeout_seconds
            .map(Duration::from_secs)
            .unwrap_or(fallback)
    }
}

/// Gateway 使用的 STT 快照；`Debug` 只暴露密钥是否配置。
#[derive(Clone, PartialEq, Eq)]
pub struct SttRuntimeConfig {
    pub enabled: bool,
    pub status: SttRuntimeStatus,
    pub platform_asr: SttPlatformAsrMode,
    pub language: Option<String>,
    pub timeout: Duration,
    pub platform_asr_attempts: u8,
    pub max_audio_bytes: u64,
    pub max_text_chars: usize,
    pub routes: SttRouteSet,
}

/// 已解析的三条转录候选链；场景链为空时由 [`SttRuntimeConfig::chain`] 回退 `default`。
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SttRouteSet {
    pub default: Vec<SttCandidate>,
    pub private: Vec<SttCandidate>,
    pub group: Vec<SttCandidate>,
}

impl SttRuntimeConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            status: SttRuntimeStatus::Disabled,
            platform_asr: SttPlatformAsrMode::Auto,
            language: None,
            timeout: Duration::from_secs(30),
            platform_asr_attempts: 2,
            max_audio_bytes: 10 * 1024 * 1024,
            max_text_chars: 2000,
            routes: SttRouteSet::default(),
        }
    }

    /// 按场景链选择候选；场景链为空时回退 `default`。
    pub fn chain(&self, scene: ChatScene) -> &[SttCandidate] {
        let chain = match scene {
            ChatScene::Private => &self.routes.private,
            ChatScene::Group => &self.routes.group,
        };
        if chain.is_empty() {
            &self.routes.default
        } else {
            chain
        }
    }

    /// 是否值得创建 STT 客户端：启用且至少一个候选有凭证。
    pub fn is_usable(&self) -> bool {
        self.enabled && self.status == SttRuntimeStatus::Available
    }

    /// 是否存在任何已解析候选；用于区分“启用但未配置”和“未启用”。
    pub fn has_routes(&self) -> bool {
        !self.routes.default.is_empty()
            || !self.routes.private.is_empty()
            || !self.routes.group.is_empty()
    }
}

impl std::fmt::Debug for SttRuntimeConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SttRuntimeConfig")
            .field("enabled", &self.enabled)
            .field("status", &self.status)
            .field("platform_asr", &self.platform_asr)
            .field("language", &self.language)
            .field("timeout", &self.timeout)
            .field("platform_asr_attempts", &self.platform_asr_attempts)
            .field("max_audio_bytes", &self.max_audio_bytes)
            .field("max_text_chars", &self.max_text_chars)
            .field("routes_configured", &self.has_routes())
            .finish()
    }
}

impl AppConfig {
    /// 解析 STT 运行快照；候选 Provider 已在 Agent 配置阶段校验存在性与 kind。
    pub fn stt_runtime(&self) -> SttRuntimeConfig {
        let settings = self.agent_config.stt();
        let default = resolve_chain(self, &settings.routes.default);
        let private = resolve_chain(self, &settings.routes.private);
        let group = resolve_chain(self, &settings.routes.group);
        let has_credentials = default
            .iter()
            .chain(private.iter())
            .chain(group.iter())
            .any(|candidate| candidate.api_key.is_some());
        let status = if !settings.enabled {
            SttRuntimeStatus::Disabled
        } else if has_credentials {
            SttRuntimeStatus::Available
        } else {
            SttRuntimeStatus::MissingCredentials
        };
        SttRuntimeConfig {
            enabled: settings.enabled,
            status,
            platform_asr: settings.platform_asr,
            language: settings.language.clone(),
            timeout: Duration::from_secs(settings.timeout_seconds),
            platform_asr_attempts: settings.platform_asr_attempts,
            max_audio_bytes: settings.max_audio_bytes,
            max_text_chars: settings.max_text_chars,
            routes: SttRouteSet {
                default,
                private,
                group,
            },
        }
    }

    fn resolve_stt_candidate(&self, model: &ModelId) -> SttCandidate {
        let provider = model.provider.clone().unwrap_or(ModelProvider::OpenAi);
        let (base_url, api_key, auth_header, auth_scheme, timeout) = match &provider {
            ModelProvider::OpenAi => (
                self.openai_base_url
                    .clone()
                    .unwrap_or_else(|| DEFAULT_OPENAI_BASE_URL.to_owned()),
                self.openai_api_key.clone(),
                "Authorization".to_owned(),
                Some("Bearer".to_owned()),
                None,
            ),
            ModelProvider::DeepSeek => (
                self.deepseek_base_url.clone(),
                self.deepseek_api_key.clone(),
                "Authorization".to_owned(),
                Some("Bearer".to_owned()),
                None,
            ),
            ModelProvider::BigModel => (
                self.bigmodel_base_url.clone(),
                self.bigmodel_api_key.clone(),
                "Authorization".to_owned(),
                Some("Bearer".to_owned()),
                None,
            ),
            ModelProvider::Gemini => (
                self.gemini_base_url.clone(),
                self.gemini_api_key.clone(),
                "Authorization".to_owned(),
                Some("Bearer".to_owned()),
                None,
            ),
            ModelProvider::Custom(name) => {
                match self
                    .agent_config
                    .provider_configs()
                    .into_iter()
                    .find(|configured| configured.id.as_str() == name)
                {
                    Some(configured) if configured.kind == AgentProviderKind::OpenAiCompatible => (
                        configured.base_url,
                        env_optional(&configured.api_key_env),
                        configured.auth_header,
                        configured.auth_scheme,
                        configured.request_timeout_seconds,
                    ),
                    // Agent 配置阶段已拒绝未知/停用/Responses 候选；保留安全兜底，
                    // 避免未来新增入口绕过校验后静默使用错误协议。
                    _ => (
                        String::new(),
                        None,
                        "Authorization".to_owned(),
                        Some("Bearer".to_owned()),
                        None,
                    ),
                }
            }
        };
        SttCandidate {
            provider: provider.as_str().to_owned(),
            model: model.name.clone(),
            base_url,
            api_key,
            auth_header,
            auth_scheme,
            request_timeout_seconds: timeout,
        }
    }
}

fn resolve_chain(config: &AppConfig, models: &[ModelId]) -> Vec<SttCandidate> {
    models
        .iter()
        .map(|model| config.resolve_stt_candidate(model))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(base_url: &str) -> SttCandidate {
        SttCandidate {
            provider: "openai".to_owned(),
            model: "whisper-1".to_owned(),
            base_url: base_url.to_owned(),
            api_key: Some("secret".to_owned()),
            auth_header: "Authorization".to_owned(),
            auth_scheme: Some("Bearer".to_owned()),
            request_timeout_seconds: None,
        }
    }

    #[test]
    fn endpoint_appends_transcriptions_to_base_url() {
        assert_eq!(
            candidate("https://api.openai.com/v1/").endpoint(),
            "https://api.openai.com/v1/audio/transcriptions"
        );
        assert_eq!(
            candidate("https://example.test/v1/audio/transcriptions").endpoint(),
            "https://example.test/v1/audio/transcriptions"
        );
    }

    #[test]
    fn debug_never_prints_api_key() {
        let rendered = format!("{:?}", candidate("https://example.test/v1"));
        assert!(rendered.contains("api_key_configured: true"));
        assert!(!rendered.contains("secret"));
    }
}
