//! 模型语音工具到平台发送层的跨层契约。
//!
//! Core 只表达“把这段文本合成语音并发送到当前会话”，不携带 QQ 上传字段、
//! 音频 URL 或平台 token；TTS 协议、音频上传和实际发送仍由 Gateway 实现。

use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;

use crate::runtime::push::PushTarget;

/// 一次语音发送意图。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceSendIntent {
    /// 由服务端根据当前请求生成，不能来自模型参数。
    pub target: PushTarget,
    /// 当前入站消息 ID；存在时优先作为 QQ 被动回复发送。
    pub reply_to_message_id: Option<String>,
    /// 已完成本地清洗和长度校验的朗读文本。
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceSendResult {
    pub message_id: Option<String>,
}

/// 语音发送错误分类。
///
/// 对模型只暴露稳定错误码和可展示文案；Provider 响应体、签名 URL、账号信息
/// 只能留在 Gateway 的脱敏日志中。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VoiceSendError {
    #[error("voice target platform is not supported")]
    UnsupportedPlatform,
    #[error("voice sending is unavailable")]
    Unavailable,
    #[error("voice target is missing")]
    TargetMissing,
    #[error("voice text is empty")]
    EmptyText,
    #[error("voice text exceeds {max_chars} characters")]
    TextTooLong { max_chars: usize },
    #[error("voice send timed out")]
    Timeout,
    #[error("voice synthesis failed: {code}")]
    Tts { code: &'static str },
    #[error("voice upload failed")]
    Upload,
    #[error("voice message send failed")]
    Send,
}

impl VoiceSendError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedPlatform => "unsupported_platform",
            Self::Unavailable => "voice_unavailable",
            Self::TargetMissing => "target_missing",
            Self::EmptyText => "empty_text",
            Self::TextTooLong { .. } => "text_too_long",
            Self::Timeout => "timeout",
            Self::Tts { .. } => "tts_error",
            Self::Upload => "upload_failed",
            Self::Send => "send_failed",
        }
    }

    pub fn user_message(&self) -> String {
        match self {
            Self::UnsupportedPlatform => "当前入口暂不支持发送语音。".to_owned(),
            Self::Unavailable => "语音功能暂不可用，本次未发送。".to_owned(),
            Self::TargetMissing => "当前会话缺少语音发送目标，本次未发送。".to_owned(),
            Self::EmptyText => "语音内容为空，本次未发送。".to_owned(),
            Self::TextTooLong { max_chars } => {
                format!("语音内容过长，最多 {max_chars} 个字符，本次未发送。")
            }
            Self::Timeout => "语音发送超时，本次是否送达无法确认。".to_owned(),
            Self::Tts { .. } => "语音合成失败，本次未发送。".to_owned(),
            Self::Upload => "语音上传失败，本次未发送。".to_owned(),
            Self::Send => "语音发送失败，本次未发送。".to_owned(),
        }
    }
}

/// Gateway 提供的语音发送能力。
///
/// 返回 `Ok` 只代表携带真实 message id 的发送结果；任何失败都必须返回错误，
/// 不能由实现方生成成功文案。
#[async_trait]
pub trait VoiceSendSink: Send + Sync {
    async fn send_voice(&self, intent: VoiceSendIntent) -> Result<VoiceSendResult, VoiceSendError>;
}

pub type DynVoiceSendSink = Arc<dyn VoiceSendSink>;
