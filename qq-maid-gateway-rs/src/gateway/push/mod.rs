//! Gateway 进程内主动推送实现与平台路由。
//!
//! Core 只通过 `PushSink` 交付推送意图；本模块按 platform/account 精确选择 sender。
//! QQ 官方继续负责 Markdown fallback 和群消息缓存；OneBot 使用原生消息 segment。

mod chat;
mod mention;

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use qq_maid_core::runtime::{
    push::{
        ONEBOT11_PLATFORM, PushError, PushIntent, PushResult, PushSink, PushTarget, PushTargetType,
        QQ_OFFICIAL_PLATFORM, normalize_push_mentions,
    },
    tools::voice::{VoiceSendError, VoiceSendIntent, VoiceSendResult, VoiceSendSink},
};
use tokio::{sync::Notify, time::Instant};
use tracing::{info, warn};

use mention::{
    PreparedQqBot2Content, mention_display_names, partition_onebot_mentions,
    prepare_qq_bot2_content, prepend_mention_notice,
};

use crate::{
    api::{
        ApiError, QqApiClient, SendMessageIds, SendResult, build_c2c_text_payload,
        build_group_text_payload,
    },
    gateway::{
        BotOutboundCache,
        logging::mask_identifier,
        onebot11::{OneBotSendError, OneBotSendResult, OneBotSender},
        ping::GatewayRuntimeStatus,
        platform::ConversationTarget,
        ref_index::SharedRefIndex,
        wechat_service::customer::WechatCustomerMessenger,
    },
    markdown::MarkdownPayload,
    message_chunk::ChunkLimits,
    tts::{DynTtsProvider, TtsAudio, TtsProvider},
};

#[async_trait]
trait PushQqSender: Send + Sync {
    async fn send_c2c_text(&self, target_id: &str, text: &str) -> SendResult;
    async fn send_c2c_markdown(&self, target_id: &str, markdown: &MarkdownPayload) -> SendResult;
    async fn send_group_text(&self, target_id: &str, text: &str) -> SendResult;
    async fn send_group_markdown(&self, target_id: &str, markdown: &MarkdownPayload) -> SendResult;
}

#[async_trait]
impl PushQqSender for QqApiClient {
    async fn send_c2c_text(&self, target_id: &str, text: &str) -> SendResult {
        QqApiClient::send_c2c_text(self, target_id, None, text).await
    }

    async fn send_c2c_markdown(&self, target_id: &str, markdown: &MarkdownPayload) -> SendResult {
        QqApiClient::send_c2c_markdown(self, target_id, None, markdown).await
    }

    async fn send_group_text(&self, target_id: &str, text: &str) -> SendResult {
        QqApiClient::send_group_text(self, target_id, None, text).await
    }

    async fn send_group_markdown(&self, target_id: &str, markdown: &MarkdownPayload) -> SendResult {
        QqApiClient::send_group_markdown(self, target_id, None, markdown).await
    }
}

#[derive(Clone)]
pub struct GatewayPushSink {
    inner: Arc<Mutex<GatewayPushState>>,
    ready: Arc<Notify>,
    /// 语音工具使用的 TTS Provider；启动时按已生效配置注入。
    voice_provider: Arc<Mutex<Option<DynTtsProvider>>>,
}

#[derive(Clone)]
struct GatewayPushState {
    qq_official: PushChannelState<GatewayPushRuntime>,
    onebot11: PushChannelState<OneBotPushRuntime>,
    wechat: PushChannelState<WechatPushRuntime>,
}

