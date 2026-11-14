use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::State,
    http::{HeaderMap, Response, StatusCode},
    response::IntoResponse,
    routing::post,
};
use futures_util::stream;
use serde_json::{Value, json};
use std::convert::Infallible;
use tokio::net::TcpListener;

use super::*;

#[derive(Clone)]
struct MockState {
    requests: Arc<Mutex<Vec<(HeaderMap, Value)>>>,
    response: Arc<Mutex<(StatusCode, Value)>>,
    delay: Arc<Mutex<Duration>>,
}

async fn qwen_handler(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    state.requests.lock().unwrap().push((headers, payload));
    let delay = *state.delay.lock().unwrap();
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }
    let (status, body) = state.response.lock().unwrap().clone();
    (status, Json(body))
}

async fn slow_qwen_body_handler() -> Response<Body> {
    let body = Body::from_stream(stream::once(async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        Ok::<Bytes, Infallible>(Bytes::from_static(
            br#"{"status_code":200,"output":{"audio":{"url":"https://audio.example.test/slow.wav"}}}"#,
        ))
    }));
    Response::builder()
        .header("content-type", "application/json")
        .body(body)
        .unwrap()
}

async fn pending_error_body_handler() -> Response<Body> {
    let body = Body::from_stream(stream::pending::<Result<Bytes, Infallible>>());
    Response::builder()
        .status(StatusCode::BAD_GATEWAY)
        .header("content-type", "application/json")
        .body(body)
        .unwrap()
}

async fn mock_provider() -> (QwenTtsProvider, MockState, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let state = MockState {
        requests: Arc::new(Mutex::new(Vec::new())),
        response: Arc::new(Mutex::new((
            StatusCode::OK,
            json!({
                "status_code": 200,
                "output": {"audio": {"url": "https://audio.example.test/result.wav?Signature=secret"}}
            }),
        ))),
        delay: Arc::new(Mutex::new(Duration::ZERO)),
    };
    let app = Router::new()
        .route("/tts", post(qwen_handler))
        .route("/tts/slow-body", post(slow_qwen_body_handler))
        .route("/tts/pending-error", post(pending_error_body_handler))
        .with_state(state.clone());
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let provider = QwenTtsProvider::new(
        qq_maid_common::http_client::client(),
        QwenTtsConfig {
            api_key: "test-key".to_owned(),
            base_url: format!("http://{address}/tts"),
            model: "qwen3-tts-flash".to_owned(),
            voice: "Cherry".to_owned(),
            request_timeout: Duration::from_secs(1),
            max_text_chars: 600,
        },
    );
    (provider, state, task)
}

#[tokio::test]
async fn qwen_non_streaming_request_returns_signed_wav_url_unchanged() {
    let (provider, state, task) = mock_provider().await;
    let audio = provider.synthesize("你好").await.unwrap();
    task.abort();

    assert_eq!(
        audio,
        TtsAudio::Url("https://audio.example.test/result.wav?Signature=secret".to_owned())
    );
    let requests = state.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0["authorization"], "Bearer test-key");
    assert_eq!(
        requests[0].1,
        json!({
            "model": "qwen3-tts-flash",
            "input": {"text": "你好", "voice": "Cherry"}
        })
    );
}

#[tokio::test]
async fn qwen_rejects_http_protocol_and_timeout_failures_without_leaking_signed_url() {
    let (mut provider, state, task) = mock_provider().await;
    *state.response.lock().unwrap() = (
        StatusCode::BAD_GATEWAY,
        json!({
            "message": "failed https://audio.example.test/result.wav?Signature=secret"
        }),
    );
    let error = provider.synthesize("你好").await.unwrap_err();
    assert!(matches!(error, TtsError::Status { status } if status == StatusCode::BAD_GATEWAY));
    assert!(!error.to_string().contains("Signature"));

    *state.response.lock().unwrap() = (StatusCode::OK, json!({"status_code": 200, "output": {}}));
    let error = provider.synthesize("你好").await.unwrap_err();
    assert!(matches!(error, TtsError::InvalidResponse));

    *state.delay.lock().unwrap() = Duration::from_millis(50);
    provider.config.request_timeout = Duration::from_millis(5);
    let error = provider.synthesize("你好").await.unwrap_err();
    task.abort();
    assert!(matches!(error, TtsError::Timeout { .. }));
}

#[tokio::test]
async fn qwen_rejects_empty_invalid_or_oversized_audio_url() {
    let (mut provider, state, task) = mock_provider().await;
    for url in ["", "file:///tmp/audio.wav", "javascript:alert(1)"] {
        *state.response.lock().unwrap() = (
            StatusCode::OK,
            json!({
                "status_code": 200,
                "output": {"audio": {"url": url}}
            }),
        );
        let error = provider.synthesize("你好").await.unwrap_err();
        assert!(matches!(
            error,
            TtsError::InvalidResponse | TtsError::InvalidAudioUrl
        ));
    }

    provider.config.max_text_chars = 2;
    let request_count = state.requests.lock().unwrap().len();
    let error = provider.synthesize("超过限制").await.unwrap_err();
    task.abort();
    assert!(matches!(error, TtsError::TextTooLong { max_chars: 2 }));
    assert_eq!(state.requests.lock().unwrap().len(), request_count);
}

