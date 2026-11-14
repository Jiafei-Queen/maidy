//! 配置中心已保存 TTS 配置的试听实现。
//!
//! 复用 Gateway 出站语音同一条 Provider 构造与合成链路；千问返回的签名音频
//! URL 会由服务端下载后以字节返回，OpenAI Speech 的音频字节直接返回。
//! 试听不经过 QQ 上传或消息发送，只验证 TTS 配置本身。

use std::time::Duration;

use qq_maid_core::{
    config::VoiceFeatureConfig,
    voice_preview::{
        VoicePreviewAudio, VoicePreviewError, VoicePreviewFormat, VoicePreviewFuture,
        VoicePreviewService,
    },
};
use reqwest::header::CONTENT_TYPE;
use tokio::time::timeout;

use super::{TtsAudio, TtsError, provider_from_config};

/// 试听只用于浏览器播放，仍设置硬上限，避免异常大响应占满进程内存。
const MAX_PREVIEW_AUDIO_BYTES: usize = 8 * 1024 * 1024;

/// 无状态试听实现；每次请求使用配置中心当前已保存并解析后的配置。
#[derive(Debug, Clone, Copy, Default)]
pub struct GatewayVoicePreview;

impl GatewayVoicePreview {
    pub fn new() -> Self {
        Self
    }
}

impl VoicePreviewService for GatewayVoicePreview {
    fn synthesize<'a>(
        &'a self,
        config: &'a VoiceFeatureConfig,
        text: &'a str,
    ) -> VoicePreviewFuture<'a> {
        Box::pin(async move {
            let provider = provider_from_config(config).ok_or(VoicePreviewError::Unavailable)?;
            let audio = provider.synthesize(text).await.map_err(map_tts_error)?;
            match audio {
                TtsAudio::File { bytes, file_name } => Ok(VoicePreviewAudio {
                    bytes,
                    format: format_from_file_name(&file_name),
                }),
                TtsAudio::Url(url) => download_audio(&url, config.request_timeout).await,
            }
        })
    }
}

fn map_tts_error(error: TtsError) -> VoicePreviewError {
    match error {
        TtsError::EmptyText => VoicePreviewError::EmptyText,
        TtsError::TextTooLong { max_chars } => VoicePreviewError::TextTooLong { max_chars },
        TtsError::Timeout { timeout_seconds } => VoicePreviewError::Timeout { timeout_seconds },
        TtsError::Http(_) => VoicePreviewError::RequestFailed,
        TtsError::Status { status } => VoicePreviewError::HttpStatus {
            status: status.as_u16(),
        },
        TtsError::ProviderStatus => VoicePreviewError::ProviderStatus,
        TtsError::InvalidResponse => VoicePreviewError::InvalidResponse,
        TtsError::InvalidAudioUrl => VoicePreviewError::InvalidAudioUrl,
        TtsError::EmptyAudio => VoicePreviewError::EmptyAudio,
        TtsError::AudioTooLarge { max_bytes } => VoicePreviewError::AudioTooLarge { max_bytes },
    }
}

async fn download_audio(
    url: &str,
    request_timeout: Duration,
) -> Result<VoicePreviewAudio, VoicePreviewError> {
    let client = qq_maid_common::http_client::client();
    let request = async {
        let mut response = client
            .get(url)
            .send()
            .await
            .map_err(|_| VoicePreviewError::AudioFetchFailed)?;
        let status = response.status();
        if !status.is_success() {
            return Err(VoicePreviewError::AudioFetchStatus {
                status: status.as_u16(),
            });
        }
        let format = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(format_from_content_type)
            .or_else(|| format_from_url(url))
            .unwrap_or(VoicePreviewFormat::Wav);
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| VoicePreviewError::AudioFetchFailed)?
        {
            if bytes.len() + chunk.len() > MAX_PREVIEW_AUDIO_BYTES {
                return Err(VoicePreviewError::AudioTooLarge {
                    max_bytes: MAX_PREVIEW_AUDIO_BYTES,
                });
            }
            bytes.extend_from_slice(&chunk);
        }
        if bytes.is_empty() {
            return Err(VoicePreviewError::EmptyAudio);
        }
        Ok(VoicePreviewAudio { bytes, format })
    };
    timeout(request_timeout, request)
        .await
        .map_err(|_| VoicePreviewError::Timeout {
            timeout_seconds: request_timeout.as_secs(),
        })?
}

