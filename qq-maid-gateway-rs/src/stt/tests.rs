use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use qq_maid_core::config::{
    ChatScene, SttCandidate, SttRouteSet, SttRuntimeConfig, SttRuntimeStatus,
};

use super::{
    AudioFormat, SttAudio, SttError, SttFuture, SttService, SttTransport, detect_audio_format,
};

fn candidate(provider: &str, model: &str, api_key: Option<&str>) -> SttCandidate {
    SttCandidate {
        provider: provider.to_owned(),
        model: model.to_owned(),
        base_url: "https://stt.invalid/v1".to_owned(),
        api_key: api_key.map(str::to_owned),
        auth_header: "Authorization".to_owned(),
        auth_scheme: Some("Bearer".to_owned()),
        request_timeout_seconds: None,
    }
}

fn config(default: Vec<SttCandidate>) -> SttRuntimeConfig {
    SttRuntimeConfig {
        enabled: true,
        status: SttRuntimeStatus::Available,
        platform_asr: qq_maid_core::config::SttPlatformAsrMode::Auto,
        language: Some("zh".to_owned()),
        timeout: Duration::from_secs(5),
        platform_asr_attempts: 2,
        max_audio_bytes: 1024 * 1024,
        max_text_chars: 2000,
        routes: SttRouteSet {
            default,
            private: Vec::new(),
            group: Vec::new(),
        },
    }
}

struct ScriptedTransport {
    results: Mutex<Vec<Result<String, SttError>>>,
    calls: Mutex<Vec<String>>,
}