#[tokio::test]
async fn qwen_timeout_covers_body_read_and_json_parsing_after_headers() {
    let (mut provider, _state, task) = mock_provider().await;
    provider.config.base_url.push_str("/slow-body");
    provider.config.request_timeout = Duration::from_millis(5);

    let error = provider.synthesize("你好").await.unwrap_err();
    task.abort();

    assert!(matches!(error, TtsError::Timeout { .. }));
}

#[tokio::test]
async fn qwen_http_error_does_not_wait_for_or_read_response_body() {
    let (mut provider, _state, task) = mock_provider().await;
    provider.config.base_url.push_str("/pending-error");

    let result = tokio::time::timeout(Duration::from_millis(100), provider.synthesize("你好"))
        .await
        .expect("HTTP error should return after headers without reading the pending body");
    task.abort();

    assert!(matches!(
        result,
        Err(TtsError::Status { status }) if status == StatusCode::BAD_GATEWAY
    ));
}

#[derive(Clone)]
struct OpenAiMockState {
    requests: Arc<Mutex<Vec<(HeaderMap, Value)>>>,
    response: Arc<Mutex<(StatusCode, Vec<u8>)>>,
    delay: Arc<Mutex<Duration>>,
}

async fn openai_handler(
    State(state): State<OpenAiMockState>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    state.requests.lock().unwrap().push((headers, payload));
    let delay = *state.delay.lock().unwrap();
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }
    let (status, body) = state.response.lock().unwrap().clone();
    (status, body)
}

async fn slow_openai_body_handler() -> Response<Body> {
    let body = Body::from_stream(stream::once(async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        Ok::<Bytes, Infallible>(Bytes::from_static(b"ID3slow"))
    }));
    Response::builder()
        .header("content-type", "audio/mpeg")
        .body(body)
        .unwrap()
}

async fn openai_mock_provider() -> (
    OpenAiTtsProvider,
    OpenAiMockState,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let state = OpenAiMockState {
        requests: Arc::new(Mutex::new(Vec::new())),
        response: Arc::new(Mutex::new((StatusCode::OK, b"ID3mock-mp3-audio".to_vec()))),
        delay: Arc::new(Mutex::new(Duration::ZERO)),
    };
    let app = Router::new()
        .route("/speech", post(openai_handler))
        .route("/speech/slow-body", post(slow_openai_body_handler))
        .with_state(state.clone());
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let provider = OpenAiTtsProvider::new(
        qq_maid_common::http_client::client(),
        OpenAiTtsConfig {
            api_key: "test-key".to_owned(),
            base_url: format!("http://{address}/speech"),
            model: "tts-1".to_owned(),
            voice: "alloy".to_owned(),
            request_timeout: Duration::from_secs(1),
            max_text_chars: 600,
        },
    );
    (provider, state, task)
}

#[tokio::test]
async fn openai_speech_request_returns_mp3_bytes_for_file_delivery() {
    let (provider, state, task) = openai_mock_provider().await;
    let audio = provider.synthesize("你好").await.unwrap();
    task.abort();

    assert_eq!(
        audio,
        TtsAudio::File {
            bytes: b"ID3mock-mp3-audio".to_vec(),
            file_name: "voice.mp3".to_owned(),
        }
    );
    let requests = state.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0["authorization"], "Bearer test-key");
    assert_eq!(
        requests[0].1,
        json!({
            "model": "tts-1",
            "input": "你好",
            "voice": "alloy",
            "response_format": "mp3"
        })
    );
}

#[tokio::test]
async fn openai_rejects_status_empty_and_oversized_audio() {
    let (mut provider, state, task) = openai_mock_provider().await;

    *state.response.lock().unwrap() = (StatusCode::UNAUTHORIZED, b"secret body".to_vec());
    let error = provider.synthesize("你好").await.unwrap_err();
    assert!(matches!(error, TtsError::Status { status } if status == StatusCode::UNAUTHORIZED));
    assert!(!error.to_string().contains("secret"));

    *state.response.lock().unwrap() = (StatusCode::OK, Vec::new());
    let error = provider.synthesize("你好").await.unwrap_err();
    assert!(matches!(error, TtsError::EmptyAudio));

    let oversized = 4 * 1024 * 1024 + 1;
    *state.response.lock().unwrap() = (StatusCode::OK, vec![0_u8; oversized]);
    let error = provider.synthesize("你好").await.unwrap_err();
    task.abort();
    assert!(matches!(error, TtsError::AudioTooLarge { .. }));

    provider.config.max_text_chars = 2;
    let error = provider.synthesize("超过限制").await.unwrap_err();
    assert!(matches!(error, TtsError::TextTooLong { max_chars: 2 }));
}

#[tokio::test]
async fn openai_timeout_covers_audio_body_read_after_headers() {
    let (mut provider, _state, task) = openai_mock_provider().await;
    provider.config.base_url.push_str("/slow-body");
    provider.config.request_timeout = Duration::from_millis(5);

    let error = provider.synthesize("你好").await.unwrap_err();
    task.abort();

    assert!(matches!(error, TtsError::Timeout { .. }));
}