#[derive(Clone)]
enum PushChannelState<T> {
    Pending,
    Bound(T),
    Unavailable(&'static str),
}

#[derive(Clone)]
enum RoutedPushRuntime {
    QqOfficial(GatewayPushRuntime),
    OneBot11(OneBotPushRuntime),
    Wechat(WechatPushRuntime),
}

// 三个平台运行时的尺寸不均衡；路由枚举只在短生命周期内使用，保留直接内联实现。
#[allow(clippy::large_enum_variant)]
enum RouteSnapshot {
    Pending,
    Bound(RoutedPushRuntime),
    Unavailable(&'static str),
}

#[derive(Clone)]
struct GatewayPushRuntime {
    api: QqApiClient,
    qq_official_account_id: String,
    runtime: GatewayRuntimeStatus,
    group_outbound_cache: Arc<Mutex<BotOutboundCache>>,
    ref_index: SharedRefIndex,
    chunk_limits: ChunkLimits,
}

#[async_trait]
trait PushOneBotSender: Send + Sync {
    fn connected_account_id(&self) -> Option<String>;
    /// 尽力显示私聊“正在输入”；失败不能影响真实发送。
    async fn send_typing(&self, target_id: &str);
    async fn send_private_text(
        &self,
        target_id: &str,
        text: &str,
    ) -> Result<OneBotSendResult, OneBotSendError>;
    async fn send_group_text(
        &self,
        target_id: &str,
        text: &str,
    ) -> Result<OneBotSendResult, OneBotSendError>;
    async fn send_group_text_with_mentions(
        &self,
        target_id: &str,
        mention_user_ids: &[String],
        text: &str,
    ) -> Result<OneBotSendResult, OneBotSendError>;
    async fn send_private_voice(
        &self,
        target_id: &str,
        file: &str,
    ) -> Result<OneBotSendResult, OneBotSendError>;
    async fn send_group_voice(
        &self,
        target_id: &str,
        file: &str,
    ) -> Result<OneBotSendResult, OneBotSendError>;
}

#[async_trait]
impl PushOneBotSender for OneBotSender {
    fn connected_account_id(&self) -> Option<String> {
        OneBotSender::connected_account_id(self)
    }

    async fn send_typing(&self, target_id: &str) {
        OneBotSender::send_typing(self, target_id).await;
    }

    async fn send_private_text(
        &self,
        target_id: &str,
        text: &str,
    ) -> Result<OneBotSendResult, OneBotSendError> {
        OneBotSender::send_private_text(self, target_id, text).await
    }

    async fn send_group_text(
        &self,
        target_id: &str,
        text: &str,
    ) -> Result<OneBotSendResult, OneBotSendError> {
        OneBotSender::send_group_text(self, target_id, text).await
    }

    async fn send_group_text_with_mentions(
        &self,
        target_id: &str,
        mention_user_ids: &[String],
        text: &str,
    ) -> Result<OneBotSendResult, OneBotSendError> {
        OneBotSender::send_group_text_with_mentions(self, target_id, mention_user_ids, text).await
    }

    async fn send_private_voice(
        &self,
        target_id: &str,
        file: &str,
    ) -> Result<OneBotSendResult, OneBotSendError> {
        OneBotSender::send_private_voice(self, target_id, file).await
    }

    async fn send_group_voice(
        &self,
        target_id: &str,
        file: &str,
    ) -> Result<OneBotSendResult, OneBotSendError> {
        OneBotSender::send_group_voice(self, target_id, file).await
    }
}

#[derive(Clone)]
struct OneBotPushRuntime {
    sender: Arc<dyn PushOneBotSender>,
    ref_index: SharedRefIndex,
}

#[derive(Clone)]
struct WechatPushRuntime {
    messenger: Arc<dyn WechatCustomerMessenger>,
}

#[derive(Debug)]
struct PushSendOutcome {
    ids: SendMessageIds,
    delivered_text: String,
}

impl GatewayPushSink {
    pub fn unbound() -> Self {
        Self {
            inner: Arc::new(Mutex::new(GatewayPushState {
                qq_official: PushChannelState::Pending,
                onebot11: PushChannelState::Pending,
                wechat: PushChannelState::Pending,
            })),
            ready: Arc::new(Notify::new()),
            voice_provider: Arc::new(Mutex::new(None)),
        }
    }

    /// 注入语音工具使用的 TTS Provider。
    ///
    /// 配置不可用或未配置 Provider 时注入 `None`，语音工具在 Core 侧也不会注册。
    pub fn set_voice_provider(&self, provider: Option<DynTtsProvider>) {
        *self
            .voice_provider
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = provider;
    }