impl ScriptedTransport {
    fn new(results: Vec<Result<String, SttError>>) -> Arc<Self> {
        Arc::new(Self {
            results: Mutex::new(results),
            calls: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl SttTransport for ScriptedTransport {
    fn transcribe<'a>(
        &'a self,
        candidate: &'a SttCandidate,
        _audio: &'a SttAudio,
        _language: Option<&'a str>,
        _timeout: Duration,
    ) -> SttFuture<'a> {
        Box::pin(async move {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{}:{}", candidate.provider, candidate.model));
            {
                let mut results = self.results.lock().unwrap();
                if results.is_empty() {
                    Err(SttError::NoCandidate)
                } else {
                    results.remove(0)
                }
            }
        })
    }
}

fn wav_bytes() -> Vec<u8> {
    let mut bytes = b"RIFF".to_vec();
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(&[0; 32]);
    bytes
}

#[tokio::test]
async fn chain_falls_back_to_next_candidate_on_failure() {
    let transport = ScriptedTransport::new(vec![
        Err(SttError::Status { status: 500 }),
        Ok("你好 世界\n".to_owned()),
    ]);
    let config = config(vec![
        candidate("openai", "whisper-1", Some("key-1")),
        candidate("bigmodel", "glm-asr-2512", Some("key-2")),
    ]);
    let service = SttService::with_transport(config, transport.clone());

    let text = service
        .transcribe_bytes(ChatScene::Private, wav_bytes(), Some("voice.wav"))
        .await
        .unwrap();

    assert_eq!(text, "你好 世界");
    assert_eq!(
        transport.calls(),
        ["openai:whisper-1", "bigmodel:glm-asr-2512"]
    );
}

#[tokio::test]
async fn all_candidates_fail_returns_last_error_without_fake_text() {
    let transport = ScriptedTransport::new(vec![
        Err(SttError::Timeout),
        Err(SttError::Status { status: 429 }),
    ]);
    let config = config(vec![
        candidate("openai", "whisper-1", Some("key-1")),
        candidate("bigmodel", "glm-asr-2512", Some("key-2")),
    ]);
    let service = SttService::with_transport(config, transport);

    let error = service
        .transcribe_bytes(ChatScene::Group, wav_bytes(), None)
        .await
        .unwrap_err();

    assert_eq!(error, SttError::Status { status: 429 });
}

#[tokio::test]
async fn candidate_without_credentials_is_skipped() {
    let transport = ScriptedTransport::new(vec![Ok("可用文本".to_owned())]);
    let config = config(vec![
        candidate("openai", "whisper-1", None),
        candidate("bigmodel", "glm-asr-2512", Some("key")),
    ]);
    let service = SttService::with_transport(config, transport.clone());

    let text = service
        .transcribe_bytes(ChatScene::Private, wav_bytes(), None)
        .await
        .unwrap();

    assert_eq!(text, "可用文本");
    assert_eq!(transport.calls(), ["bigmodel:glm-asr-2512"]);
}

#[tokio::test]
async fn transcript_truncates_on_character_boundary() {
    let transport = ScriptedTransport::new(vec![Ok("一二三四五六".to_owned())]);
    let mut config = config(vec![candidate("openai", "whisper-1", Some("key"))]);
    config.max_text_chars = 3;
    let service = SttService::with_transport(config, transport);

    let text = service
        .transcribe_bytes(ChatScene::Private, wav_bytes(), None)
        .await
        .unwrap();

    assert_eq!(text, "一二三");
}

#[tokio::test]
async fn silk_audio_is_rejected_before_upload() {
    let transport = ScriptedTransport::new(vec![Ok("不应调用".to_owned())]);
    let config = config(vec![candidate("openai", "whisper-1", Some("key"))]);
    let service = SttService::with_transport(config, transport.clone());

    let error = service
        .transcribe_bytes(
            ChatScene::Private,
            b"\x02#!SILK_V3raw".to_vec(),
            Some("voice.silk"),
        )
        .await
        .unwrap_err();

    assert_eq!(error, SttError::SilkNotSupported);
    assert!(transport.calls().is_empty());
}

#[tokio::test]
async fn oversized_audio_is_rejected_before_upload() {
    let transport = ScriptedTransport::new(vec![Ok("不应调用".to_owned())]);
    let mut config = config(vec![candidate("openai", "whisper-1", Some("key"))]);
    config.max_audio_bytes = 16;
    let service = SttService::with_transport(config, transport.clone());

    let error = service
        .transcribe_bytes(ChatScene::Private, wav_bytes(), None)
        .await
        .unwrap_err();

    assert_eq!(error, SttError::AudioTooLarge);
    assert!(transport.calls().is_empty());
}

#[tokio::test]
async fn openai_transport_sends_multipart_with_model_language_and_auth() {
    use axum::{Json, Router, body::Bytes, extract::State, http::HeaderMap, routing::post};
    use serde_json::{Value, json};

    #[derive(Clone)]
    struct Capture(Arc<Mutex<(Option<String>, String)>>);

    async fn handler(
        State(capture): State<Capture>,
        headers: HeaderMap,
        body: Bytes,
    ) -> Json<Value> {
        let authorization = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        *capture.0.lock().unwrap() = (authorization, String::from_utf8_lossy(&body).into_owned());
        Json(json!({"text": "识别成功"}))
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let captured = Arc::new(Mutex::new((None, String::new())));
    let app = Router::new()
        .route("/audio/transcriptions", post(handler))
        .with_state(Capture(captured.clone()));
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let audio = super::audio::audio_from_bytes(wav_bytes(), None, 1024 * 1024).unwrap();
    let candidate = SttCandidate {
        provider: "openai".to_owned(),
        model: "whisper-1".to_owned(),
        base_url: format!("http://{addr}"),
        api_key: Some("test-key".to_owned()),
        auth_header: "Authorization".to_owned(),
        auth_scheme: Some("Bearer".to_owned()),
        request_timeout_seconds: None,
    };
    let client = super::OpenAiSttClient::new(qq_maid_common::http_client::client());
    let text = client
        .transcribe(&candidate, &audio, Some("zh"), Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(text, "识别成功");

    let (authorization, body) = captured.lock().unwrap().clone();
    assert_eq!(authorization.as_deref(), Some("Bearer test-key"));
    assert!(body.contains("name=\"model\""));
    assert!(body.contains("whisper-1"));
    assert!(body.contains("name=\"language\""));
    assert!(body.contains("zh"));
    assert!(body.contains("name=\"file\"; filename=\"voice.wav\""));
    assert!(body.contains("Content-Type: audio/wav"));
    assert!(!body.contains("test-key"));

    server.abort();
}

#[test]
fn detects_supported_audio_headers() {
    assert_eq!(detect_audio_format(&wav_bytes()), Some(AudioFormat::Wav));
    assert_eq!(detect_audio_format(b"ID3\x04"), Some(AudioFormat::Mp3));
    assert_eq!(detect_audio_format(b"OggS\0\0"), Some(AudioFormat::Ogg));
    assert_eq!(detect_audio_format(b"fLaC\0"), Some(AudioFormat::Flac));
    assert_eq!(
        detect_audio_format(b"\0\0\0\x20ftypM4A "),
        Some(AudioFormat::Mp4)
    );
    assert_eq!(detect_audio_format(b"#!AMR\n"), Some(AudioFormat::Amr));
    assert_eq!(detect_audio_format(b"#!SILK_V3"), Some(AudioFormat::Silk));
    assert_eq!(detect_audio_format(b"unknown"), None);
}
