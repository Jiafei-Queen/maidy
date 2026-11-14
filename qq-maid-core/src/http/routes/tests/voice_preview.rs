//! TTS 试听管理接口的认证、二进制响应与错误映射。
use super::*;
use crate::{
    config::{VoiceFeatureConfig, center::ConfigCenterPaths, managed_config_fields},
    voice_preview::{
        VoicePreviewAudio, VoicePreviewError, VoicePreviewFormat, VoicePreviewFuture,
        VoicePreviewService,
    },
};
use std::collections::HashMap;

const PATH: &str = "/api/v1/console/configuration/voice/preview";

struct StubVoicePreview {
    failure: Option<VoicePreviewError>,
}

impl VoicePreviewService for StubVoicePreview {
    fn synthesize<'a>(
        &'a self,
        _config: &'a VoiceFeatureConfig,
        _text: &'a str,
    ) -> VoicePreviewFuture<'a> {
        let failure = self.failure.clone();
        Box::pin(async move {
            match failure {
                Some(error) => Err(error),
                None => Ok(VoicePreviewAudio {
                    bytes: b"ID3-test-audio".to_vec(),
                    format: VoicePreviewFormat::Mp3,
                }),
            }
        })
    }
}

async fn preview_request(
    state: OpsHttpState,
    body: serde_json::Value,
    cookie: Option<&str>,
    csrf: Option<&str>,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let app = build_router(state);
    let mut builder = axum::http::Request::builder()
        .method("POST")
        .uri(PATH)
        .header("content-type", "application/json")
        .header("host", "localhost");
    if let Some(cookie) = cookie {
        builder = builder.header(
            "cookie",
            format!("{}={cookie}", crate::management::SESSION_COOKIE_NAME),
        );
    }
    if let Some(csrf) = csrf {
        builder = builder.header("x-csrf-token", csrf);
    }
    let response = app
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, headers, body)
}

fn configured_state(
    failure: Option<VoicePreviewError>,
    with_api_key: bool,
) -> (OpsHttpState, String, String) {
    let (database, directory) =
        SqliteDatabase::open_temp_directory("qq-maid-voice-preview", APP_MIGRATIONS).unwrap();
    let (auth, cookie, csrf) = initialize_test_admin(database.clone(), &directory);
    let mut external = HashMap::from([
        ("TTS_PROVIDER".to_owned(), "openai".to_owned()),
        (
            "OPENAI_TTS_BASE_URL".to_owned(),
            "https://example.com/v1/audio/speech".to_owned(),
        ),
        ("OPENAI_TTS_MODEL".to_owned(), "test-tts".to_owned()),
        ("OPENAI_TTS_VOICE".to_owned(), "test-voice".to_owned()),
    ]);
    if with_api_key {
        external.insert("OPENAI_TTS_API_KEY".to_owned(), "test-key".to_owned());
    }
    let center = ConfigCenter::open(
        managed_config_fields(),
        ConfigCenterPaths {
            managed_config_file: directory.join("config/runtime.toml"),
            master_key_file: directory.join("config/secrets/master.key"),
        },
        database,
    )
    .unwrap()
    .with_external_environment(external);
    let mut state = test_state();
    state.config.web_console_enabled = true;
    state.admin_auth = Some(auth);
    state.config_center = Some(center);
    state.voice_preview = Some(Arc::new(StubVoicePreview { failure }));
    (state, cookie, csrf)
}

#[tokio::test]
async fn voice_preview_requires_session_and_csrf() -> Result<(), Infallible> {
    let (state, cookie, _csrf) = configured_state(None, true);
    let body = json!({"text": "试听"});

    let (status, _, _) = preview_request(state.clone(), body.clone(), None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _, _) = preview_request(state, body, Some(&cookie), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    Ok(())
}

#[tokio::test]
async fn voice_preview_returns_playable_audio_without_store_cache() -> Result<(), Infallible> {
    let (state, cookie, csrf) = configured_state(None, true);
    let (status, headers, body) =
        preview_request(state, json!({"text": "试听"}), Some(&cookie), Some(&csrf)).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"ID3-test-audio");
    assert_eq!(
        headers
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("audio/mpeg")
    );
    assert_eq!(
        headers
            .get(axum::http::header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("private, no-store")
    );
    assert!(
        headers
            .get(axum::http::header::CONTENT_DISPOSITION)
            .is_some()
    );
    Ok(())
}

#[tokio::test]
async fn voice_preview_maps_provider_failures_to_safe_json() -> Result<(), Infallible> {
    let (state, cookie, csrf) =
        configured_state(Some(VoicePreviewError::HttpStatus { status: 400 }), true);
    let (status, headers, body) =
        preview_request(state, json!({"text": "试听"}), Some(&cookie), Some(&csrf)).await;

    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        headers
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "http_status");
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("HTTP 400")
    );
    Ok(())
}

#[tokio::test]
async fn voice_preview_rejects_missing_saved_credentials_before_tts() -> Result<(), Infallible> {
    let (state, cookie, csrf) = configured_state(None, false);
    let (status, _, body) =
        preview_request(state, json!({"text": "试听"}), Some(&cookie), Some(&csrf)).await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["code"], "voice_unavailable");
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("OPENAI_TTS_API_KEY")
    );
    Ok(())
}

#[tokio::test]
async fn voice_preview_reports_unavailable_service_and_invalid_text() -> Result<(), Infallible> {
    let (mut state, cookie, csrf) = configured_state(None, true);
    state.voice_preview = None;
    let (status, _, _) =
        preview_request(state, json!({"text": "试听"}), Some(&cookie), Some(&csrf)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (state, cookie, csrf) = configured_state(Some(VoicePreviewError::EmptyText), true);
    let (status, _, body) =
        preview_request(state, json!({"text": "  "}), Some(&cookie), Some(&csrf)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["code"], "empty_text");
    Ok(())
}