    fn voice_provider(&self) -> Option<DynTtsProvider> {
        self.voice_provider
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn bind(
        &self,
        api: QqApiClient,
        qq_official_account_id: impl Into<String>,
        runtime: GatewayRuntimeStatus,
        group_outbound_cache: Arc<Mutex<BotOutboundCache>>,
        ref_index: SharedRefIndex,
        chunk_limits: ChunkLimits,
    ) {
        // Core scheduler 可能在 Gateway 首次连接 QQ 前启动，因此 sink 需要先存在；
        // 真正发送前必须已绑定运行期上下文，否则返回可观测错误而不是静默丢消息。
        self.inner.lock().unwrap().qq_official = PushChannelState::Bound(GatewayPushRuntime {
            api,
            qq_official_account_id: qq_official_account_id.into(),
            runtime,
            group_outbound_cache,
            ref_index,
            chunk_limits,
        });
        self.ready.notify_waiters();
    }

    pub(crate) fn mark_qq_official_unavailable(&self, summary: &'static str) {
        self.inner.lock().unwrap().qq_official = PushChannelState::Unavailable(summary);
        self.ready.notify_waiters();
    }

    pub(crate) fn bind_onebot11(&self, sender: OneBotSender, ref_index: SharedRefIndex) {
        self.bind_onebot_sender(Arc::new(sender), ref_index);
    }

    fn bind_onebot_sender(&self, sender: Arc<dyn PushOneBotSender>, ref_index: SharedRefIndex) {
        self.inner.lock().unwrap().onebot11 =
            PushChannelState::Bound(OneBotPushRuntime { sender, ref_index });
        self.ready.notify_waiters();
    }

    pub(crate) fn mark_onebot11_unavailable(&self, summary: &'static str) {
        self.inner.lock().unwrap().onebot11 = PushChannelState::Unavailable(summary);
        self.ready.notify_waiters();
    }

    /// 绑定微信服务号客服文本发送能力。
    pub(crate) fn bind_wechat(&self, messenger: Arc<dyn WechatCustomerMessenger>) {
        self.inner.lock().unwrap().wechat =
            PushChannelState::Bound(WechatPushRuntime { messenger });
        self.ready.notify_waiters();
    }

    pub(crate) fn mark_wechat_unavailable(&self, summary: &'static str) {
        self.inner.lock().unwrap().wechat = PushChannelState::Unavailable(summary);
        self.ready.notify_waiters();
    }

    async fn runtime(&self, platform: &str) -> Result<RoutedPushRuntime, PushError> {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            // 先创建 waiter 再读取状态，避免 bind 恰好发生在状态读取和等待之间时漏通知。
            let notified = self.ready.notified();
            match self.route_snapshot(platform)? {
                RouteSnapshot::Bound(runtime) => return Ok(runtime),
                RouteSnapshot::Unavailable(summary) => {
                    return Err(PushError::Failed {
                        summary: summary.to_owned(),
                    });
                }
                RouteSnapshot::Pending => {}
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return Err(PushError::Failed {
                    summary: format!("gateway push sink for `{platform}` is not ready"),
                });
            }
        }
    }

