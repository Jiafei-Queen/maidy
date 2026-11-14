//! 入站语音转写（STT）。
//!
//! 音频只在 Gateway 内存中流转，Core/LLM 只接收转写文本。Provider 复用现有
//! OpenAI 兼容连接，只实现 `/audio/transcriptions`；平台自带转写由各 adapter 优先尝试。

use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};

use qq_maid_core::config::{ChatScene, SttCandidate, SttRuntimeConfig};
use thiserror::Error;
use tracing::{debug, info, warn};

mod audio;
mod openai;

pub use audio::{AudioFormat, SttAudio, detect_audio_format};
pub use openai::OpenAiSttClient;

pub type SttFuture<'a> = Pin<Box<dyn Future<Output = Result<String, SttError>> + Send + 'a>>;

/// 单个 Provider 的转录传输层；测试可注入脚本化实现验证候选降级顺序。
pub trait SttTransport: Send + Sync {
    fn transcribe<'a>(
        &'a self,
        candidate: &'a SttCandidate,
        audio: &'a SttAudio,
        language: Option<&'a str>,
        timeout: Duration,
    ) -> SttFuture<'a>;
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SttError {
    #[error("STT is not configured")]
    NotConfigured,
    #[error("STT audio download failed")]
    DownloadFailed,
    #[error("STT provider request failed")]
    RequestFailed,
    #[error("STT audio exceeds size limit")]
    AudioTooLarge,
    #[error("STT audio format is not supported")]
    UnsupportedFormat,
    #[error("SILK audio cannot be uploaded without platform transcoding")]
    SilkNotSupported,
    #[error("STT provider returned HTTP status {status}")]
    Status { status: u16 },
    #[error("STT request timed out")]
    Timeout,
    #[error("STT provider returned no usable text")]
    EmptyTranscript,
    #[error("STT provider response was invalid")]
    InvalidResponse,
    #[error("no STT candidate is available")]
    NoCandidate,
}

impl SttError {
    /// 稳定的错误分类，用于结构化日志；不包含响应正文或音频信息。
    pub fn kind(&self) -> &'static str {
        match self {
            Self::NotConfigured => "not_configured",
            Self::DownloadFailed => "download_failed",
            Self::RequestFailed => "request_failed",
            Self::AudioTooLarge => "audio_too_large",
            Self::UnsupportedFormat => "unsupported_format",
            Self::SilkNotSupported => "silk_not_supported",
            Self::Status { .. } => "provider_status",
            Self::Timeout => "timeout",
            Self::EmptyTranscript => "empty_transcript",
            Self::InvalidResponse => "invalid_response",
            Self::NoCandidate => "no_candidate",
        }
    }
}

/// 已就绪的 STT 服务；`None` 表示未启用或没有任何候选凭证。
#[derive(Clone)]
pub struct SttService {
    config: Arc<SttRuntimeConfig>,
    client: reqwest::Client,
    transport: Arc<dyn SttTransport>,
}

impl std::fmt::Debug for SttService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SttService")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl SttService {
    /// 仅在 `enabled=true` 且至少一个候选配置了密钥时创建客户端。
    pub fn from_config(config: &SttRuntimeConfig) -> Option<Arc<Self>> {
        if !config.is_usable() {
            return None;
        }
        let client = qq_maid_common::http_client::client();
        Some(Arc::new(Self {
            config: Arc::new(config.clone()),
            transport: Arc::new(OpenAiSttClient::new(client.clone())),
            client,
        }))
    }

    #[cfg(test)]
    pub(crate) fn with_transport(
        config: SttRuntimeConfig,
        transport: Arc<dyn SttTransport>,
    ) -> Arc<Self> {
        Arc::new(Self {
            config: Arc::new(config),
            client: qq_maid_common::http_client::client(),
            transport,
        })
    }

    pub fn config(&self) -> &SttRuntimeConfig {
        &self.config
    }

    /// 下载并转写远程音频；下载阶段同样受音频上限与超时约束。
    pub async fn transcribe_url(&self, scene: ChatScene, url: &str) -> Result<String, SttError> {
        let audio = audio::download_audio(
            &self.client,
            url,
            self.config.max_audio_bytes,
            self.config.timeout,
        )
        .await?;
        self.transcribe_audio(scene, audio).await
    }

    /// 转写已在内存中的音频；上传前统一做格式嗅探，避免把 SILK 直接发给服务商。
    pub async fn transcribe_bytes(
        &self,
        scene: ChatScene,
        bytes: Vec<u8>,
        filename_hint: Option<&str>,
    ) -> Result<String, SttError> {
        let audio = audio::audio_from_bytes(bytes, filename_hint, self.config.max_audio_bytes)?;
        self.transcribe_audio(scene, audio).await
    }

    async fn transcribe_audio(
        &self,
        scene: ChatScene,
        audio: SttAudio,
    ) -> Result<String, SttError> {
        let mut last_error = SttError::NoCandidate;
        for (index, candidate) in self.config.chain(scene).iter().enumerate() {
            if candidate
                .api_key
                .as_deref()
                .map(str::trim)
                .unwrap_or_default()
                .is_empty()
            {
                debug!(
                    provider = %candidate.provider,
                    model = %candidate.model,
                    chain_index = index,
                    error_class = "missing_credentials",
                    "STT 候选缺少凭证，已跳过"
                );
                continue;
            }
            let timeout = candidate.request_timeout(self.config.timeout);
            let started = Instant::now();
            match self
                .transport
                .transcribe(candidate, &audio, self.config.language.as_deref(), timeout)
                .await
            {
                Ok(text) => {
                    let text = clean_transcript(&text, self.config.max_text_chars)?;
                    info!(
                        scene = scene.as_str(),
                        provider = %candidate.provider,
                        model = %candidate.model,
                        chain_index = index,
                        duration_ms = started.elapsed().as_millis() as u64,
                        audio_bytes = audio.bytes.len(),
                        transcript_chars = text.chars().count(),
                        "语音转写成功"
                    );
                    return Ok(text);
                }
                Err(error) => {
                    debug!(
                        scene = scene.as_str(),
                        provider = %candidate.provider,
                        model = %candidate.model,
                        chain_index = index,
                        duration_ms = started.elapsed().as_millis() as u64,
                        error_class = error.kind(),
                        "STT 候选失败，尝试下一候选"
                    );
                    last_error = error;
                }
            }
        }
        warn!(
            scene = scene.as_str(),
            error_class = last_error.kind(),
            "STT 候选链全部失败"
        );
        Err(last_error)
    }
}

/// 清理 Provider 返回文本：去掉控制字符、折叠空白并按字符边界截断。
fn clean_transcript(value: &str, max_chars: usize) -> Result<String, SttError> {
    let mut cleaned = String::with_capacity(value.len().min(max_chars * 4));
    let mut pending_space = false;
    let mut chars = 0usize;
    for ch in value.chars() {
        if ch.is_control() || ch.is_whitespace() {
            pending_space = !cleaned.is_empty();
            continue;
        }
        if pending_space {
            cleaned.push(' ');
            pending_space = false;
            chars += 1;
        }
        cleaned.push(ch);
        chars += 1;
        if chars >= max_chars {
            break;
        }
    }
    let cleaned = cleaned.trim().to_owned();
    if cleaned.is_empty() {
        return Err(SttError::EmptyTranscript);
    }
    Ok(cleaned)
}

#[cfg(test)]
mod tests;
