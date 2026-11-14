//! TTS 试听的跨层契约。
//!
//! Web 控制台需要在经过 QQ 发送前验证配置中心已保存的 TTS 配置；Core 只定义
//! 试听请求、可播放音频结果和错误分类，具体 Provider 协议与音频下载仍由
//! Gateway 实现。根入口在装配时注入 Gateway 的试听服务，避免 Core 反向依赖
//! Gateway。

use std::{future::Future, pin::Pin};

use crate::config::VoiceFeatureConfig;

/// 浏览器可直接播放的音频格式。
///
/// 只保留当前 Provider 实际可能返回的格式，避免把上游 `Content-Type` 原文
/// 直接透传到响应头。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoicePreviewFormat {
    Mp3,
    Wav,
}

impl VoicePreviewFormat {
    pub fn content_type(self) -> &'static str {
        match self {
            Self::Mp3 => "audio/mpeg",
            Self::Wav => "audio/wav",
        }
    }

    pub fn file_name(self) -> &'static str {
        match self {
            Self::Mp3 => "voice-preview.mp3",
            Self::Wav => "voice-preview.wav",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoicePreviewAudio {
    pub bytes: Vec<u8>,
    pub format: VoicePreviewFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoicePreviewError {
    Unavailable,
    EmptyText,
    TextTooLong { max_chars: usize },
    Timeout { timeout_seconds: u64 },
    RequestFailed,
    HttpStatus { status: u16 },
    ProviderStatus,
    InvalidResponse,
    InvalidAudioUrl,
    EmptyAudio,
    AudioTooLarge { max_bytes: usize },
    AudioFetchFailed,
    AudioFetchStatus { status: u16 },
}

impl VoicePreviewError {
    /// 日志与 API 只暴露稳定分类，不拼接 Provider 响应体或签名 URL。
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unavailable => "voice_unavailable",
            Self::EmptyText => "empty_text",
            Self::TextTooLong { .. } => "text_too_long",
            Self::Timeout { .. } => "timeout",
            Self::RequestFailed => "request_failed",
            Self::HttpStatus { .. } => "http_status",
            Self::ProviderStatus => "provider_status",
            Self::InvalidResponse => "invalid_response",
            Self::InvalidAudioUrl => "invalid_audio_url",
            Self::EmptyAudio => "empty_audio",
            Self::AudioTooLarge { .. } => "audio_too_large",
            Self::AudioFetchFailed => "audio_fetch_failed",
            Self::AudioFetchStatus { .. } => "audio_fetch_status",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::Unavailable => {
                "语音功能当前不可用：请检查 TTS Provider、API Key 与 Base URL 配置".to_owned()
            }
            Self::EmptyText => "试听文本不能为空".to_owned(),
            Self::TextTooLong { max_chars } => {
                format!("试听文本最多 {max_chars} 个字符")
            }
            Self::Timeout { timeout_seconds } => {
                format!("TTS 请求超时（{timeout_seconds} 秒）")
            }
            Self::RequestFailed => "请求 TTS Provider 失败，请检查网络与 Base URL".to_owned(),
            Self::HttpStatus { status } => {
                format!("TTS 服务返回 HTTP {status}；请检查模型、音色、API Key 和账户余额")
            }
            Self::ProviderStatus => "TTS 服务返回业务错误，请检查模型与音色配置".to_owned(),
            Self::InvalidResponse => "TTS 服务返回了无法识别的响应".to_owned(),
            Self::InvalidAudioUrl => "TTS 服务返回的音频地址无效".to_owned(),
            Self::EmptyAudio => "TTS 服务返回了空音频".to_owned(),
            Self::AudioTooLarge { max_bytes } => {
                format!("TTS 音频超过 {max_bytes} 字节，已停止试听")
            }
            Self::AudioFetchFailed => "下载 TTS 音频失败，请检查音频地址是否可访问".to_owned(),
            Self::AudioFetchStatus { status } => {
                format!("下载 TTS 音频失败（HTTP {status}）")
            }
        }
    }
}

pub type VoicePreviewFuture<'a> =
    Pin<Box<dyn Future<Output = Result<VoicePreviewAudio, VoicePreviewError>> + Send + 'a>>;

/// 管理面试听入口；实现方按传入的已保存配置构造 Provider。
pub trait VoicePreviewService: Send + Sync {
    fn synthesize<'a>(
        &'a self,
        config: &'a VoiceFeatureConfig,
        text: &'a str,
    ) -> VoicePreviewFuture<'a>;
}

pub type DynVoicePreviewService = std::sync::Arc<dyn VoicePreviewService>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_messages_are_stable_and_do_not_include_audio_urls() {
        assert_eq!(VoicePreviewError::Unavailable.code(), "voice_unavailable");
        assert_eq!(
            VoicePreviewError::HttpStatus { status: 400 }.message(),
            "TTS 服务返回 HTTP 400；请检查模型、音色、API Key 和账户余额"
        );
        assert_eq!(
            VoicePreviewError::TextTooLong { max_chars: 600 }.message(),
            "试听文本最多 600 个字符"
        );
    }
}
