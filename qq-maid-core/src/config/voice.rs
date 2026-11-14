//! 最终回复语音投递的共享功能配置。
//!
//! Core 用预检状态决定 `/语音 开启` 是否允许写入，Gateway 使用同一快照构造具体
//! TTS Provider。无效 TTS 配置只关闭语音能力，避免破坏既有文本机器人启动。

use std::{collections::HashMap, time::Duration};

pub const DEFAULT_QWEN_TTS_BASE_URL: &str =
    "https://dashscope.aliyuncs.com/api/v1/services/aigc/multimodal-generation/generation";
pub const DEFAULT_QWEN_TTS_MODEL: &str = "qwen3-tts-flash";
pub const DEFAULT_QWEN_TTS_VOICE: &str = "Cherry";
pub const DEFAULT_OPENAI_TTS_BASE_URL: &str = "https://api.openai.com/v1/audio/speech";
pub const DEFAULT_OPENAI_TTS_MODEL: &str = "tts-1";
pub const DEFAULT_OPENAI_TTS_VOICE: &str = "alloy";
pub const DEFAULT_TTS_REQUEST_TIMEOUT_SECONDS: u64 = 30;
pub const DEFAULT_TTS_MAX_TEXT_CHARS: usize = 600;
const MAX_TTS_REQUEST_TIMEOUT_SECONDS: u64 = 120;
const MAX_TTS_TEXT_CHARS: usize = 600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TtsProviderMode {
    Disabled,
    Qwen,
    OpenAi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceFeatureStatus {
    Disabled,
    Available,
    Invalid(VoicePreflightError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoicePreflightError {
    UnsupportedProvider,
    MissingApiKey,
    InvalidBaseUrl,
    InvalidRequestTimeout,
    InvalidTextLimit,
}

#[derive(Clone, PartialEq, Eq)]
pub struct VoiceFeatureConfig {
    pub provider: TtsProviderMode,
    pub status: VoiceFeatureStatus,
    pub qwen_api_key: Option<String>,
    pub qwen_base_url: String,
    pub qwen_model: String,
    pub qwen_voice: String,
    pub openai_api_key: Option<String>,
    pub openai_base_url: String,
    pub openai_model: String,
    pub openai_voice: String,
    pub request_timeout: Duration,
    pub max_text_chars: usize,
}

impl std::fmt::Debug for VoiceFeatureConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VoiceFeatureConfig")
            .field("provider", &self.provider)
            .field("status", &self.status)
            .field("qwen_api_key_configured", &self.qwen_api_key.is_some())
            .field("qwen_base_url", &self.qwen_base_url)
            .field("qwen_model", &self.qwen_model)
            .field("qwen_voice", &self.qwen_voice)
            .field("openai_api_key_configured", &self.openai_api_key.is_some())
            .field("openai_base_url", &self.openai_base_url)
            .field("openai_model", &self.openai_model)
            .field("openai_voice", &self.openai_voice)
            .field("request_timeout", &self.request_timeout)
            .field("max_text_chars", &self.max_text_chars)
            .finish()
    }
}

impl Default for VoiceFeatureConfig {
    fn default() -> Self {
        Self {
            provider: TtsProviderMode::Disabled,
            status: VoiceFeatureStatus::Disabled,
            qwen_api_key: None,
            qwen_base_url: DEFAULT_QWEN_TTS_BASE_URL.to_owned(),
            qwen_model: DEFAULT_QWEN_TTS_MODEL.to_owned(),
            qwen_voice: DEFAULT_QWEN_TTS_VOICE.to_owned(),
            openai_api_key: None,
            openai_base_url: DEFAULT_OPENAI_TTS_BASE_URL.to_owned(),
            openai_model: DEFAULT_OPENAI_TTS_MODEL.to_owned(),
            openai_voice: DEFAULT_OPENAI_TTS_VOICE.to_owned(),
            request_timeout: Duration::from_secs(DEFAULT_TTS_REQUEST_TIMEOUT_SECONDS),
            max_text_chars: DEFAULT_TTS_MAX_TEXT_CHARS,
        }
    }
}

impl VoiceFeatureConfig {
    pub fn from_environment(environment: &HashMap<String, String>) -> Self {
        let mut config = Self::default();
        let provider = optional(environment, "TTS_PROVIDER")
            .unwrap_or_else(|| "disabled".to_owned())
            .to_ascii_lowercase();
        config.provider = match provider.as_str() {
            "disabled" => return config,
            "qwen" => TtsProviderMode::Qwen,
            "openai" => TtsProviderMode::OpenAi,
            _ => {
                config.status =
                    VoiceFeatureStatus::Invalid(VoicePreflightError::UnsupportedProvider);
                return config;
            }
        };
        match config.provider {
            TtsProviderMode::Qwen => {
                config.qwen_api_key = optional(environment, "QWEN_TTS_API_KEY");
                config.qwen_base_url = optional(environment, "QWEN_TTS_BASE_URL")
                    .unwrap_or_else(|| DEFAULT_QWEN_TTS_BASE_URL.to_owned());
                config.qwen_model = optional(environment, "QWEN_TTS_MODEL")
                    .unwrap_or_else(|| DEFAULT_QWEN_TTS_MODEL.to_owned());
                config.qwen_voice = optional(environment, "QWEN_TTS_VOICE")
                    .unwrap_or_else(|| DEFAULT_QWEN_TTS_VOICE.to_owned());
            }
            TtsProviderMode::OpenAi => {
                config.openai_api_key = optional(environment, "OPENAI_TTS_API_KEY");
                config.openai_base_url = optional(environment, "OPENAI_TTS_BASE_URL")
                    .unwrap_or_else(|| DEFAULT_OPENAI_TTS_BASE_URL.to_owned());
                config.openai_model = optional(environment, "OPENAI_TTS_MODEL")
                    .unwrap_or_else(|| DEFAULT_OPENAI_TTS_MODEL.to_owned());
                config.openai_voice = optional(environment, "OPENAI_TTS_VOICE")
                    .unwrap_or_else(|| DEFAULT_OPENAI_TTS_VOICE.to_owned());
            }
            TtsProviderMode::Disabled => return config,
        }

        let Some(timeout_seconds) = parse_bounded_u64(
            environment,
            "TTS_REQUEST_TIMEOUT_SECONDS",
            DEFAULT_TTS_REQUEST_TIMEOUT_SECONDS,
            1,
            MAX_TTS_REQUEST_TIMEOUT_SECONDS,
        ) else {
            config.status = VoiceFeatureStatus::Invalid(VoicePreflightError::InvalidRequestTimeout);
            return config;
        };
        config.request_timeout = Duration::from_secs(timeout_seconds);
        let Some(max_text_chars) = parse_bounded_usize(
            environment,
            "TTS_MAX_TEXT_CHARS",
            DEFAULT_TTS_MAX_TEXT_CHARS,
            1,
            MAX_TTS_TEXT_CHARS,
        ) else {
            config.status = VoiceFeatureStatus::Invalid(VoicePreflightError::InvalidTextLimit);
            return config;
        };
        config.max_text_chars = max_text_chars;

        let validation = match config.provider {
            TtsProviderMode::Qwen => {
                validate_provider(config.qwen_api_key.as_ref(), &config.qwen_base_url)
            }
            TtsProviderMode::OpenAi => {
                validate_provider(config.openai_api_key.as_ref(), &config.openai_base_url)
            }
            TtsProviderMode::Disabled => Ok(()),
        };
        if let Err(error) = validation {
            config.status = VoiceFeatureStatus::Invalid(error);
            return config;
        }
        config.status = VoiceFeatureStatus::Available;
        config
    }

    pub fn is_available(&self) -> bool {
        self.status == VoiceFeatureStatus::Available
    }

    pub fn enable_rejection_text(&self) -> Option<&'static str> {
        match self.status {
            VoiceFeatureStatus::Available => None,
            VoiceFeatureStatus::Disabled => {
                Some("语音功能当前未启用，请先配置 TTS_PROVIDER=qwen 或 openai")
            }
            VoiceFeatureStatus::Invalid(VoicePreflightError::MissingApiKey) => {
                Some(match self.provider {
                    TtsProviderMode::OpenAi => "语音功能不可用：缺少 OPENAI_TTS_API_KEY",
                    _ => "语音功能不可用：缺少 QWEN_TTS_API_KEY",
                })
            }
            VoiceFeatureStatus::Invalid(VoicePreflightError::InvalidBaseUrl) => {
                Some(match self.provider {
                    TtsProviderMode::OpenAi => "语音功能不可用：OPENAI_TTS_BASE_URL 无效",
                    _ => "语音功能不可用：QWEN_TTS_BASE_URL 无效",
                })
            }
            VoiceFeatureStatus::Invalid(_) => {
                Some("语音功能配置预检失败，请联系管理员检查 TTS 配置")
            }
        }
    }
}

