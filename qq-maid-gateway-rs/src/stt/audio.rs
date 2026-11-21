//! 入站语音的格式嗅探、大小限制与下载。
//!
//! 平台上报的 URL 与 MIME 都不可信：上传 STT 前只根据文件头判断容器，SILK 等
//! 无法被通用转写服务识别的格式直接失败，避免把原始 QQ 语音发给第三方。

use std::time::Duration;

use tokio::time::timeout;

use super::SttError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    Wav,
    Mp3,
    Ogg,
    Mp4,
    Flac,
    Amr,
    Silk,
}

impl AudioFormat {
    pub fn mime_type(self) -> &'static str {
        match self {
            Self::Wav => "audio/wav",
            Self::Mp3 => "audio/mpeg",
            Self::Ogg => "audio/ogg",
            Self::Mp4 => "audio/mp4",
            Self::Flac => "audio/flac",
            Self::Amr => "audio/amr",
            Self::Silk => "audio/silk",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Mp3 => "mp3",
            Self::Ogg => "ogg",
            Self::Mp4 => "m4a",
            Self::Flac => "flac",
            Self::Amr => "amr",
            Self::Silk => "silk",
        }
    }

    pub fn is_uploadable(self) -> bool {
        self != Self::Silk
    }
}

/// 待转写音频；文件名与 MIME 由嗅探结果决定，不采用平台上报值。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SttAudio {
    pub bytes: Vec<u8>,
    pub filename: String,
    pub mime_type: &'static str,
}

/// 根据文件头识别常见音频容器；无法识别时返回 `None`。
pub fn detect_audio_format(bytes: &[u8]) -> Option<AudioFormat> {
    // QQ SILK 可能带 0x02 长度前缀，也可能直接以 #!SILK_V3 开头。
    let silk = bytes.strip_prefix(&[0x02]).unwrap_or(bytes);
    if silk.starts_with(b"#!SILK_V3") || bytes.starts_with(b"#!SILK_V3") {
        return Some(AudioFormat::Silk);
    }
    if bytes.starts_with(b"#!AMR") {
        return Some(AudioFormat::Amr);
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        return Some(AudioFormat::Wav);
    }
    if bytes.starts_with(b"fLaC") {
        return Some(AudioFormat::Flac);
    }
    if bytes.starts_with(b"OggS") {
        return Some(AudioFormat::Ogg);
    }
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        return Some(AudioFormat::Mp4);
    }
    if bytes.starts_with(b"ID3") {
        return Some(AudioFormat::Mp3);
    }
    // MPEG audio frame sync：11 个连续 1 位。
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0 {
        return Some(AudioFormat::Mp3);
    }
    None
}

/// 校验大小并构造可上传音频；识别为 SILK 时明确失败。
pub fn audio_from_bytes(
    bytes: Vec<u8>,
    filename_hint: Option<&str>,
    max_bytes: u64,
) -> Result<SttAudio, SttError> {
    if bytes.is_empty() {
        return Err(SttError::UnsupportedFormat);
    }
    if bytes.len() as u64 > max_bytes {
        return Err(SttError::AudioTooLarge);
    }
    let format = match detect_audio_format(&bytes) {
        Some(format) => format,
        None if format_from_filename(filename_hint) == Some(AudioFormat::Silk) => {
            return Err(SttError::SilkNotSupported);
        }
        None => return Err(SttError::UnsupportedFormat),
    };
    if !format.is_uploadable() {
        return Err(SttError::SilkNotSupported);
    }
    Ok(SttAudio {
        filename: format!("voice.{}", format.extension()),
        mime_type: format.mime_type(),
        bytes,
    })
}

/// 下载远程音频；拒绝非 http(s)、超限响应与空 body。
pub(super) async fn download_audio(
    client: &reqwest::Client,
    url: &str,
    max_bytes: u64,
    request_timeout: Duration,
) -> Result<SttAudio, SttError> {
    let parsed = reqwest::Url::parse(url).map_err(|_| SttError::DownloadFailed)?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(SttError::DownloadFailed);
    }
    let download = async {
        let response = client
            .get(parsed)
            .send()
            .await
            .map_err(|_| SttError::DownloadFailed)?;
        if !response.status().is_success() {
            return Err(SttError::DownloadFailed);
        }
        if response
            .content_length()
            .is_some_and(|length| length > max_bytes)
        {
            return Err(SttError::AudioTooLarge);
        }
        let mut bytes = Vec::new();
        let mut body = response;
        while let Some(chunk) = body.chunk().await.map_err(|_| SttError::DownloadFailed)? {
            if bytes.len() as u64 + chunk.len() as u64 > max_bytes {
                return Err(SttError::AudioTooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        audio_from_bytes(bytes, None, max_bytes)
    };
    timeout(request_timeout, download)
        .await
        .map_err(|_| SttError::Timeout)?
}

fn format_from_filename(filename: Option<&str>) -> Option<AudioFormat> {
    let extension = filename?.rsplit('.').next()?.to_ascii_lowercase();
    match extension.as_str() {
        "wav" => Some(AudioFormat::Wav),
        "mp3" => Some(AudioFormat::Mp3),
        "ogg" | "opus" => Some(AudioFormat::Ogg),
        "m4a" | "mp4" | "aac" => Some(AudioFormat::Mp4),
        "flac" => Some(AudioFormat::Flac),
        "amr" => Some(AudioFormat::Amr),
        "silk" => Some(AudioFormat::Silk),
        _ => None,
    }
}
