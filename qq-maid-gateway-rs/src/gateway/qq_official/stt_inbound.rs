//! QQ 官方语音的转录编排。
//!
//! 平台 `asr_refer_text` 已由事件解析阶段注入；本模块按 `[stt]` 候选链决定是否采用
//! 平台文本、是否下载 `voice_wav_url` 调用远程 STT。转写只注入 input parts，不写入正文。

use qq_maid_common::input_part::{MessageInputPart, TextSource};
use qq_maid_core::config::{ChatScene, SttRuntimeConfig, SttStep};
use tracing::debug;

use crate::{
    gateway::event::Attachment,
    stt::{SttAudio, SttError, SttService},
};

const TRANSCRIPT_PREFIX: &str = "[语音转文字]";
const FAILED_PLACEHOLDER: &str = "[语音消息：未识别出文字]";

/// 按候选链补齐当前消息的语音转写。
///
/// 平台文本会先从 input parts 取出，只有链路执行到 `platform:qq` 时才重新注入；
/// 因此把平台候选放在远程候选之后就真的先调用远程 STT，不会同时留下两份转写。
pub(super) async fn apply_voice_transcript(
    stt: Option<&SttService>,
    config: &SttRuntimeConfig,
    scene: ChatScene,
    input_parts: &mut Vec<MessageInputPart>,
    attachments: &[Attachment],
) {
    let platform_text = take_platform_transcript(input_parts);
    let steps = config.steps(scene);
    if steps.is_empty() {
        return;
    }
    let attachment = attachments.iter().find(|attachment| attachment.is_audio());
    if attachment.is_none() && platform_text.is_none() {
        return;
    }

    let mut remote_audio: Option<SttAudio> = None;
    let mut last_error = SttError::NoCandidate;
    for (chain_index, step) in steps.iter().enumerate() {
        match step {
            SttStep::Platform => {
                let Some(text) = platform_text
                    .as_deref()
                    .map(str::trim)
                    .filter(|text| usable_platform_text(text))
                else {
                    continue;
                };
                input_parts.push(transcript_part(text));
                return;
            }
            SttStep::Remote(candidate) => {
                let Some(stt) = stt else {
                    last_error = SttError::NotConfigured;
                    continue;
                };
                let Some(attachment) = attachment else {
                    continue;
                };
                if remote_audio.is_none() {
                    let Some(url) = audio_url(attachment) else {
                        continue;
                    };
                    match stt.fetch_audio(url).await {
                        Ok(audio) => remote_audio = Some(audio),
                        Err(error) => {
                            debug!(
                                chain_index,
                                error_class = error.kind(),
                                "QQ 官方语音音频下载失败"
                            );
                            last_error = error;
                            continue;
                        }
                    }
                }
                let Some(audio) = remote_audio.as_ref() else {
                    last_error = SttError::DownloadFailed;
                    continue;
                };
                match stt
                    .transcribe_candidate(scene, chain_index, candidate, audio)
                    .await
                {
                    Ok(text) => {
                        input_parts.push(transcript_part(&text));
                        return;
                    }
                    Err(error) => {
                        debug!(
                            chain_index,
                            provider = %candidate.provider,
                            model = %candidate.model,
                            error_class = error.kind(),
                            "QQ 官方语音 STT 候选失败，尝试下一候选"
                        );
                        last_error = error;
                    }
                }
            }
        }
    }
    debug!(error_class = last_error.kind(), "QQ 官方语音候选链全部失败");
    input_parts.push(failure_placeholder());
}

/// 取出事件解析阶段注入的平台转写，并从 input parts 移除；后续按链路位置决定是否放回。
fn take_platform_transcript(input_parts: &mut Vec<MessageInputPart>) -> Option<String> {
    let mut found = None;
    input_parts.retain(|part| {
        let MessageInputPart::Text {
            text,
            source: Some(TextSource::Transcript),
        } = part
        else {
            return true;
        };
        if found.is_none() {
            let content = text
                .strip_prefix(TRANSCRIPT_PREFIX)
                .unwrap_or(text)
                .trim()
                .to_owned();
            if !content.is_empty() {
                found = Some(content);
            }
        }
        false
    });
    found
}

fn audio_url(attachment: &Attachment) -> Option<&str> {
    attachment
        .voice_wav_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            attachment
                .url
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
}

fn usable_platform_text(text: &str) -> bool {
    text.chars().any(char::is_alphanumeric)
}

fn transcript_part(text: &str) -> MessageInputPart {
    MessageInputPart::Text {
        text: format!("{TRANSCRIPT_PREFIX} {text}"),
        source: Some(TextSource::Transcript),
    }
}

