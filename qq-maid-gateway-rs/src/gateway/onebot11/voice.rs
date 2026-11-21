//! OneBot 11 语音转写编排：按 `[stt]` 候选链执行平台与远程候选。
//!
//! 只处理 `record` 段映射出的音频文件；转写文本以 `Transcript` 注入 input parts，
//! 不写入 `inbound.text`，因此语音不会触发 `/todo` 等命令。日志不记录转写文本、
//! 音频 URL 或 base64。

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use qq_maid_common::input_part::{MediaStatus, MessageInputPart, MessageMedia, TextSource};
use qq_maid_core::config::{ChatScene, SttRuntimeConfig, SttStep};
use serde_json::json;
use tokio::time::sleep;
use tracing::{debug, info};

use crate::{
    gateway::{
        onebot11::connection::{OneBotCallError, OneBotConnectionContext},
        platform::{ConversationTarget, InboundMessage, onebot11::RECORD_MEDIA_ID},
    },
    stt::{SttAudio, SttError, SttService},
};

const PLATFORM_ASR_RETRY_DELAY: Duration = Duration::from_millis(400);
/// 语音转码可能触发 NapCat 本机 ffmpeg；常规 action 预算过短时至少给到 30 秒。
const VOICE_ACTION_MIN_TIMEOUT: Duration = Duration::from_secs(30);
const TRANSCRIPT_PREFIX: &str = "[语音转文字]";
const FAILED_PLACEHOLDER: &str = "[语音消息：未识别出文字]";

#[derive(Debug, Default)]
struct CapabilityCache {
    generation: Option<u64>,
    platform_asr_unsupported: bool,
    get_record_unsupported: bool,
}

/// 单连接语音转写器；能力探测结果按连接 generation 缓存。
#[derive(Clone)]
pub(super) struct OneBotVoiceTranscriber {
    connection: OneBotConnectionContext,
    stt: Option<Arc<SttService>>,
    config: Arc<SttRuntimeConfig>,
    capabilities: Arc<Mutex<CapabilityCache>>,
}

impl OneBotVoiceTranscriber {
    pub(super) fn new(
        connection: OneBotConnectionContext,
        stt: Option<Arc<SttService>>,
        config: Arc<SttRuntimeConfig>,
    ) -> Self {
        Self {
            connection,
            stt,
            config,
            capabilities: Arc::new(Mutex::new(CapabilityCache::default())),
        }
    }