fn validate_provider(api_key: Option<&String>, base_url: &str) -> Result<(), VoicePreflightError> {
    if api_key.is_none() {
        return Err(VoicePreflightError::MissingApiKey);
    }
    let base_url_valid = reqwest::Url::parse(base_url).is_ok_and(|url| {
        url.scheme() == "https" && url.host_str().is_some() && url.username().is_empty()
    });
    if !base_url_valid {
        return Err(VoicePreflightError::InvalidBaseUrl);
    }
    Ok(())
}

fn optional(environment: &HashMap<String, String>, name: &str) -> Option<String> {
    environment
        .get(name)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn parse_bounded_u64(
    environment: &HashMap<String, String>,
    name: &str,
    default: u64,
    min: u64,
    max: u64,
) -> Option<u64> {
    let value = match optional(environment, name) {
        Some(raw) => raw.parse().ok()?,
        None => default,
    };
    (min..=max).contains(&value).then_some(value)
}

fn parse_bounded_usize(
    environment: &HashMap<String, String>,
    name: &str,
    default: usize,
    min: usize,
    max: usize,
) -> Option<usize> {
    let value = match optional(environment, name) {
        Some(raw) => raw.parse().ok()?,
        None => default,
    };
    (min..=max).contains(&value).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_and_invalid_qwen_configs_are_unavailable_without_exposing_secret() {
        let disabled = VoiceFeatureConfig::from_environment(&HashMap::new());
        assert_eq!(disabled.status, VoiceFeatureStatus::Disabled);
        assert_eq!(
            disabled.enable_rejection_text(),
            Some("语音功能当前未启用，请先配置 TTS_PROVIDER=qwen 或 openai")
        );

        let missing_key = VoiceFeatureConfig::from_environment(&HashMap::from([(
            "TTS_PROVIDER".to_owned(),
            "qwen".to_owned(),
        )]));
        assert_eq!(
            missing_key.status,
            VoiceFeatureStatus::Invalid(VoicePreflightError::MissingApiKey)
        );
        assert_eq!(
            missing_key.enable_rejection_text(),
            Some("语音功能不可用：缺少 QWEN_TTS_API_KEY")
        );

        let invalid_url = VoiceFeatureConfig::from_environment(&HashMap::from([
            ("TTS_PROVIDER".to_owned(), "qwen".to_owned()),
            ("QWEN_TTS_API_KEY".to_owned(), "secret-key".to_owned()),
            (
                "QWEN_TTS_BASE_URL".to_owned(),
                "http://localhost/tts".to_owned(),
            ),
        ]));
        assert_eq!(
            invalid_url.status,
            VoiceFeatureStatus::Invalid(VoicePreflightError::InvalidBaseUrl)
        );
        assert!(!format!("{invalid_url:?}").contains("secret-key"));
    }

    #[test]
    fn valid_qwen_config_is_available_with_expected_defaults() {
        let config = VoiceFeatureConfig::from_environment(&HashMap::from([
            ("TTS_PROVIDER".to_owned(), "qwen".to_owned()),
            ("QWEN_TTS_API_KEY".to_owned(), "secret-key".to_owned()),
        ]));

        assert!(config.is_available());
        assert_eq!(config.qwen_model, DEFAULT_QWEN_TTS_MODEL);
        assert_eq!(config.qwen_voice, DEFAULT_QWEN_TTS_VOICE);
        assert_eq!(config.max_text_chars, DEFAULT_TTS_MAX_TEXT_CHARS);
    }

    #[test]
    fn openai_config_uses_its_own_credentials_and_defaults() {
        let missing_key = VoiceFeatureConfig::from_environment(&HashMap::from([(
            "TTS_PROVIDER".to_owned(),
            "openai".to_owned(),
        )]));
        assert_eq!(
            missing_key.status,
            VoiceFeatureStatus::Invalid(VoicePreflightError::MissingApiKey)
        );
        assert_eq!(
            missing_key.enable_rejection_text(),
            Some("语音功能不可用：缺少 OPENAI_TTS_API_KEY")
        );

        let config = VoiceFeatureConfig::from_environment(&HashMap::from([
            ("TTS_PROVIDER".to_owned(), "openai".to_owned()),
            ("OPENAI_TTS_API_KEY".to_owned(), "secret-key".to_owned()),
        ]));
        assert!(config.is_available());
        assert_eq!(config.openai_base_url, DEFAULT_OPENAI_TTS_BASE_URL);
        assert_eq!(config.openai_model, DEFAULT_OPENAI_TTS_MODEL);
        assert_eq!(config.openai_voice, DEFAULT_OPENAI_TTS_VOICE);
        assert!(config.qwen_api_key.is_none());
        assert!(!format!("{config:?}").contains("secret-key"));
    }

    #[test]
    fn openai_base_url_must_use_https_and_report_provider_specific_hint() {
        let config = VoiceFeatureConfig::from_environment(&HashMap::from([
            ("TTS_PROVIDER".to_owned(), "openai".to_owned()),
            ("OPENAI_TTS_API_KEY".to_owned(), "secret-key".to_owned()),
            (
                "OPENAI_TTS_BASE_URL".to_owned(),
                "http://localhost/speech".to_owned(),
            ),
        ]));
        assert_eq!(
            config.status,
            VoiceFeatureStatus::Invalid(VoicePreflightError::InvalidBaseUrl)
        );
        assert_eq!(
            config.enable_rejection_text(),
            Some("语音功能不可用：OPENAI_TTS_BASE_URL 无效")
        );
    }
}
