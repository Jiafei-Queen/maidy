//! OpenAI 兼容 `/audio/transcriptions` 客户端。
//!
//! 只发送 `file`、`model`、`language` 与 `response_format=json`，不附带平台元数据；
//! Provider 差异通过候选链降级处理，不在协议层猜测能力。

use std::time::Duration;

use qq_maid_core::config::SttCandidate;
use reqwest::{
    header::{HeaderName, HeaderValue},
    multipart::{Form, Part},
};
use serde_json::Value;
use tokio::time::timeout;

use super::{SttAudio, SttError, SttFuture, SttTransport};

const MAX_RESPONSE_BYTES: usize = 64 * 1024;

#[derive(Clone)]
pub struct OpenAiSttClient {
    client: reqwest::Client,
}

impl OpenAiSttClient {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    async fn request(
        &self,
        candidate: &SttCandidate,
        audio: &SttAudio,
        language: Option<&str>,
        request_timeout: Duration,
    ) -> Result<String, SttError> {
        let Some(api_key) = candidate
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
        else {
            return Err(SttError::NotConfigured);
        };
        let mut form = Form::new()
            .text("model", candidate.model.clone())
            .text("response_format", "json")
            .part(
                "file",
                Part::bytes(audio.bytes.clone())
                    .file_name(audio.filename.clone())
                    .mime_str(audio.mime_type)
                    .map_err(|_| SttError::InvalidResponse)?,
            );
        // 空 language 不发送，避免部分兼容服务把空串当作非法枚举。
        if let Some(language) = language.map(str::trim).filter(|value| !value.is_empty()) {
            form = form.text("language", language.to_owned());
        }
        let mut request = self.client.post(candidate.endpoint()).multipart(form);
        let header = HeaderName::from_bytes(candidate.auth_header.as_bytes())
            .unwrap_or(reqwest::header::AUTHORIZATION);
        let value = match candidate.auth_scheme.as_deref().map(str::trim) {
            Some(scheme) if !scheme.is_empty() => format!("{scheme} {api_key}"),
            _ => api_key.to_owned(),
        };
        let mut value = HeaderValue::from_str(&value).map_err(|_| SttError::NotConfigured)?;
        value.set_sensitive(true);
        request = request.header(header, value);

        let exchange = async {
            let response = request.send().await.map_err(|_| SttError::RequestFailed)?;
            if !response.status().is_success() {
                return Err(SttError::Status {
                    status: response.status().as_u16(),
                });
            }
            let mut body = Vec::new();
            let mut response = response;
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| SttError::RequestFailed)?
            {
                if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
                    return Err(SttError::InvalidResponse);
                }
                body.extend_from_slice(&chunk);
            }
            parse_transcript(&body)
        };
        timeout(request_timeout, exchange)
            .await
            .map_err(|_| SttError::Timeout)?
    }
}

impl SttTransport for OpenAiSttClient {
    fn transcribe<'a>(
        &'a self,
        candidate: &'a SttCandidate,
        audio: &'a SttAudio,
        language: Option<&'a str>,
        request_timeout: Duration,
    ) -> SttFuture<'a> {
        Box::pin(async move {
            self.request(candidate, audio, language, request_timeout)
                .await
        })
    }
}

fn parse_transcript(body: &[u8]) -> Result<String, SttError> {
    let value: Value = serde_json::from_slice(body).map_err(|_| SttError::InvalidResponse)?;
    let text = value
        .get("text")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get("choices")
                .and_then(Value::as_array)
                .and_then(|choices| choices.first())
                .and_then(|choice| choice.get("message"))
                .and_then(|message| message.get("content"))
                .and_then(Value::as_str)
        })
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or(SttError::EmptyTranscript)?;
    Ok(text.to_owned())
}