fn format_from_content_type(value: &str) -> Option<VoicePreviewFormat> {
    let value = value.to_ascii_lowercase();
    if value.contains("mpeg") || value.contains("mp3") {
        Some(VoicePreviewFormat::Mp3)
    } else if value.contains("wav") || value.contains("wave") {
        Some(VoicePreviewFormat::Wav)
    } else {
        None
    }
}

fn format_from_file_name(value: &str) -> VoicePreviewFormat {
    if value.to_ascii_lowercase().ends_with(".wav") {
        VoicePreviewFormat::Wav
    } else {
        VoicePreviewFormat::Mp3
    }
}

fn format_from_url(value: &str) -> Option<VoicePreviewFormat> {
    let path = reqwest::Url::parse(value).ok()?.path().to_ascii_lowercase();
    if path.ends_with(".mp3") {
        Some(VoicePreviewFormat::Mp3)
    } else if path.ends_with(".wav") {
        Some(VoicePreviewFormat::Wav)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn available_config() -> VoiceFeatureConfig {
        VoiceFeatureConfig::from_environment(&HashMap::from([
            ("TTS_PROVIDER".to_owned(), "openai".to_owned()),
            ("OPENAI_TTS_API_KEY".to_owned(), "test-key".to_owned()),
            (
                "OPENAI_TTS_BASE_URL".to_owned(),
                "https://example.com/v1/audio/speech".to_owned(),
            ),
            ("OPENAI_TTS_MODEL".to_owned(), "test-tts".to_owned()),
            ("OPENAI_TTS_VOICE".to_owned(), "test-voice".to_owned()),
        ]))
    }

    #[tokio::test]
    async fn unavailable_config_fails_before_any_network_request() {
        let error = GatewayVoicePreview::new()
            .synthesize(&VoiceFeatureConfig::default(), "试听")
            .await
            .unwrap_err();
        assert_eq!(error.code(), "voice_unavailable");
    }

    #[tokio::test]
    async fn available_config_is_redacted_in_debug() {
        let config = available_config();
        assert!(config.is_available());
        // 该用例只验证配置已通过预检；真实网络请求不在单元测试中执行。
        assert_eq!(config.openai_model, "test-tts");
        assert_eq!(config.openai_voice, "test-voice");
        assert!(!format!("{config:?}").contains("test-key"));
    }

    #[test]
    fn detects_browser_playable_formats() {
        assert_eq!(
            format_from_content_type("audio/mpeg"),
            Some(VoicePreviewFormat::Mp3)
        );
        assert_eq!(
            format_from_content_type("audio/x-wav"),
            Some(VoicePreviewFormat::Wav)
        );
        assert_eq!(format_from_content_type("application/octet-stream"), None);
        assert_eq!(format_from_file_name("voice.mp3"), VoicePreviewFormat::Mp3);
        assert_eq!(format_from_file_name("voice.wav"), VoicePreviewFormat::Wav);
        assert_eq!(
            format_from_url("https://example.com/audio/voice.wav?token=1"),
            Some(VoicePreviewFormat::Wav)
        );
        assert_eq!(
            format_from_url("https://example.com/audio/voice.mp3"),
            Some(VoicePreviewFormat::Mp3)
        );
        assert_eq!(format_from_url("https://example.com/audio/voice"), None);
    }

    #[test]
    fn preview_error_mapping_keeps_http_status_for_diagnostics() {
        let error = map_tts_error(TtsError::Status {
            status: reqwest::StatusCode::BAD_REQUEST,
        });
        assert_eq!(error.code(), "http_status");
        assert!(error.message().contains("HTTP 400"));
    }

    #[tokio::test]
    async fn openai_preview_returns_provider_audio_bytes() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                axum::Router::new().route(
                    "/speech",
                    axum::routing::post(|| async {
                        (
                            [(reqwest::header::CONTENT_TYPE, "audio/mpeg")],
                            b"ID3-preview".to_vec(),
                        )
                    }),
                ),
            )
            .await
            .unwrap();
        });
        let mut config = VoiceFeatureConfig::default();
        config.provider = qq_maid_core::config::TtsProviderMode::OpenAi;
        config.status = qq_maid_core::config::VoiceFeatureStatus::Available;
        config.openai_api_key = Some("test-key".to_owned());
        config.openai_base_url = format!("http://{addr}/speech");
        config.openai_model = "test-tts".to_owned();
        config.openai_voice = "test-voice".to_owned();

        let audio = GatewayVoicePreview::new()
            .synthesize(&config, "试听")
            .await
            .unwrap();
        assert_eq!(audio.format, VoicePreviewFormat::Mp3);
        assert_eq!(audio.bytes, b"ID3-preview");
        server.abort();
    }

    #[tokio::test]
    async fn qwen_preview_downloads_the_returned_audio_url() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let audio_url = format!("http://{addr}/voice.wav");
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                axum::Router::new()
                    .route(
                        "/tts",
                        axum::routing::post(move || {
                            let audio_url = audio_url.clone();
                            async move {
                                axum::Json(serde_json::json!({
                                    "output": {"audio": {"url": audio_url}}
                                }))
                            }
                        }),
                    )
                    .route(
                        "/voice.wav",
                        axum::routing::get(|| async {
                            (
                                [(reqwest::header::CONTENT_TYPE, "audio/wav")],
                                b"RIFF-preview".to_vec(),
                            )
                        }),
                    ),
            )
            .await
            .unwrap();
        });
        let mut config = VoiceFeatureConfig::default();
        config.provider = qq_maid_core::config::TtsProviderMode::Qwen;
        config.status = qq_maid_core::config::VoiceFeatureStatus::Available;
        config.qwen_api_key = Some("test-key".to_owned());
        config.qwen_base_url = format!("http://{addr}/tts");
        config.qwen_model = "test-tts".to_owned();
        config.qwen_voice = "test-voice".to_owned();

        let audio = GatewayVoicePreview::new()
            .synthesize(&config, "试听")
            .await
            .unwrap();
        assert_eq!(audio.format, VoicePreviewFormat::Wav);
        assert_eq!(audio.bytes, b"RIFF-preview");
        server.abort();
    }

    #[tokio::test]
    async fn downloads_qwen_signed_audio_url_and_detects_wav() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                axum::Router::new().route(
                    "/voice.wav",
                    axum::routing::get(|| async {
                        (
                            [(reqwest::header::CONTENT_TYPE, "audio/wav")],
                            b"RIFF-preview".to_vec(),
                        )
                    }),
                ),
            )
            .await
            .unwrap();
        });

        let audio = download_audio(&format!("http://{addr}/voice.wav"), Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(audio.format, VoicePreviewFormat::Wav);
        assert_eq!(audio.bytes, b"RIFF-preview");
        server.abort();
    }

    #[tokio::test]
    async fn signed_audio_url_failure_reports_safe_status() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                axum::Router::new().route(
                    "/voice.wav",
                    axum::routing::get(|| async { reqwest::StatusCode::FORBIDDEN }),
                ),
            )
            .await
            .unwrap();
        });

        let error = download_audio(&format!("http://{addr}/voice.wav"), Duration::from_secs(5))
            .await
            .unwrap_err();
        assert_eq!(error.code(), "audio_fetch_status");
        assert!(error.message().contains("403"));
        server.abort();
    }
}