    fn route_snapshot(&self, platform: &str) -> Result<RouteSnapshot, PushError> {
        let state = self.inner.lock().unwrap();
        match platform {
            QQ_OFFICIAL_PLATFORM => Ok(match &state.qq_official {
                PushChannelState::Pending => RouteSnapshot::Pending,
                PushChannelState::Bound(runtime) => {
                    RouteSnapshot::Bound(RoutedPushRuntime::QqOfficial(runtime.clone()))
                }
                PushChannelState::Unavailable(summary) => RouteSnapshot::Unavailable(summary),
            }),
            ONEBOT11_PLATFORM => Ok(match &state.onebot11 {
                PushChannelState::Pending => RouteSnapshot::Pending,
                PushChannelState::Bound(runtime) => {
                    RouteSnapshot::Bound(RoutedPushRuntime::OneBot11(runtime.clone()))
                }
                PushChannelState::Unavailable(summary) => RouteSnapshot::Unavailable(summary),
            }),
            "wechat_service" => Ok(match &state.wechat {
                PushChannelState::Pending => RouteSnapshot::Pending,
                PushChannelState::Bound(runtime) => {
                    RouteSnapshot::Bound(RoutedPushRuntime::Wechat(runtime.clone()))
                }
                PushChannelState::Unavailable(summary) => RouteSnapshot::Unavailable(summary),
            }),
            other => Err(PushError::Failed {
                summary: format!("push platform `{other}` is not supported by gateway"),
            }),
        }
    }
}

#[async_trait]
impl PushSink for GatewayPushSink {
    async fn push(&self, intent: PushIntent) -> Result<PushResult, PushError> {
        let platform = intent.target.platform.trim();
        match self.runtime(platform).await? {
            RoutedPushRuntime::QqOfficial(runtime) => runtime.push(intent).await,
            RoutedPushRuntime::OneBot11(runtime) => runtime.push(intent).await,
            RoutedPushRuntime::Wechat(_) => Err(PushError::Failed {
                summary: "wechat service account does not support active push".to_owned(),
            }),
        }
    }
}

#[async_trait]
impl VoiceSendSink for GatewayPushSink {
    async fn send_voice(&self, intent: VoiceSendIntent) -> Result<VoiceSendResult, VoiceSendError> {
        let platform = match intent.target.platform.trim() {
            "onebot" => ONEBOT11_PLATFORM,
            other => other,
        };
        if !matches!(platform, QQ_OFFICIAL_PLATFORM | ONEBOT11_PLATFORM) {
            return Err(VoiceSendError::UnsupportedPlatform);
        }
        let provider = self.voice_provider().ok_or(VoiceSendError::Unavailable)?;
        let runtime = self
            .runtime(platform)
            .await
            .map_err(|_| VoiceSendError::Unavailable)?;
        match runtime {
            RoutedPushRuntime::QqOfficial(runtime) => {
                runtime.send_voice(provider.as_ref(), intent).await
            }
            RoutedPushRuntime::OneBot11(runtime) => {
                runtime.send_voice(provider.as_ref(), intent).await
            }
            RoutedPushRuntime::Wechat(_) => Err(VoiceSendError::UnsupportedPlatform),
        }
    }
}

impl OneBotPushRuntime {
    async fn send_voice(
        &self,
        provider: &dyn TtsProvider,
        intent: VoiceSendIntent,
    ) -> Result<VoiceSendResult, VoiceSendError> {
        let target_id = intent.target.target_id.trim();
        if target_id.is_empty() {
            return Err(VoiceSendError::TargetMissing);
        }
        let target_account = intent
            .target
            .account_id
            .as_deref()
            .map(str::trim)
            .filter(|account| !account.is_empty())
            .ok_or(VoiceSendError::Unavailable)?;
        let connected_account = self
            .sender
            .connected_account_id()
            .ok_or(VoiceSendError::Unavailable)?;
        if target_account != connected_account {
            return Err(VoiceSendError::Unavailable);
        }
        let text = intent.text.trim();
        if text.is_empty() {
            return Err(VoiceSendError::EmptyText);
        }
        // 私聊且存在停顿时才显示“正在输入”；typing 是独立 best-effort 请求，
        // 适配器不支持或响应慢都不能阻塞语音发送。
        let typing = (intent.target.target_type == PushTargetType::Private
            && intent
                .target_delay
                .is_some_and(|target_delay| !target_delay.is_zero()))
        .then(|| {
            let sender = self.sender.clone();
            let target_id = target_id.to_owned();
            tokio::spawn(async move { sender.send_typing(&target_id).await })
        });
        let started_at = Instant::now();
        let audio = match provider.synthesize(text).await {
            Ok(audio) => audio,
            Err(error) => {
                // 合成失败时不再等待，直接返回真实错误。
                if let Some(typing) = typing {
                    typing.abort();
                }
                return Err(VoiceSendError::Tts { code: error.code() });
            }
        };
        // 目标停顿包含 TTS 耗时：合成慢时只补齐剩余差值，合成快时才补停顿。
        if let Some(target_delay) = intent.target_delay {
            let remaining = target_delay.saturating_sub(started_at.elapsed());
            if !remaining.is_zero() {
                tokio::time::sleep(remaining).await;
            }
        }
        let file = onebot_voice_file(audio)?;
        let result = match intent.target.target_type {
            PushTargetType::Private => self.sender.send_private_voice(target_id, &file).await,
            PushTargetType::Group => self.sender.send_group_voice(target_id, &file).await,
        };
        match result {
            Ok(sent) => {
                self.record_voice_ref_index(&intent.target, target_account, &sent.message_id, text);
                info!(
                    target_type = %intent.target.target_type.as_str(),
                    target = %mask_identifier(target_id),
                    "Gateway OneBot 语音工具已发送"
                );
                Ok(VoiceSendResult {
                    message_id: Some(sent.message_id),
                })
            }
            Err(error) => {
                warn!(
                    target_type = %intent.target.target_type.as_str(),
                    target = %mask_identifier(target_id),
                    error = %error,
                    "Gateway OneBot 语音工具发送失败"
                );
                Err(voice_error_from_onebot(error))
            }
        }
    }

