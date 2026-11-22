//! STT 运行配置：把 `[stt]` 候选链解析为 Gateway 可直接消费的执行步骤。
//!
//! 语音音频只在 Gateway 流转；远程候选只暴露 OpenAI 兼容转录所需的 Base URL、
//! 认证方式和密钥，平台候选仅保留 `platform:qq` 标记，不复制模型路由或 Provider 管理状态。

use std::time::Duration;

use qq_maid_llm::provider::types::{ModelId, ModelProvider};

use super::{
    AppConfig, ChatScene, SttPlatformAsrMode, agent::AgentProviderKind, env_optional,
    is_platform_stt_model,
};

const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";

/// STT 链整体状态；缺少凭证不阻止启动，只跳过远程候选。
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

/// 转录链路中的一步：平台自带转写不需要音频上传，远程候选复用 OpenAI 兼容协议。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SttStep {
    /// QQ 官方 `asr_refer_text` 或 OneBot/NapCat `fetch_ptt_text`。
    Platform,
    /// OpenAI 兼容 `/audio/transcriptions` 候选。
    Remote(SttCandidate),
}

/// Gateway 使用的 STT 快照；`Debug` 只暴露密钥是否配置。
#[derive(Clone, PartialEq, Eq)]
pub struct SttRuntimeConfig {
    pub enabled: bool,
    pub status: SttRuntimeStatus,
    /// 旧配置的兼容开关；新链路由 `platform:qq` 候选显式表达。
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
    pub default: Vec<SttStep>,
    pub private: Vec<SttStep>,
    pub group: Vec<SttStep>,
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
    pub fn chain(&self, scene: ChatScene) -> &[SttStep] {
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

    /// 实际执行的链路：显式平台候选保留原顺序；缺少平台候选的旧配置仍按
    /// `platform_asr = "auto"` 前置一次平台转写，保持升级前行为。
    ///
    /// 只要任一条链显式包含 `platform:qq`，就不再为其它链自动补平台候选，
    /// 这样用户从某条链删除平台转录后不会又被旧开关隐式加回来。
    pub fn steps(&self, scene: ChatScene) -> Vec<SttStep> {
        let chain = self.chain(scene);
        let has_platform = self.has_platform_candidate();
        let mut steps = Vec::with_capacity(chain.len() + 1);
        if !has_platform && self.platform_asr == SttPlatformAsrMode::Auto {
            steps.push(SttStep::Platform);
        }
        for step in chain {
            match step {
                SttStep::Platform => steps.push(SttStep::Platform),
                SttStep::Remote(candidate) if self.enabled => {
                    steps.push(SttStep::Remote(candidate.clone()));
                }
                // 关闭外部 STT 时仍保留平台转写；远程候选按全局开关停用。
                SttStep::Remote(_) => {}
            }
        }
        steps
    }

    /// 是否存在显式 `platform:qq` 候选；用于旧 `platform_asr` 兼容判断。
    pub fn has_platform_candidate(&self) -> bool {
        self.routes
            .default
            .iter()
            .chain(self.routes.private.iter())
            .chain(self.routes.group.iter())
            .any(|step| matches!(step, SttStep::Platform))
    }

    /// 是否存在远程转录候选；平台候选不创建 HTTP 客户端。
    pub fn has_remote_candidates(&self) -> bool {
        self.remote_candidates().next().is_some()
    }

    /// 是否存在至少一个带凭证的远程候选。
    pub fn has_remote_credentials(&self) -> bool {
        self.remote_candidates().any(|candidate| {
            candidate
                .api_key
                .as_deref()
                .is_some_and(|key| !key.trim().is_empty())
        })
    }

    fn remote_candidates(&self) -> impl Iterator<Item = &SttCandidate> {
        self.routes
            .default
            .iter()
            .chain(self.routes.private.iter())
            .chain(self.routes.group.iter())
            .filter_map(|step| match step {
                SttStep::Remote(candidate) => Some(candidate),
                SttStep::Platform => None,
            })
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
        let routes = SttRouteSet {
            default: resolve_chain(self, &settings.routes.default),
            private: resolve_chain(self, &settings.routes.private),
            group: resolve_chain(self, &settings.routes.group),
        };
        let mut runtime = SttRuntimeConfig {
            enabled: settings.enabled,
            status: SttRuntimeStatus::Disabled,
            platform_asr: settings.platform_asr,
            language: settings.language.clone(),
            timeout: Duration::from_secs(settings.timeout_seconds),
            platform_asr_attempts: settings.platform_asr_attempts,
            max_audio_bytes: settings.max_audio_bytes,
            max_text_chars: settings.max_text_chars,
            routes,
        };
        runtime.status = if !settings.enabled {
            SttRuntimeStatus::Disabled
        } else if runtime.has_platform_candidate() || runtime.has_remote_credentials() {
            SttRuntimeStatus::Available
        } else {
            SttRuntimeStatus::MissingCredentials
        };
        runtime
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
                        // 已停用的自定义连接跳过远程转写，与内置连接停用语义保持一致。
                        if configured.enabled {
                            env_optional(&configured.api_key_env)
                        } else {
                            None
                        },
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

fn resolve_chain(config: &AppConfig, models: &[ModelId]) -> Vec<SttStep> {
    models
        .iter()
        .map(|model| {
            if is_platform_stt_model(model) {
                SttStep::Platform
            } else {
                SttStep::Remote(config.resolve_stt_candidate(model))
            }
        })
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

    fn steps_config(
        steps: Vec<SttStep>,
        enabled: bool,
        mode: SttPlatformAsrMode,
    ) -> SttRuntimeConfig {
        SttRuntimeConfig {
            enabled,
            status: SttRuntimeStatus::Available,
            platform_asr: mode,
            language: None,
            timeout: Duration::from_secs(30),
            platform_asr_attempts: 2,
            max_audio_bytes: 1024 * 1024,
            max_text_chars: 2000,
            routes: SttRouteSet {
                default: steps,
                private: Vec::new(),
                group: Vec::new(),
            },
        }
    }

    #[test]
    fn explicit_platform_candidate_keeps_chain_order() {
        let config = steps_config(
            vec![
                SttStep::Remote(candidate("https://example.test/v1")),
                SttStep::Platform,
            ],
            true,
            SttPlatformAsrMode::Auto,
        );
        assert_eq!(
            config.steps(ChatScene::Private),
            vec![
                SttStep::Remote(candidate("https://example.test/v1")),
                SttStep::Platform,
            ]
        );
        assert!(config.has_platform_candidate());
    }

    #[test]
    fn explicit_platform_candidate_works_with_legacy_field_off() {
        let config = steps_config(
            vec![
                SttStep::Platform,
                SttStep::Remote(candidate("https://example.test/v1")),
            ],
            true,
            SttPlatformAsrMode::Off,
        );
        assert_eq!(
            config.steps(ChatScene::Private),
            vec![
                SttStep::Platform,
                SttStep::Remote(candidate("https://example.test/v1")),
            ]
        );
    }

    #[test]
    fn legacy_auto_prepends_platform_when_chain_has_no_platform_candidate() {
        let config = steps_config(
            vec![SttStep::Remote(candidate("https://example.test/v1"))],
            true,
            SttPlatformAsrMode::Auto,
        );
        let steps = config.steps(ChatScene::Group);
        assert!(matches!(steps.first(), Some(SttStep::Platform)));
        assert_eq!(steps.len(), 2);
    }

    #[test]
    fn removing_platform_from_one_chain_does_not_reintroduce_legacy_step() {
        let mut config = steps_config(vec![SttStep::Platform], true, SttPlatformAsrMode::Auto);
        config.routes.private = vec![SttStep::Remote(candidate("https://example.test/v1"))];
        assert_eq!(
            config.steps(ChatScene::Private),
            vec![SttStep::Remote(candidate("https://example.test/v1"))]
        );
    }

    #[test]
    fn disabled_stt_keeps_platform_step_but_skips_remote_candidates() {
        let config = steps_config(
            vec![
                SttStep::Platform,
                SttStep::Remote(candidate("https://example.test/v1")),
            ],
            false,
            SttPlatformAsrMode::Auto,
        );
        assert_eq!(config.steps(ChatScene::Private), vec![SttStep::Platform]);
    }

    #[test]
    fn disabled_legacy_auto_still_allows_platform_only() {
        let config = steps_config(Vec::new(), false, SttPlatformAsrMode::Auto);
        assert_eq!(config.steps(ChatScene::Private), vec![SttStep::Platform]);
        let config = steps_config(Vec::new(), false, SttPlatformAsrMode::Off);
        assert!(config.steps(ChatScene::Private).is_empty());
    }
}