fn failure_placeholder() -> MessageInputPart {
    MessageInputPart::Text {
        text: FAILED_PLACEHOLDER.to_owned(),
        source: Some(TextSource::Context),
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use qq_maid_common::input_part::{MediaStatus, MessageMedia};
    use qq_maid_core::config::{
        SttCandidate, SttPlatformAsrMode, SttRouteSet, SttRuntimeStatus, SttStep,
    };

    use super::*;
    use crate::stt::{SttFuture, SttTransport};

    fn audio_attachment() -> Attachment {
        Attachment {
            content_type: Some("voice".to_owned()),
            filename: Some("voice.silk".to_owned()),
            url: Some("https://example.test/voice.silk".to_owned()),
            size_bytes: None,
            media_id: None,
            file_id: None,
            attachment_id: None,
            asr_refer_text: None,
            voice_wav_url: Some("https://example.test/voice.wav".to_owned()),
        }
    }

    fn file_part() -> MessageInputPart {
        MessageInputPart::file(MessageMedia {
            mime_type: Some("audio/silk".to_owned()),
            status: MediaStatus::Available,
            ..Default::default()
        })
    }

    fn transcript_part() -> MessageInputPart {
        MessageInputPart::Text {
            text: "[语音转文字] 平台结果".to_owned(),
            source: Some(TextSource::Transcript),
        }
    }

    fn remote_candidate() -> SttCandidate {
        SttCandidate {
            provider: "openai".to_owned(),
            model: "whisper-1".to_owned(),
            base_url: "https://stt.invalid/v1".to_owned(),
            api_key: Some("key".to_owned()),
            auth_header: "Authorization".to_owned(),
            auth_scheme: Some("Bearer".to_owned()),
            request_timeout_seconds: None,
        }
    }

    fn wav_bytes() -> Vec<u8> {
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(&[0; 32]);
        bytes
    }

    fn config(
        default: Vec<SttStep>,
        enabled: bool,
        platform_asr: SttPlatformAsrMode,
    ) -> SttRuntimeConfig {
        SttRuntimeConfig {
            enabled,
            status: SttRuntimeStatus::Available,
            platform_asr,
            language: Some("zh".to_owned()),
            timeout: Duration::from_secs(5),
            platform_asr_attempts: 1,
            max_audio_bytes: 1024 * 1024,
            max_text_chars: 2000,
            routes: SttRouteSet {
                default,
                private: Vec::new(),
                group: Vec::new(),
            },
        }
    }

    struct ScriptedTransport;

    impl SttTransport for ScriptedTransport {
        fn transcribe<'a>(
            &'a self,
            _candidate: &'a SttCandidate,
            _audio: &'a SttAudio,
            _language: Option<&'a str>,
            _timeout: Duration,
        ) -> SttFuture<'a> {
            Box::pin(async move { Ok("远程结果".to_owned()) })
        }
    }

    #[tokio::test]
    async fn platform_step_keeps_existing_result() {
        let mut parts = vec![file_part(), transcript_part()];
        apply_voice_transcript(
            None,
            &config(vec![SttStep::Platform], false, SttPlatformAsrMode::Auto),
            ChatScene::Private,
            &mut parts,
            &[audio_attachment()],
        )
        .await;
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1].text_content(), Some("[语音转文字] 平台结果"));
    }

    #[tokio::test]
    async fn platform_off_without_remote_candidates_drops_transcript() {
        let mut parts = vec![file_part(), transcript_part()];
        apply_voice_transcript(
            None,
            &config(Vec::new(), false, SttPlatformAsrMode::Off),
            ChatScene::Group,
            &mut parts,
            &[audio_attachment()],
        )
        .await;
        assert_eq!(parts.len(), 1);
        assert_eq!(
            parts[0].media_kind(),
            Some(qq_maid_common::input_part::MediaKind::File)
        );
    }

    #[tokio::test]
    async fn punctuation_only_platform_transcript_falls_back() {
        let mut parts = vec![
            file_part(),
            MessageInputPart::Text {
                text: "[语音转文字] 。。。？！".to_owned(),
                source: Some(TextSource::Transcript),
            },
        ];
        apply_voice_transcript(
            None,
            &config(vec![SttStep::Platform], false, SttPlatformAsrMode::Auto),
            ChatScene::Private,
            &mut parts,
            &[audio_attachment()],
        )
        .await;
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1].text_content(), Some(FAILED_PLACEHOLDER));
    }

    #[tokio::test]
    async fn missing_platform_text_without_stt_adds_placeholder() {
        let mut parts = vec![file_part()];
        apply_voice_transcript(
            None,
            &config(Vec::new(), false, SttPlatformAsrMode::Auto),
            ChatScene::Private,
            &mut parts,
            &[audio_attachment()],
        )
        .await;
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1].text_content(), Some(FAILED_PLACEHOLDER));
    }

    #[tokio::test]
    async fn remote_candidate_before_platform_uses_remote_result() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app =
            axum::Router::new().route("/voice.wav", axum::routing::get(|| async { wav_bytes() }));
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let mut parts = vec![file_part(), transcript_part()];
        let mut attachment = audio_attachment();
        attachment.voice_wav_url = Some(format!("http://{addr}/voice.wav"));
        let stt = SttService::with_transport(
            config(
                vec![SttStep::Remote(remote_candidate()), SttStep::Platform],
                true,
                SttPlatformAsrMode::Auto,
            ),
            Arc::new(ScriptedTransport),
        );
        apply_voice_transcript(
            Some(&stt),
            stt.config(),
            ChatScene::Private,
            &mut parts,
            &[attachment],
        )
        .await;
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1].text_content(), Some("[语音转文字] 远程结果"));
        server.abort();
    }

    #[tokio::test]
    async fn non_audio_message_is_untouched() {
        let mut parts = vec![MessageInputPart::file(MessageMedia::default())];
        let attachment = Attachment {
            content_type: Some("application/pdf".to_owned()),
            filename: Some("doc.pdf".to_owned()),
            ..audio_attachment()
        };
        apply_voice_transcript(
            None,
            &config(Vec::new(), false, SttPlatformAsrMode::Auto),
            ChatScene::Private,
            &mut parts,
            &[attachment],
        )
        .await;
        assert_eq!(parts.len(), 1);
    }
}
