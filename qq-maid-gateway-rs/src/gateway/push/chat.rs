//! `send_message` 基础设施 Tool 的 Gateway 出站实现。
//!
//! Core 只提供平台无关目标与文本，本模块负责 QQ 官方、OneBot 11 和微信客服的
//! 真实发送、分段、引用索引与拟人间隔。所有失败都以真实平台结果为准。

use async_trait::async_trait;
use qq_maid_core::runtime::{
    push::{ONEBOT11_PLATFORM, PushTarget, PushTargetType, QQ_OFFICIAL_PLATFORM},
    tools::message::{
        ChatSendError, ChatSendIntent, ChatSendResult, ChatSendSink, SentChatMessage,
    },
};
use tracing::warn;

use crate::{
    api::SendMessageIds,
    gateway::{
        logging::{mask_identifier, mask_openid},
        outbound::{ReplyTarget, RuntimeRecordingGroupSender, RuntimeRecordingSender},
        platform::ConversationTarget,
        ref_index::SharedRefIndex,
        wechat_service::customer::WECHAT_CUSTOMER_TEXT_MAX_BYTES,
    },
    message_chunk::{send_c2c_outbound_chunked, send_group_outbound_chunked},
    render::OutboundMessage,
};

use super::{
    GatewayPushRuntime, GatewayPushSink, OneBotPushRuntime, RoutedPushRuntime, WechatPushRuntime,
};

impl OneBotPushRuntime {
    async fn send_chat_messages(
        &self,
        intent: ChatSendIntent,
    ) -> Result<ChatSendResult, ChatSendError> {
        let target_id = intent.target.target_id.trim();
        if target_id.is_empty() {
            return Err(ChatSendError::TargetMissing);
        }
        let target_account = intent
            .target
            .account_id
            .as_deref()
            .map(str::trim)
            .filter(|account| !account.is_empty())
            .ok_or(ChatSendError::Unavailable)?;
        let connected_account = self
            .sender
            .connected_account_id()
            .ok_or(ChatSendError::Unavailable)?;
        if target_account != connected_account {
            return Err(ChatSendError::Unavailable);
        }
        let mut sent = Vec::new();
        // 首条消息前的拟人等待；私聊同时显示平台“正在输入”，群聊没有该状态。
        if let Some(delay) = intent.initial_delay {
            if intent.target.target_type == PushTargetType::Private {
                let sender = self.sender.clone();
                let target_id = target_id.to_owned();
                tokio::join!(
                    async move { sender.send_typing(&target_id).await },
                    tokio::time::sleep(delay)
                );
            } else {
                tokio::time::sleep(delay).await;
            }
        }
        for (index, text) in intent.messages.iter().enumerate() {
            let text = text.trim();
            if text.is_empty() {
                return Err(ChatSendError::Send);
            }
            let result = match intent.target.target_type {
                PushTargetType::Private => self.sender.send_private_text(target_id, text).await,
                PushTargetType::Group => self.sender.send_group_text(target_id, text).await,
            };
            match result {
                Ok(result) => {
                    record_onebot_chat_ref_index(
                        &self.ref_index,
                        &intent.target,
                        target_account,
                        &result.message_id,
                        text,
                    );
                    let message = SentChatMessage {
                        text: text.to_owned(),
                        message_id: Some(result.message_id),
                    };
                    if let Some(progress) = intent.on_sent.as_ref() {
                        progress.notify(message.clone());
                    }
                    sent.push(message);
                }
                Err(error) => {
                    warn!(
                        target_type = %intent.target.target_type.as_str(),
                        target = %mask_identifier(target_id),
                        error = %error,
                        "Gateway OneBot 发言工具发送失败"
                    );
                    return Err(if sent.is_empty() {
                        ChatSendError::Send
                    } else {
                        ChatSendError::Partial { sent }
                    });
                }
            }
            // 最后一条之后不再停顿；停顿只用于模拟拟人化发送间隔。
            if index + 1 < intent.messages.len()
                && let Some(gap) = intent.gap
            {
                tokio::time::sleep(gap).await;
            }
        }
        Ok(ChatSendResult { sent })
    }
}