    fn record_voice_ref_index(
        &self,
        target: &PushTarget,
        account_id: &str,
        message_id: &str,
        spoken_text: &str,
    ) {
        let conversation = match target.target_type {
            PushTargetType::Private => ConversationTarget::Private {
                target_id: target.target_id.clone(),
            },
            PushTargetType::Group => ConversationTarget::Group {
                target_id: target.target_id.clone(),
            },
        };
        let mut ref_index = match self.ref_index.lock() {
            Ok(ref_index) => ref_index,
            Err(_) => {
                warn!(
                    target_type = %target.target_type.as_str(),
                    target = %mask_identifier(&target.target_id),
                    "ref_index 锁已中毒，跳过写入 OneBot 语音引用索引"
                );
                return;
            }
        };
        ref_index.insert_bot_outbound(
            crate::gateway::platform::Platform::OneBot11,
            Some(account_id),
            &conversation,
            Some(message_id.to_owned()),
            spoken_text,
            None,
        );
    }

    async fn push(&self, intent: PushIntent) -> Result<PushResult, PushError> {
        let target_id = intent.target.target_id.trim();
        let text = intent.text.trim();
        if target_id.is_empty() || text.is_empty() {
            return Err(PushError::Failed {
                summary: "target_id and text are required".to_owned(),
            });
        }
        let target_account = intent
            .target
            .account_id
            .as_deref()
            .map(str::trim)
            .filter(|account| !account.is_empty())
            .ok_or_else(|| PushError::Failed {
                summary: "onebot11 push target account_id is required".to_owned(),
            })?;
        let connected_account =
            self.sender
                .connected_account_id()
                .ok_or_else(|| PushError::Failed {
                    summary: "OneBot 11 account is offline".to_owned(),
                })?;
        if target_account != connected_account {
            return Err(PushError::Failed {
                summary: "push target account does not match connected OneBot 11 account"
                    .to_owned(),
            });
        }

        // OneBot 一期只有 text segment；Markdown、图片等结构化意图统一使用上游已生成的
        // 纯文本 fallback，不能把 QQ Markdown payload 或 CQ 码带入 sender。
        let fallback_text = intent
            .fallback_text
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(text);
        let base_delivered_text = if matches!(intent.message_type.trim(), "" | "text") {
            text
        } else {
            fallback_text
        };
        let mentions = normalize_push_mentions(intent.mentions.clone());
        let (valid_mentions, invalid_mentions) = partition_onebot_mentions(&mentions);
        let degraded_names = mention_display_names(&invalid_mentions);
        let delivered_text = if intent.target.target_type == PushTargetType::Group {
            if !invalid_mentions.is_empty() {
                warn!(
                    platform = ONEBOT11_PLATFORM,
                    invalid_mention_count = invalid_mentions.len(),
                    "部分 OneBot 成员 ID 无效，推送中的 mention 已部分降级"
                );
            }
            prepend_mention_notice(base_delivered_text, &degraded_names, false)
        } else {
            base_delivered_text.to_owned()
        };
        let result = match intent.target.target_type {
            PushTargetType::Private => {
                if !mentions.is_empty() {
                    warn!(
                        platform = ONEBOT11_PLATFORM,
                        mention_count = mentions.len(),
                        "私聊消息不支持提及群成员，已忽略推送中的 mention"
                    );
                }
                self.sender
                    .send_private_text(target_id, &delivered_text)
                    .await
            }
            PushTargetType::Group if valid_mentions.is_empty() => {
                self.sender
                    .send_group_text(target_id, &delivered_text)
                    .await
            }
            PushTargetType::Group => {
                self.sender
                    .send_group_text_with_mentions(target_id, &valid_mentions, &delivered_text)
                    .await
            }
        }
        .map_err(|error| PushError::Failed {
            // sender 的错误摘要不会包含消息正文、token 或完整 response envelope。
            summary: error.to_string(),
        })?;
        let conversation = match intent.target.target_type {
            PushTargetType::Private => ConversationTarget::Private {
                target_id: intent.target.target_id.clone(),
            },
            PushTargetType::Group => ConversationTarget::Group {
                target_id: intent.target.target_id.clone(),
            },
        };
        self.ref_index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert_bot_outbound(
                crate::gateway::platform::Platform::OneBot11,
                Some(target_account),
                &conversation,
                Some(result.message_id.clone()),
                &delivered_text,
                intent.visible_entity_snapshot.clone(),
            );
        Ok(PushResult {
            message_id: Some(result.message_id),
        })
    }
}

impl GatewayPushRuntime {
    async fn send_voice(
        &self,
        provider: &dyn TtsProvider,
        intent: VoiceSendIntent,
    ) -> Result<VoiceSendResult, VoiceSendError> {
        let target_id = intent.target.target_id.trim();
        if target_id.is_empty() {
            return Err(VoiceSendError::TargetMissing);
        }
        validate_qq_official_voice_account(&intent.target, &self.qq_official_account_id)?;
        let text = intent.text.trim();
        if text.is_empty() {
            return Err(VoiceSendError::EmptyText);
        }
        // 与 OneBot 保持同一语义：目标停顿包含 TTS 耗时，只补齐剩余差值。
        // QQ 官方的“正在输入”由回合级 typing guard 负责，这里不重复发送。
        let started_at = Instant::now();
        let audio = provider
            .synthesize(text)
            .await
            .map_err(|error| VoiceSendError::Tts { code: error.code() })?;
        if let Some(target_delay) = intent.target_delay {
            let remaining = target_delay.saturating_sub(started_at.elapsed());
            if !remaining.is_zero() {
                tokio::time::sleep(remaining).await;
            }
        }
        let reply_to = intent.reply_to_message_id.as_deref();
        let result = match (intent.target.target_type, audio) {
            (PushTargetType::Private, TtsAudio::Url(url)) => {
                self.api.send_c2c_voice_url(target_id, reply_to, &url).await
            }
            (PushTargetType::Private, TtsAudio::File { bytes, file_name }) => {
                self.api
                    .send_c2c_voice_file(target_id, reply_to, &bytes, &file_name)
                    .await
            }
            (PushTargetType::Group, TtsAudio::Url(url)) => {
                self.api
                    .send_group_voice_url(target_id, reply_to, &url)
                    .await
            }
            (PushTargetType::Group, TtsAudio::File { bytes, file_name }) => {
                self.api
                    .send_group_voice_file(target_id, reply_to, &bytes, &file_name)
                    .await
            }
        };
        match &result {
            Ok(_) => self.runtime.record_qq_send_success(),
            Err(error) => self.runtime.record_qq_send_failure(error.log_summary()),
        }
        match result {
            Ok(ids) => {
                if intent.target.target_type == PushTargetType::Group {
                    let mut cache = self.group_outbound_cache.lock().unwrap();
                    cache.insert(ids.message_id.clone());
                    cache.insert_ref_index_id(ids.ref_index_id.clone());
                }
                self.record_voice_ref_index(&intent.target, &ids, text);
                info!(
                    target_type = %intent.target.target_type.as_str(),
                    target = %mask_identifier(target_id),
                    "Gateway 语音工具已发送"
                );
                Ok(VoiceSendResult {
                    message_id: ids.message_id,
                })
            }
            Err(error) => {
                warn!(
                    target_type = %intent.target.target_type.as_str(),
                    target = %mask_identifier(target_id),
                    error = %error.log_summary(),
                    "Gateway 语音工具发送失败"
                );
                Err(voice_error_from_api(error))
            }
        }
    }

