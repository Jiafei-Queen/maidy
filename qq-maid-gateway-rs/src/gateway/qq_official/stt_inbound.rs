//! QQ 官方语音的 STT 回退。
//!
//! 平台 `asr_refer_text` 仍是第一优先级；只有没有可用平台文本时，才下载
//! `voice_wav_url`（或原始 URL）并调用 STT 链。转写只注入 input parts，不写入正文。

use qq_maid_common::input_part::{MessageInputPart, TextSource};
use qq_maid_core::config::{ChatScene, SttPlatformAsrMode};
use tracing::debug;

use crate::{gateway::event::Attachment, stt::SttService};

const TRANSCRIPT_PREFIX: &str = "[语音转文字]";
const FAILED_PLACEHOLDER: &str = "[语音消息：未识别出文字]";

/// 按平台 ASR 策略补齐当前消息的语音转写。
///
/// - `platform_asr=off`：先移除事件解析阶段注入的平台转写，再走 STT 链。
/// - `platform_asr=auto`：平台已有转写时不重复上传音频。
pub(super) async fn apply_voice_transcript(
    stt: Option<&SttService>,
    stt_enabled: bool,
    platform_asr: SttPlatformAsrMode,
    scene: ChatScene,
    input_parts: &mut Vec<MessageInputPart>,
    attachments: &[Attachment],
) {
    let platform_transcript_usable = usable_platform_transcript(input_parts);
    if platform_asr == SttPlatformAsrMode::Auto && platform_transcript_usable {
        return;
    }
    if platform_asr == SttPlatformAsrMode::Off || !platform_transcript_usable {
        // 平台纯标点或空转写不算成功；清理后继续走 STT 链。
        input_parts.retain(|part| {
            !matches!(
                part,
                MessageInputPart::Text {
                    source: Some(TextSource::Transcript),
                    ..
                }
            )
        });
    }
    let Some(attachment) = attachments.iter().find(|attachment| attachment.is_audio()) else {
        return;
    };
    // 平台与 STT 都主动关闭时保持旧行为，不额外插入占位提示。
    if !stt_enabled && platform_asr == SttPlatformAsrMode::Off {
        return;
    }
    let stt = match stt {
        Some(stt) => stt,
        None => {
            input_parts.push(failure_placeholder());
            return;
        }
    };
    let url = attachment
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
        });
    let Some(url) = url else {
        input_parts.push(failure_placeholder());
        return;
    };
    match stt.transcribe_url(scene, url).await {
        Ok(text) => input_parts.push(MessageInputPart::Text {
            text: format!("{TRANSCRIPT_PREFIX} {text}"),
            source: Some(TextSource::Transcript),
        }),
        Err(error) => {
            debug!(error_class = error.kind(), "QQ 官方语音 STT 回退失败");
            input_parts.push(failure_placeholder());
        }
    }
}

fn usable_platform_transcript(input_parts: &[MessageInputPart]) -> bool {
    input_parts.iter().any(|part| {
        let MessageInputPart::Text {
            text,
            source: Some(TextSource::Transcript),
        } = part
        else {
            return false;
        };
        let content = text.strip_prefix(TRANSCRIPT_PREFIX).unwrap_or(text).trim();
        content.chars().any(char::is_alphanumeric)
    })
}

fn failure_placeholder() -> MessageInputPart {
    MessageInputPart::Text {
        text: FAILED_PLACEHOLDER.to_owned(),
        source: Some(TextSource::Context),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qq_maid_common::input_part::{MediaStatus, MessageMedia};

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
            false,
            SttPlatformAsrMode::Auto,
            ChatScene::Private,
            &mut parts,
            &[audio_attachment()],
        )
        .await;
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1].text_content(), Some(FAILED_PLACEHOLDER));
    }

    #[tokio::test]
    async fn platform_auto_with_transcript_keeps_existing_result() {
        let mut parts = vec![file_part(), transcript_part()];
        apply_voice_transcript(
            None,
            false,
            SttPlatformAsrMode::Auto,
            ChatScene::Private,
            &mut parts,
            &[audio_attachment()],
        )
        .await;
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1].text_content(), Some("[语音转文字] 平台结果"));
    }

    #[tokio::test]
    async fn platform_off_drops_transcript_and_falls_back_to_placeholder_without_stt() {
        let mut parts = vec![file_part(), transcript_part()];
        apply_voice_transcript(
            None,
            true,
            SttPlatformAsrMode::Off,
            ChatScene::Group,
            &mut parts,
            &[audio_attachment()],
        )
        .await;
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1].text_content(), Some(FAILED_PLACEHOLDER));
        assert!(matches!(
            parts[1],
            MessageInputPart::Text {
                source: Some(TextSource::Context),
                ..
            }
        ));
    }

    #[tokio::test]
    async fn missing_platform_text_without_stt_adds_placeholder() {
        let mut parts = vec![file_part()];
        apply_voice_transcript(
            None,
            false,
            SttPlatformAsrMode::Auto,
            ChatScene::Private,
            &mut parts,
            &[audio_attachment()],
        )
        .await;
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1].text_content(), Some(FAILED_PLACEHOLDER));
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
            false,
            SttPlatformAsrMode::Auto,
            ChatScene::Private,
            &mut parts,
            &[attachment],
        )
        .await;
        assert_eq!(parts.len(), 1);
    }
}