impl GatewayPushRuntime {
    async fn send_chat_messages(
        &self,
        intent: ChatSendIntent,
    ) -> Result<ChatSendResult, ChatSendError> {
        let target_id = intent.target.target_id.trim();
        if target_id.is_empty() {
            return Err(ChatSendError::TargetMissing);
        }
        if let Some(account) = intent
            .target
            .account_id
            .as_deref()
            .map(str::trim)
            .filter(|account| !account.is_empty())
            && account != self.qq_official_account_id
        {
            return Err(ChatSendError::Unavailable);
        }
        let mut sent = Vec::new();
        if let Some(delay) = intent.initial_delay {
            tokio::time::sleep(delay).await;
        }
        for (index, text) in intent.messages.iter().enumerate() {
            let text = text.trim();
            if text.is_empty() {
                return Err(ChatSendError::Send);
            }
            let outbound = OutboundMessage::Text {
                text: text.to_owned(),
            };
            let reply_to = intent.reply_to_message_id.clone();
            let result = match intent.target.target_type {
                PushTargetType::Private => {
                    let Some(target) =
                        ReplyTarget::qq_c2c(target_id.to_owned(), reply_to).to_qq_c2c_target()
                    else {
                        return Err(ChatSendError::TargetMissing);
                    };
                    let sender = RuntimeRecordingSender {
                        inner: &self.api,
                        runtime: &self.runtime,
                    };
                    send_c2c_outbound_chunked(
                        &sender,
                        &target,
                        &outbound,
                        &self.chunk_limits,
                        |_, _| {},
                    )
                    .await
                }
                PushTargetType::Group => {
                    let Some(target) =
                        ReplyTarget::qq_group(target_id.to_owned(), reply_to).to_qq_group_target()
                    else {
                        return Err(ChatSendError::TargetMissing);
                    };
                    let sender = RuntimeRecordingGroupSender {
                        inner: &self.api,
                        runtime: &self.runtime,
                    };
                    send_group_outbound_chunked(
                        &sender,
                        &target,
                        &outbound,
                        &self.chunk_limits,
                        |_, _| {},
                    )
                    .await
                }
            };
            match result {
                Ok(ids) => {
                    self.record_chat_ref_index(&intent.target, &ids, text);
                    let message = SentChatMessage {
                        text: text.to_owned(),
                        message_id: ids.iter().find_map(|ids| ids.message_id.clone()),
                    };
                    if let Some(progress) = intent.on_sent.as_ref() {
                        progress.notify(message.clone());
                    }
                    sent.push(message);
                }
                Err(error) => {
                    // 当前消息已送达部分分段时也要计入账本，否则最终正文会重复外发。
                    if error.has_partial_progress() {
                        let message = SentChatMessage {
                            text: text.to_owned(),
                            message_id: None,
                        };
                        if let Some(progress) = intent.on_sent.as_ref() {
                            progress.notify(message.clone());
                        }
                        sent.push(message);
                    }
                    warn!(
                        target_type = %intent.target.target_type.as_str(),
                        target = %mask_openid(target_id),
                        error = %error,
                        "Gateway QQ 发言工具发送失败"
                    );
                    return Err(if sent.is_empty() {
                        ChatSendError::Send
                    } else {
                        ChatSendError::Partial { sent }
                    });
                }
            }
            // 最后一条之后不再停顿；停顿只用于模拟拟人化发送间隔。
            if index + 1 < intent.messages.len()
                && let Some(gap) = intent.gap
            {
                tokio::time::sleep(gap).await;
            }
        }
        Ok(ChatSendResult { sent })
    }

    fn record_chat_ref_index(&self, target: &PushTarget, ids: &[SendMessageIds], text: &str) {
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
                    target = %mask_openid(&target.target_id),
                    "ref_index 锁已中毒，跳过写入 QQ 发言引用索引"
                );
                return;
            }
        };
        for ids in ids {
            ref_index.insert_bot_outbound(
                crate::gateway::platform::Platform::QqOfficial,
                Some(&self.qq_official_account_id),
                &conversation,
                ids.ref_index_lookup_id().map(str::to_owned),
                text,
                None,
            );
        }
    }
}

