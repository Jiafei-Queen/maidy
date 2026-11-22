//! 语音转写（STT）的 Agent 策略配置。
//!
//! `[stt]` 与 `[model_routes.*]` 同构：只保存非敏感的转录模型候选链与运行参数，
//! STT 提供商复用现有 `[providers.*]` / 内置连接，密钥仍由环境变量提供。

use serde::{Deserialize, Serialize};

use qq_maid_llm::provider::types::{ModelId, ModelProvider};

use crate::error::LlmError;

use super::{AgentProviderConfig, AgentProviderKind, ChatScene};

pub const DEFAULT_STT_TIMEOUT_SECONDS: u64 = 30;
pub const DEFAULT_STT_PLATFORM_ASR_ATTEMPTS: u8 = 2;
pub const DEFAULT_STT_MAX_AUDIO_BYTES: u64 = 10 * 1024 * 1024;
pub const DEFAULT_STT_MAX_TEXT_CHARS: usize = 2000;
pub const MAX_STT_TIMEOUT_SECONDS: u64 = 120;
pub const MAX_STT_PLATFORM_ASR_ATTEMPTS: u8 = 3;
pub const MIN_STT_MAX_AUDIO_BYTES: u64 = 64 * 1024;
pub const MAX_STT_MAX_AUDIO_BYTES: u64 = 12 * 1024 * 1024;
pub const MAX_STT_MAX_TEXT_CHARS: usize = 8000;
pub const MAX_STT_LANGUAGE_CHARS: usize = 16;

/// 平台自带转写在转录链路中的保留 Provider 名。
///
/// `platform:qq` 不是真实 HTTP Provider：它表示 QQ 官方 `asr_refer_text` 或
/// OneBot/NapCat `fetch_ptt_text`。使用保留前缀可以继续复用 `provider:model`
/// 候选格式，同时避免与用户声明的普通 Provider 混淆。
pub const PLATFORM_STT_PROVIDER: &str = "platform";
pub const PLATFORM_STT_QQ_MODEL: &str = "qq";
pub const PLATFORM_STT_QQ_CANDIDATE: &str = "platform:qq";

/// 判断候选是否为 QQ 平台自带转写。
pub fn is_platform_stt_model(model: &ModelId) -> bool {
    matches!(
        model.provider.as_ref(),
        Some(ModelProvider::Custom(name)) if name == PLATFORM_STT_PROVIDER
    ) && model.name == PLATFORM_STT_QQ_MODEL
}

/// 旧版平台自带语音转写策略；新配置使用候选链中的 `platform:qq`。
///
/// 该字段仅在链路没有显式平台候选时提供升级兼容，避免旧配置升级后丢失平台转写。
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SttPlatformAsrMode {
    /// 旧配置默认值：链路没有 `platform:qq` 时前置一次平台转写。
    #[default]
    Auto,
    /// 不自动补平台转写；显式 `platform:qq` 候选仍按链路执行。
    Off,
}

impl SttPlatformAsrMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Off => "off",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(in crate::config) struct SttConfigFile {
    #[serde(default)]
    pub(in crate::config) enabled: bool,
    #[serde(default)]
    pub(in crate::config) platform_asr: SttPlatformAsrMode,
    #[serde(default)]
    pub(in crate::config) language: String,
    #[serde(default = "default_stt_timeout_seconds")]
    pub(in crate::config) timeout_seconds: u64,
    #[serde(default = "default_stt_platform_asr_attempts")]
    pub(in crate::config) platform_asr_attempts: u8,
    #[serde(default = "default_stt_max_audio_bytes")]
    pub(in crate::config) max_audio_bytes: u64,
    #[serde(default = "default_stt_max_text_chars")]
    pub(in crate::config) max_text_chars: usize,
    #[serde(default)]
    pub(in crate::config) routes: SttRoutesFile,
}

