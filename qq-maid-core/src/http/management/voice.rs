//! 配置中心已保存 TTS 配置的试听接口。
//!
//! 试听只调用 Gateway 注入的 TTS 服务并返回音频字节，不经过 QQ 上传与发送；
//! 因此设置错误、鉴权失败或音色不可用时可以直接看到上游 HTTP 状态。

use axum::{
    body::Body,
    http::{HeaderValue, header},
    response::IntoResponse,
};
use serde::Deserialize;

use crate::{
    config::VoiceFeatureConfig,
    voice_preview::{VoicePreviewAudio, VoicePreviewError},
};

use super::*;

pub(super) fn router() -> Router<OpsHttpState> {
    Router::new().route(
        "/api/v1/console/configuration/voice/preview",
        post(voice_preview),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VoicePreviewRequest {
    text: String,
}

async fn voice_preview(
    State(state): State<OpsHttpState>,
    headers: HeaderMap,
    Json(payload): Json<VoicePreviewRequest>,
) -> Response {
    let (auth, _, _, actor) = match admin_context(&state, &headers, true) {
        Ok(value) => value,
        Err(response) => return respond(&state, &headers, *response),
    };
    let Some(service) = state.voice_preview.as_ref() else {
        let _ = auth.audit(Some(actor), "config.voice.preview", "unavailable");
        return respond(
            &state,
            &headers,
            api_error(
                StatusCode::NOT_FOUND,
                "voice_preview_unavailable",
                "语音试听不可用：当前进程没有装配 TTS 服务",
            ),
        );
    };
    let Some(center) = state.config_center.as_ref() else {
        let _ = auth.audit(Some(actor), "config.voice.preview", "unavailable");
        return respond(
            &state,
            &headers,
            api_error(
                StatusCode::NOT_FOUND,
                "configuration_unavailable",
                "配置中心不可用",
            ),
        );
    };
    // 与模型连接测试一致：读取已保存配置和已解密 secret，不等待重启。
    let environment = match center.current_resolved_environment() {
        Ok(value) => value,
        Err(error) => {
            return configuration_failure(&state, &headers, actor, "config.voice.preview", error);
        }
    };
    let config = VoiceFeatureConfig::from_environment(&environment);
    if !config.is_available() {
        let _ = auth.audit(Some(actor), "config.voice.preview", "voice_unavailable");
        return respond(
            &state,
            &headers,
            api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "voice_unavailable",
                config
                    .enable_rejection_text()
                    .unwrap_or("语音功能当前不可用，请检查 TTS 配置"),
            ),
        );
    }
    match service.synthesize(&config, &payload.text).await {
        Ok(audio) => {
            if let Err(error) = auth.audit(Some(actor), "config.voice.preview", "success") {
                return respond(
                    &state,
                    &headers,
                    api_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        error.code(),
                        error.message(),
                    ),
                );
            }
            audio_response(&state, &headers, audio)
        }
        Err(error) => {
            let _ = auth.audit(Some(actor), "config.voice.preview", error.code());
            let status = preview_error_status(&error);
            respond(
                &state,
                &headers,
                api_error(status, error.code(), error.message()),
            )
        }
    }
}

fn preview_error_status(error: &VoicePreviewError) -> StatusCode {
    match error {
        VoicePreviewError::EmptyText | VoicePreviewError::TextTooLong { .. } => {
            StatusCode::UNPROCESSABLE_ENTITY
        }
        VoicePreviewError::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::BAD_GATEWAY,
    }
}

fn audio_response(state: &OpsHttpState, headers: &HeaderMap, audio: VoicePreviewAudio) -> Response {
    let content_type = HeaderValue::from_static(audio.format.content_type());
    let disposition = HeaderValue::from_str(&format!(
        "inline; filename=\"{}\"",
        audio.format.file_name()
    ))
    .expect("preview file name must be a valid header value");
    let content_length = HeaderValue::from_str(&audio.bytes.len().to_string())
        .expect("audio byte length must be a valid header value");
    let mut response = Body::from(audio.bytes).into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, content_type);
    response
        .headers_mut()
        .insert(header::CONTENT_LENGTH, content_length);
    response
        .headers_mut()
        .insert(header::CONTENT_DISPOSITION, disposition);
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    crate::http::console_routes::with_console_cors(response, state, headers)
}