impl WechatPushRuntime {
    async fn send_chat_messages(
        &self,
        intent: ChatSendIntent,
    ) -> Result<ChatSendResult, ChatSendError> {
        let touser = intent.target.target_id.trim();
        if touser.is_empty() {
            return Err(ChatSendError::TargetMissing);
        }
        let mut sent = Vec::new();
        if let Some(delay) = intent.initial_delay {
            tokio::time::sleep(delay).await;
        }
        for (index, text) in intent.messages.iter().enumerate() {
            let text = text.trim();
            if text.is_empty() {
                return Err(ChatSendError::Send);
            }
            let mut chunks_sent = 0usize;
            let mut chunk_failed = false;
            for chunk in crate::message_chunk::chunk_plain_text_by_utf8_bytes(
                text,
                WECHAT_CUSTOMER_TEXT_MAX_BYTES,
            ) {
                if self.messenger.send_text(touser, chunk).await.is_err() {
                    chunk_failed = true;
                    break;
                }
                chunks_sent += 1;
            }
            if chunk_failed {
                // 当前消息已有分片送达时也计入账本，避免最终正文重复外发。
                if chunks_sent > 0 {
                    let message = SentChatMessage {
                        text: text.to_owned(),
                        message_id: None,
                    };
                    if let Some(progress) = intent.on_sent.as_ref() {
                        progress.notify(message.clone());
                    }
                    sent.push(message);
                }
                return Err(if sent.is_empty() {
                    ChatSendError::Send
                } else {
                    ChatSendError::Partial { sent }
                });
            }
            let message = SentChatMessage {
                text: text.to_owned(),
                message_id: None,
            };
            if let Some(progress) = intent.on_sent.as_ref() {
                progress.notify(message.clone());
            }
            sent.push(message);
            // 最后一条之后不再停顿；停顿只用于模拟拟人化发送间隔。
            if index + 1 < intent.messages.len()
                && let Some(gap) = intent.gap
            {
                tokio::time::sleep(gap).await;
            }
        }
        Ok(ChatSendResult { sent })
    }
}

fn record_onebot_chat_ref_index(
    ref_index: &SharedRefIndex,
    target: &PushTarget,
    account_id: &str,
    message_id: &str,
    text: &str,
) {
    let conversation = match target.target_type {
        PushTargetType::Private => ConversationTarget::Private {
            target_id: target.target_id.clone(),
        },
        PushTargetType::Group => ConversationTarget::Group {
            target_id: target.target_id.clone(),
        },
    };
    let mut ref_index = match ref_index.lock() {
        Ok(ref_index) => ref_index,
        Err(_) => return,
    };
    ref_index.insert_bot_outbound(
        crate::gateway::platform::Platform::OneBot11,
        Some(account_id),
        &conversation,
        Some(message_id.to_owned()),
        text,
        None,
    );
}

#[async_trait]
impl ChatSendSink for GatewayPushSink {
    async fn send_messages(&self, intent: ChatSendIntent) -> Result<ChatSendResult, ChatSendError> {
        let platform = match intent.target.platform.trim() {
            "onebot" => ONEBOT11_PLATFORM,
            other => other,
        };
        if !matches!(
            platform,
            QQ_OFFICIAL_PLATFORM | ONEBOT11_PLATFORM | "wechat_service"
        ) {
            return Err(ChatSendError::UnsupportedPlatform);
        }
        let runtime = self
            .runtime(platform)
            .await
            .map_err(|_| ChatSendError::Unavailable)?;
        match runtime {
            RoutedPushRuntime::QqOfficial(runtime) => runtime.send_chat_messages(intent).await,
            RoutedPushRuntime::OneBot11(runtime) => runtime.send_chat_messages(intent).await,
            RoutedPushRuntime::Wechat(runtime) => runtime.send_chat_messages(intent).await,
        }
    }
}