    fn record_voice_ref_index(
        &self,
        target: &PushTarget,
        sent_ids: &SendMessageIds,
        spoken_text: &str,
    ) {
        let Some(ref_index_id) = sent_ids.ref_index_id.as_deref() else {
            return;
        };
        let conversation = match target.target_type {
            PushTargetType::Private => ConversationTarget::Private {
                target_id: target.target_id.clone(),
            },
            PushTargetType::Group => ConversationTarget::Group {
                target_id: target.target_id.clone(),
            },
        };
        let mut ref_index = match self.ref_index.lock() {
            Ok(ref_index) => ref_index,
            Err(_) => {
                warn!(
                    target_type = %target.target_type.as_str(),
                    target = %mask_identifier(&target.target_id),
                    "ref_index 锁已中毒，跳过写入语音引用索引"
                );
                return;
            }
        };
        ref_index.insert_bot_outbound(
            crate::gateway::platform::Platform::QqOfficial,
            Some(&self.qq_official_account_id),
            &conversation,
            Some(ref_index_id.to_owned()),
            spoken_text,
            None,
        );
    }

    async fn push(&self, intent: PushIntent) -> Result<PushResult, PushError> {
        let target_id = intent.target.target_id.trim();
        let text = intent.text.trim();
        if target_id.is_empty() || text.is_empty() {
            return Err(PushError::Failed {
                summary: "target_id and text are required".to_owned(),
            });
        }
        validate_qq_official_target(&intent, &self.qq_official_account_id)?;

        let fallback_text = intent
            .fallback_text
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(text);
        let mentions = normalize_push_mentions(intent.mentions.clone());
        let prepared = prepare_qq_bot2_content(
            intent.target.target_type,
            &mentions,
            text,
            fallback_text,
            intent.message_type.trim(),
        );
        let message_type = intent.message_type.trim();
        let result = match intent.target.target_type {
            PushTargetType::Private => {
                send_private_push(
                    &self.api,
                    target_id,
                    message_type,
                    &prepared.content,
                    &prepared.fallback_content,
                )
                .await
            }
            PushTargetType::Group => {
                send_group_push(&self.api, target_id, message_type, &prepared).await
            }
        };
        match &result {
            Ok(_) => self.runtime.record_qq_send_success(),
            Err(err) => self.runtime.record_qq_send_failure(err.log_summary()),
        }
        match result {
            Ok(outcome) => Ok(self.record_successful_push(&intent, target_id, outcome)),
            Err(err) => {
                warn!(
                    platform = %intent.target.platform,
                    target_type = %intent.target.target_type.as_str(),
                    target = %mask_identifier(target_id),
                    error = %err.log_summary(),
                    "Gateway 推送失败"
                );
                Err(PushError::Failed {
                    summary: err.log_summary(),
                })
            }
        }
    }