impl Default for SttConfigFile {
    fn default() -> Self {
        Self {
            enabled: false,
            platform_asr: SttPlatformAsrMode::Auto,
            language: String::new(),
            timeout_seconds: DEFAULT_STT_TIMEOUT_SECONDS,
            platform_asr_attempts: DEFAULT_STT_PLATFORM_ASR_ATTEMPTS,
            max_audio_bytes: DEFAULT_STT_MAX_AUDIO_BYTES,
            max_text_chars: DEFAULT_STT_MAX_TEXT_CHARS,
            routes: SttRoutesFile::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(in crate::config) struct SttRoutesFile {
    #[serde(default)]
    pub(in crate::config) default: Vec<String>,
    #[serde(default)]
    pub(in crate::config) private: Vec<String>,
    #[serde(default)]
    pub(in crate::config) group: Vec<String>,
}

/// 已解析的转录模型候选链；空数组保留“该场景未单独配置”的语义。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SttRouteChains {
    pub default: Vec<ModelId>,
    pub private: Vec<ModelId>,
    pub group: Vec<ModelId>,
}

impl SttRouteChains {
    /// 场景链为空时回退 `default`，避免漏配某个场景就完全失去转录能力。
    pub fn for_scene(&self, scene: ChatScene) -> &[ModelId] {
        let chain = match scene {
            ChatScene::Private => &self.private,
            ChatScene::Group => &self.group,
        };
        if chain.is_empty() {
            &self.default
        } else {
            chain
        }
    }

    pub fn is_empty(&self) -> bool {
        self.default.is_empty() && self.private.is_empty() && self.group.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SttSettings {
    pub enabled: bool,
    pub platform_asr: SttPlatformAsrMode,
    pub language: Option<String>,
    pub timeout_seconds: u64,
    pub platform_asr_attempts: u8,
    pub max_audio_bytes: u64,
    pub max_text_chars: usize,
    pub routes: SttRouteChains,
}

impl Default for SttSettings {
    fn default() -> Self {
        let file = SttConfigFile::default();
        Self {
            enabled: file.enabled,
            platform_asr: file.platform_asr,
            language: None,
            timeout_seconds: file.timeout_seconds,
            platform_asr_attempts: file.platform_asr_attempts,
            max_audio_bytes: file.max_audio_bytes,
            max_text_chars: file.max_text_chars,
            routes: SttRouteChains::default(),
        }
    }
}

impl SttSettings {
    pub(in crate::config) fn from_file(
        file: SttConfigFile,
        providers: &std::collections::HashMap<String, AgentProviderConfig>,
    ) -> Result<Self, LlmError> {
        validate_range(
            "stt.timeout_seconds",
            file.timeout_seconds,
            1,
            MAX_STT_TIMEOUT_SECONDS,
        )?;
        validate_range(
            "stt.platform_asr_attempts",
            file.platform_asr_attempts as u64,
            1,
            MAX_STT_PLATFORM_ASR_ATTEMPTS as u64,
        )?;
        validate_range(
            "stt.max_audio_bytes",
            file.max_audio_bytes,
            MIN_STT_MAX_AUDIO_BYTES,
            MAX_STT_MAX_AUDIO_BYTES,
        )?;
        validate_range(
            "stt.max_text_chars",
            file.max_text_chars as u64,
            1,
            MAX_STT_MAX_TEXT_CHARS as u64,
        )?;
        let language = validate_language(file.language)?;
        let routes = SttRouteChains {
            default: parse_candidates(
                "stt.routes.default",
                &file.routes.default,
                providers,
                file.enabled,
            )?,
            private: parse_candidates(
                "stt.routes.private",
                &file.routes.private,
                providers,
                file.enabled,
            )?,
            group: parse_candidates(
                "stt.routes.group",
                &file.routes.group,
                providers,
                file.enabled,
            )?,
        };
        if file.enabled && routes.is_empty() {
            return Err(LlmError::config(
                "stt is enabled but no transcription model route is configured; set stt.routes.default or a scene route",
            ));
        }
        Ok(Self {
            enabled: file.enabled,
            platform_asr: file.platform_asr,
            language,
            timeout_seconds: file.timeout_seconds,
            platform_asr_attempts: file.platform_asr_attempts,
            max_audio_bytes: file.max_audio_bytes,
            max_text_chars: file.max_text_chars,
            routes,
        })
    }

    /// 暴露全部已解析候选，供启动期校验内置 Provider 是否被显式停用。
    pub fn candidate_models(&self) -> impl Iterator<Item = &ModelId> {
        self.routes
            .default
            .iter()
            .chain(self.routes.private.iter())
            .chain(self.routes.group.iter())
    }
}

/// 校验转录候选：必须显式 `provider:model`；启用 STT 时再校验 Provider 存在、启用且为
/// OpenAI 兼容，停用状态允许保留历史候选，避免删除 Provider 后无法关闭功能。
fn parse_candidates(
    field_name: &str,
    values: &[String],
    providers: &std::collections::HashMap<String, AgentProviderConfig>,
    validate_providers: bool,
) -> Result<Vec<ModelId>, LlmError> {
    let mut candidates = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let location = format!("{field_name}[{index}]");
        let model = ModelId::parse_config(value, &location)?;
        let Some(provider) = model.provider.clone() else {
            return Err(LlmError::config(format!(
                "invalid {location}: transcription candidate must use provider:model"
            )));
        };
        // `platform:qq` 是内置平台转写占位，不需要也不允许映射到真实 Provider。
        if matches!(&provider, ModelProvider::Custom(name) if name == PLATFORM_STT_PROVIDER) {
            if model.name != PLATFORM_STT_QQ_MODEL {
                return Err(LlmError::config(format!(
                    "{location} references unknown platform transcription model `{}`; only {PLATFORM_STT_QQ_CANDIDATE} is supported",
                    model.name
                )));
            }
        } else if validate_providers {
            match &provider {
                ModelProvider::OpenAi
                | ModelProvider::DeepSeek
                | ModelProvider::BigModel
                | ModelProvider::Gemini => {}
                ModelProvider::Custom(name) => {
                    let Some(configured) = providers.values().find(|item| item.id.as_str() == name)
                    else {
                        return Err(LlmError::config(format!(
                            "{location} references custom provider `{name}`, but providers.{name} is not configured"
                        )));
                    };
                    if configured.kind != AgentProviderKind::OpenAiCompatible {
                        return Err(LlmError::config(format!(
                            "{location} references provider `{name}` with kind openai_responses; transcription requires an openai_compatible provider"
                        )));
                    }
                }
            }
        }
        if candidates
            .iter()
            .any(|existing: &ModelId| existing == &model)
        {
            return Err(LlmError::config(format!(
                "invalid {field_name}: duplicate transcription candidate `{}`",
                value.trim()
            )));
        }
        candidates.push(model);
    }
    Ok(candidates)
}

fn validate_language(value: String) -> Result<Option<String>, LlmError> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.chars().count() > MAX_STT_LANGUAGE_CHARS
        || value
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return Err(LlmError::config(
            "stt.language must be at most 16 visible characters without whitespace",
        ));
    }
    Ok(Some(value.to_owned()))
}

fn validate_range(field: &str, value: u64, min: u64, max: u64) -> Result<(), LlmError> {
    if value < min || value > max {
        return Err(LlmError::config(format!(
            "{field} must be between {min} and {max}, got {value}"
        )));
    }
    Ok(())
}

fn default_stt_timeout_seconds() -> u64 {
    DEFAULT_STT_TIMEOUT_SECONDS
}

fn default_stt_platform_asr_attempts() -> u8 {
    DEFAULT_STT_PLATFORM_ASR_ATTEMPTS
}

fn default_stt_max_audio_bytes() -> u64 {
    DEFAULT_STT_MAX_AUDIO_BYTES
}

fn default_stt_max_text_chars() -> usize {
    DEFAULT_STT_MAX_TEXT_CHARS
}
