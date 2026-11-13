use std::time::Duration;

use serde::Serialize;
use tokio::time::timeout;

use super::{TtsAudio, TtsError, TtsFuture, TtsProvider};

/// OpenAI Speech 固定返回 mp3：QQ 语音支持 mp3，且比 WAV 更适合 `file_data`
/// 直传的大小限制。
const RESPONSE_FORMAT: &str = "mp3";
const FILE_NAME: &str = "voice.mp3";
/// QQ 语音二进制直传上限约 5 MiB；保留编码与请求体余量。
const MAX_AUDIO_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone)]
pub struct OpenAiTtsConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub voice: String,
    pub request_timeout: Duration,
    pub max_text_chars: usize,
}

impl std::fmt::Debug for OpenAiTtsConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OpenAiTtsConfig")
            .field("api_key_configured", &!self.api_key.trim().is_empty())
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("voice", &self.voice)
            .field("request_timeout", &self.request_timeout)
            .field("max_text_chars", &self.max_text_chars)
            .finish()
    }
}

#[derive(Clone)]
pub struct OpenAiTtsProvider {
    client: reqwest::Client,
    pub(super) config: OpenAiTtsConfig,
}

#[derive(Serialize)]
struct OpenAiRequest<'a> {
    model: &'a str,
    input: &'a str,
    voice: &'a str,
    response_format: &'static str,
}

impl OpenAiTtsProvider {
    pub fn new(client: reqwest::Client, config: OpenAiTtsConfig) -> Self {
        Self { client, config }
    }

    async fn synthesize_inner(&self, text: &str) -> Result<TtsAudio, TtsError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(TtsError::EmptyText);
        }
        if text.chars().count() > self.config.max_text_chars {
            return Err(TtsError::TextTooLong {
                max_chars: self.config.max_text_chars,
            });
        }

        // 同一个超时覆盖发送、响应头、完整音频 body 读取和大小校验，避免 Provider
        // 先返回响应头再悬挂 body 时长期占住 QQ 最终回复。
        let request = async {
            let mut response = self
                .client
                .post(&self.config.base_url)
                .bearer_auth(&self.config.api_key)
                .json(&OpenAiRequest {
                    model: &self.config.model,
                    input: text,
                    voice: &self.config.voice,
                    response_format: RESPONSE_FORMAT,
                })
                .send()
                .await
                .map_err(TtsError::Http)?;
            let status = response.status();
            if !status.is_success() {
                // 错误体可能回显请求信息；不读取正文即可完成阶段化分类。
                return Err(TtsError::Status { status });
            }
            // 按块读取并在超限前停止，避免异常大响应占满内存。
            let mut audio = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(TtsError::Http)? {
                if audio.len() + chunk.len() > MAX_AUDIO_BYTES {
                    return Err(TtsError::AudioTooLarge {
                        max_bytes: MAX_AUDIO_BYTES,
                    });
                }
                audio.extend_from_slice(&chunk);
            }
            if audio.is_empty() {
                return Err(TtsError::EmptyAudio);
            }
            Ok(TtsAudio::File {
                bytes: audio,
                file_name: FILE_NAME.to_owned(),
            })
        };
        timeout(self.config.request_timeout, request)
            .await
            .map_err(|_| TtsError::Timeout {
                timeout_seconds: self.config.request_timeout.as_secs(),
            })?
    }
}

impl TtsProvider for OpenAiTtsProvider {
    fn synthesize<'a>(&'a self, text: &'a str) -> TtsFuture<'a> {
        Box::pin(async move { self.synthesize_inner(text).await })
    }
}