    pub(super) async fn apply(&self, inbound: &mut InboundMessage) {
        let Some((index, media)) = first_voice_media(inbound) else {
            return;
        };
        let scene = match inbound.conversation {
            ConversationTarget::Private { .. } => ChatScene::Private,
            ConversationTarget::Group { .. } => ChatScene::Group,
            _ => return,
        };
        let steps = self.config.steps(scene);
        if steps.is_empty() {
            return;
        }
        if media.status == MediaStatus::SizeExceeded {
            inbound
                .input_parts
                .insert(index + 1, failure_placeholder_part());
            return;
        }

        let mut remote_audio: Option<SttAudio> = None;
        let mut last_error = SttError::NoCandidate;
        for (chain_index, step) in steps.iter().enumerate() {
            match step {
                SttStep::Platform => {
                    if let Some(text) = self.platform_transcript(&inbound.message_id).await {
                        inbound
                            .input_parts
                            .insert(index + 1, transcript_part(&text));
                        return;
                    }
                }
                SttStep::Remote(candidate) => {
                    let Some(stt) = self.stt.as_ref() else {
                        last_error = SttError::NotConfigured;
                        continue;
                    };
                    if remote_audio.is_none() {
                        match self.load_audio(stt, &media).await {
                            Ok(audio) => remote_audio = Some(audio),
                            Err(error) => {
                                debug!(
                                    chain_index,
                                    error_class = error.kind(),
                                    "OneBot 11 语音音频准备失败"
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
                            inbound
                                .input_parts
                                .insert(index + 1, transcript_part(&text));
                            return;
                        }
                        Err(error) => {
                            debug!(
                                chain_index,
                                provider = %candidate.provider,
                                model = %candidate.model,
                                error_class = error.kind(),
                                "OneBot 11 语音 STT 候选失败，尝试下一候选"
                            );
                            last_error = error;
                        }
                    }
                }
            }
        }
        debug!(
            error_class = last_error.kind(),
            "OneBot 11 语音转写候选链全部失败"
        );
        inbound
            .input_parts
            .insert(index + 1, failure_placeholder_part());
    }

    fn voice_action_timeout(&self) -> Duration {
        self.connection
            .request_timeout()
            .max(VOICE_ACTION_MIN_TIMEOUT)
    }

    async fn platform_transcript(&self, message_id: &str) -> Option<String> {
        if self.platform_asr_unsupported() {
            return None;
        }
        for attempt in 0..self.config.platform_asr_attempts.max(1) {
            if attempt > 0 {
                sleep(PLATFORM_ASR_RETRY_DELAY).await;
            }
            match self
                .connection
                .call_with_timeout(
                    "fetch_ptt_text",
                    json!({ "message_id": message_id }),
                    self.voice_action_timeout(),
                )
                .await
            {
                Ok(response) if response.retcode == 0 => {
                    let text = response
                        .data
                        .get("text")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|text| usable_transcript(text));
                    if let Some(text) = text {
                        info!(source = "platform_asr", "OneBot 11 语音已通过平台转写识别");
                        return Some(text.to_owned());
                    }
                }
                Ok(response) if response.retcode == 1404 => {
                    self.mark_platform_asr_unsupported();
                    info!(
                        source = "platform_asr",
                        "OneBot 客户端不支持 fetch_ptt_text，后续语音将跳过平台候选"
                    );
                    return None;
                }
                Ok(_) => {}
                Err(OneBotCallError::NotConnected | OneBotCallError::ConnectionClosed) => {
                    // 连接已断开时不再重试或回退，避免对已失效 transport 继续发请求。
                    return None;
                }
                Err(_) => {}
            }
        }
        debug!(
            source = "platform_asr",
            attempts = self.config.platform_asr_attempts,
            "OneBot 11 平台转写未返回可用文本"
        );
        None
    }

    /// 平台转码优先；同一轮多个远程候选复用同一份音频，避免重复触发 ffmpeg。
    async fn load_audio(
        &self,
        stt: &SttService,
        media: &MessageMedia,
    ) -> Result<SttAudio, SttError> {
        if let Some(file_token) = media
            .file_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            && !self.get_record_unsupported()
        {
            match self.get_record(file_token).await {
                Ok(Some(bytes)) => return stt.prepare_audio(bytes, Some("voice.mp3")),
                Ok(None) => {}
                Err(error) => {
                    debug!(
                        source = "get_record",
                        error_class = error.kind(),
                        "OneBot 11 语音转码失败，尝试原始 URL"
                    );
                }
            }
        }
        match media.url.as_deref().map(str::trim) {
            Some(url) if !url.is_empty() => stt.fetch_audio(url).await,
            _ => Err(SttError::DownloadFailed),
        }
    }

    async fn get_record(&self, file_token: &str) -> Result<Option<Vec<u8>>, SttError> {
        let response = self
            .connection
            .call_with_timeout(
                "get_record",
                json!({ "file": file_token, "out_format": "mp3" }),
                self.voice_action_timeout(),
            )
            .await
            .map_err(|_| SttError::DownloadFailed)?;
        if response.retcode == 1404 {
            self.mark_get_record_unsupported();
            return Ok(None);
        }
        if response.retcode != 0 {
            return Err(SttError::DownloadFailed);
        }
        let encoded = response
            .data
            .get("base64")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let Some(encoded) = encoded else {
            return Ok(None);
        };
        let bytes = STANDARD
            .decode(encoded)
            .map_err(|_| SttError::InvalidResponse)?;
        if bytes.is_empty() {
            return Err(SttError::InvalidResponse);
        }
        Ok(Some(bytes))
    }

    fn platform_asr_unsupported(&self) -> bool {
        let mut cache = self
            .capabilities
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let generation = refresh_capability_generation(&mut cache, self.connection.generation());
        cache.generation = generation;
        cache.platform_asr_unsupported
    }

    fn mark_platform_asr_unsupported(&self) {
        let mut cache = self
            .capabilities
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let generation = refresh_capability_generation(&mut cache, self.connection.generation());
        cache.generation = generation;
        cache.platform_asr_unsupported = true;
    }

    fn get_record_unsupported(&self) -> bool {
        let mut cache = self
            .capabilities
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let generation = refresh_capability_generation(&mut cache, self.connection.generation());
        cache.generation = generation;
        cache.get_record_unsupported
    }

    fn mark_get_record_unsupported(&self) {
        let mut cache = self
            .capabilities
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let generation = refresh_capability_generation(&mut cache, self.connection.generation());
        cache.generation = generation;
        cache.get_record_unsupported = true;
    }
}

/// 连接 generation 变化时清空上一连接的探测结果。
fn refresh_capability_generation(
    cache: &mut CapabilityCache,
    generation: Option<u64>,
) -> Option<u64> {
    if cache.generation != generation {
        *cache = CapabilityCache {
            generation,
            ..CapabilityCache::default()
        };
    }
    generation
}

/// 选择第一个可识别的音频文件段；语音只转写一次。
fn first_voice_media(inbound: &InboundMessage) -> Option<(usize, MessageMedia)> {
    inbound
        .input_parts
        .iter()
        .enumerate()
        .find_map(|(index, part)| {
            let media = part.media()?;
            is_voice_media(media).then(|| (index, media.clone()))
        })
}

fn is_voice_media(media: &MessageMedia) -> bool {
    media.media_id.as_deref() == Some(RECORD_MEDIA_ID)
}

/// 平台转写至少需要一个字母、数字或 CJK 字符，纯标点视为失败。
fn usable_transcript(text: &str) -> bool {
    text.chars().any(char::is_alphanumeric)
}

fn transcript_part(text: &str) -> MessageInputPart {
    MessageInputPart::Text {
        text: format!("{TRANSCRIPT_PREFIX} {text}"),
        source: Some(TextSource::Transcript),
    }
}

fn failure_placeholder_part() -> MessageInputPart {
    MessageInputPart::Text {
        text: FAILED_PLACEHOLDER.to_owned(),
        source: Some(TextSource::Context),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qq_maid_common::input_part::MediaStatus;

    fn media(mime: &str, filename: &str) -> MessageMedia {
        MessageMedia {
            mime_type: Some(mime.to_owned()),
            filename: Some(filename.to_owned()),
            file_id: Some("abc".to_owned()),
            status: MediaStatus::Available,
            ..Default::default()
        }
    }

    #[test]
    fn punctuation_only_platform_text_is_not_usable() {
        assert!(!usable_transcript("。。。？！"));
        assert!(usable_transcript("你好"));
        assert!(usable_transcript("hello"));
    }

    #[test]
    fn voice_media_detection_requires_record_marker() {
        let mut record = media("audio/silk", "voice.silk");
        record.media_id = Some(RECORD_MEDIA_ID.to_owned());
        assert!(is_voice_media(&record));
        // 普通 file 段即使 MIME 是音频也不会被当作语音转写。
        assert!(!is_voice_media(&media("audio/mpeg", "song.mp3")));
        assert!(!is_voice_media(&media("application/pdf", "doc.pdf")));
    }
}