    fn record_successful_push(
        &self,
        intent: &PushIntent,
        target_id: &str,
        outcome: PushSendOutcome,
    ) -> PushResult {
        if intent.target.target_type == PushTargetType::Group {
            let mut cache = self.group_outbound_cache.lock().unwrap();
            cache.insert(outcome.ids.message_id.clone());
            cache.insert_ref_index_id(outcome.ids.ref_index_id.clone());
        }
        self.record_push_ref_index(intent, &outcome.ids, &outcome.delivered_text);
        info!(
            platform = %intent.target.platform,
            target_type = %intent.target.target_type.as_str(),
            target = %mask_identifier(target_id),
            "Gateway 推送已发送"
        );
        PushResult {
            message_id: outcome.ids.message_id,
        }
    }

    fn record_push_ref_index(
        &self,
        intent: &PushIntent,
        sent_ids: &SendMessageIds,
        delivered_text: &str,
    ) {
        let Some(ref_index_id) = sent_ids.ref_index_id.as_deref() else {
            return;
        };
        let conversation = match intent.target.target_type {
            PushTargetType::Private => ConversationTarget::Private {
                target_id: intent.target.target_id.clone(),
            },
            PushTargetType::Group => ConversationTarget::Group {
                target_id: intent.target.target_id.clone(),
            },
        };
        let mut ref_index = match self.ref_index.lock() {
            Ok(ref_index) => ref_index,
            Err(_) => {
                warn!(
                    target_type = %intent.target.target_type.as_str(),
                    target = %mask_identifier(&intent.target.target_id),
                    ref_index_id = %mask_identifier(ref_index_id),
                    "ref_index 锁已中毒，跳过写入推送引用索引"
                );
                return;
            }
        };
        ref_index.insert_bot_outbound(
            crate::gateway::platform::Platform::QqOfficial,
            Some(&self.qq_official_account_id),
            &conversation,
            Some(ref_index_id.to_owned()),
            delivered_text,
            intent.visible_entity_snapshot.clone(),
        );
    }
}

fn validate_qq_official_target(
    intent: &PushIntent,
    qq_official_account_id: &str,
) -> Result<(), PushError> {
    let platform = intent.target.platform.trim();
    if platform != QQ_OFFICIAL_PLATFORM {
        let summary = if platform == "wechat_service" {
            "wechat_service proactive customer-service push is not available in this gateway sink"
                .to_owned()
        } else {
            format!("push platform `{platform}` is not supported by qq official gateway sink")
        };
        return Err(PushError::Failed { summary });
    }

    if let Some(account_id) = intent.target.account_id.as_deref().map(str::trim)
        && !account_id.is_empty()
        && account_id != qq_official_account_id.trim()
    {
        return Err(PushError::Failed {
            summary: "push target account does not match bound qq official account".to_owned(),
        });
    }
    Ok(())
}

fn validate_qq_official_voice_account(
    target: &PushTarget,
    qq_official_account_id: &str,
) -> Result<(), VoiceSendError> {
    if let Some(account_id) = target.account_id.as_deref().map(str::trim)
        && !account_id.is_empty()
        && account_id != qq_official_account_id.trim()
    {
        // 账号不匹配时不暴露绑定账号，只返回统一不可用分类。
        return Err(VoiceSendError::Unavailable);
    }
    Ok(())
}

fn voice_error_from_api(error: ApiError) -> VoiceSendError {
    match error {
        ApiError::VoiceUpload(_) => VoiceSendError::Upload,
        ApiError::VoiceSend(_) => VoiceSendError::Send,
        ApiError::Unsupported(_) => VoiceSendError::UnsupportedPlatform,
        _ => VoiceSendError::Send,
    }
}

fn onebot_voice_file(audio: TtsAudio) -> Result<String, VoiceSendError> {
    match audio {
        TtsAudio::Url(url) if !url.trim().is_empty() => Ok(url),
        TtsAudio::Url(_) => Err(VoiceSendError::Tts {
            code: "invalid_audio_url",
        }),
        TtsAudio::File { bytes, .. } if !bytes.is_empty() => {
            Ok(format!("base64://{}", STANDARD.encode(bytes)))
        }
        TtsAudio::File { .. } => Err(VoiceSendError::Tts {
            code: "empty_audio",
        }),
    }
}

fn voice_error_from_onebot(error: OneBotSendError) -> VoiceSendError {
    match error {
        OneBotSendError::InvalidTargetId => VoiceSendError::TargetMissing,
        OneBotSendError::InvalidVoice => VoiceSendError::Send,
        OneBotSendError::Transport(_)
        | OneBotSendError::Rejected { .. }
        | OneBotSendError::InvalidData
        | OneBotSendError::InvalidImage => VoiceSendError::Send,
    }
}

async fn send_private_push<S: PushQqSender + ?Sized>(
    sender: &S,
    target_id: &str,
    message_type: &str,
    text: &str,
    fallback_text: &str,
) -> Result<PushSendOutcome, crate::api::ApiError> {
    match message_type {
        "markdown" => {
            let markdown = MarkdownPayload::new(text.to_owned());
            match sender.send_c2c_markdown(target_id, &markdown).await {
                Ok(ids) => Ok(PushSendOutcome {
                    ids,
                    delivered_text: text.to_owned(),
                }),
                Err(err) => {
                    warn!(
                        target = %mask_identifier(target_id),
                        error = %err.log_summary(),
                        "Markdown 推送失败，将降级为文本推送"
                    );
                    sender
                        .send_c2c_text(target_id, fallback_text)
                        .await
                        .map(|ids| PushSendOutcome {
                            ids,
                            delivered_text: fallback_text.to_owned(),
                        })
                }
            }
        }
        "text" | "" => {
            // 主动推送没有原始 QQ msg_id，因此只发送 content/msg_type/msg_seq。
            let _shape = build_c2c_text_payload(text, None, 1);
            sender
                .send_c2c_text(target_id, text)
                .await
                .map(|ids| PushSendOutcome {
                    ids,
                    delivered_text: text.to_owned(),
                })
        }
        _ => Err(crate::api::ApiError::Unsupported("message_type")),
    }
}

async fn send_group_push<S: PushQqSender + ?Sized>(
    sender: &S,
    target_id: &str,
    message_type: &str,
    prepared: &PreparedQqBot2Content,
) -> Result<PushSendOutcome, crate::api::ApiError> {
    match message_type {
        "markdown" => {
            let markdown = MarkdownPayload::new(prepared.content.clone());
            match sender.send_group_markdown(target_id, &markdown).await {
                Ok(ids) => Ok(PushSendOutcome {
                    ids,
                    delivered_text: prepared.ref_index_content.clone(),
                }),
                Err(err) => {
                    warn!(
                        target = %mask_identifier(target_id),
                        error = %err.log_summary(),
                        "群聊 Markdown 推送失败，将降级为文本推送"
                    );
                    sender
                        .send_group_text(target_id, &prepared.fallback_content)
                        .await
                        .map(|ids| PushSendOutcome {
                            ids,
                            delivered_text: prepared.fallback_ref_index_content.clone(),
                        })
                }
            }
        }
        "text" | "" => {
            // QQ 群 openid 主动消息使用 /v2/groups/{group_openid}/messages。
            let _shape = build_group_text_payload(&prepared.content, None, 1);
            sender
                .send_group_text(target_id, &prepared.content)
                .await
                .map(|ids| PushSendOutcome {
                    ids,
                    delivered_text: prepared.ref_index_content.clone(),
                })
        }
        _ => Err(crate::api::ApiError::Unsupported("message_type")),
    }
}

#[cfg(test)]
mod tests;
