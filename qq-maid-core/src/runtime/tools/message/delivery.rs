//! 模型发言工具到平台发送层的跨层契约。
//!
//! Core 只表达“把这几段文本发到当前会话”，不携带 QQ/OneBot/微信协议字段。
//! 平台分段、被动回复预算和真实发送仍由 Gateway 实现。

use std::{
    collections::HashSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use thiserror::Error;

use crate::runtime::push::PushTarget;

/// 单条已成功交付的发言。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentChatMessage {
    /// 服务端清洗后的原文；用于会话历史，不使用平台分段后的片段。
    pub text: String,
    pub message_id: Option<String>,
}

/// 一次模型发言意图。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatSendIntent {
    /// 由服务端根据当前请求生成，不能来自模型参数。
    pub target: PushTarget,
    /// 当前入站消息 ID；存在时优先作为 QQ 被动回复发送。
    pub reply_to_message_id: Option<String>,
    /// 已完成清洗和长度校验的文本，按模型给出的顺序发送。
    pub messages: Vec<String>,
    /// 首条消息前的单次拟人延迟；由 Core 按配置采样并按工具预算裁剪。
    pub initial_delay: Option<Duration>,
    /// 相邻两条消息之间的拟人停顿；由服务端决定，模型不能控制。
    pub gap: Option<Duration>,
    /// 每条逻辑消息真实落地后的增量回调。
    ///
    /// 超时或取消会直接丢弃 sink future；调用方通过该回调登记已经送达的消息，
    /// 避免账本少计导致最终文本重复外发。
    pub on_sent: Option<ChatSendProgress>,
}

/// 单条消息真实落地后的增量回调。
#[derive(Clone)]
pub struct ChatSendProgress(Arc<dyn Fn(SentChatMessage) + Send + Sync>);

impl ChatSendProgress {
    pub fn new(callback: impl Fn(SentChatMessage) + Send + Sync + 'static) -> Self {
        Self(Arc::new(callback))
    }

    pub fn notify(&self, message: SentChatMessage) {
        (self.0)(message);
    }
}

impl std::fmt::Debug for ChatSendProgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ChatSendProgress")
    }
}

impl PartialEq for ChatSendProgress {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for ChatSendProgress {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatSendResult {
    pub sent: Vec<SentChatMessage>,
}

/// 发言发送错误分类。
///
/// 对模型只暴露稳定错误码和可展示文案；平台响应体和账号信息只能留在
/// Gateway 的脱敏日志中。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ChatSendError {
    #[error("chat target platform is not supported")]
    UnsupportedPlatform,
    #[error("chat sending is unavailable")]
    Unavailable,
    #[error("chat target is missing")]
    TargetMissing,
    #[error("chat send timed out")]
    Timeout,
    #[error("chat send failed")]
    Send,
    /// 已成功发送一部分后失败；`sent` 中的消息已经真实落地。
    #[error("chat send partially failed")]
    Partial { sent: Vec<SentChatMessage> },
}

impl ChatSendError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedPlatform => "unsupported_platform",
            Self::Unavailable => "chat_send_unavailable",
            Self::TargetMissing => "target_missing",
            Self::Timeout => "timeout",
            Self::Send => "send_failed",
            Self::Partial { .. } => "partial_failure",
        }
    }

    pub fn user_message(&self) -> &'static str {
        match self {
            Self::UnsupportedPlatform => "当前入口暂不支持发送消息。",
            Self::Unavailable => "消息发送暂不可用，本次未发送。",
            Self::TargetMissing => "当前会话缺少发送目标，本次未发送。",
            Self::Timeout => "消息发送超时，本次是否送达无法确认。",
            Self::Send => "消息发送失败，本次未发送。",
            Self::Partial { .. } => "部分消息已发送，后续发送失败。",
        }
    }

    pub fn partial_sent(&self) -> &[SentChatMessage] {
        match self {
            Self::Partial { sent } => sent,
            _ => &[],
        }
    }
}

/// Gateway 提供的会话发言能力。
///
/// 返回 `Ok` 只代表消息真实落地；任何失败都必须返回错误，不能由实现方
/// 生成成功文案。
#[async_trait]
pub trait ChatSendSink: Send + Sync {
    async fn send_messages(&self, intent: ChatSendIntent) -> Result<ChatSendResult, ChatSendError>;
}

pub type DynChatSendSink = Arc<dyn ChatSendSink>;

/// 单轮发言的投递账本。
///
/// 账本同时承担三个职责：限制单轮消息条数、给 `send_message` 去重、记录真实
/// 发送结果供最终文本抑制判断。账本只描述本轮，不持久化。
pub struct TurnDelivery {
    inner: Mutex<DeliveryInner>,
    accepting: AtomicBool,
}

#[derive(Debug)]
struct DeliveryInner {
    sent: Vec<SentChatMessage>,
    failures: usize,
    dedupe: HashSet<String>,
    latest_reply_to_message_id: Option<String>,
    max_messages: usize,
}

impl TurnDelivery {
    /// `max_messages` 是本轮最多允许发送的消息条数，必须为正。
    pub fn new(max_messages: usize, latest_reply_to_message_id: Option<String>) -> Self {
        Self {
            inner: Mutex::new(DeliveryInner {
                sent: Vec::new(),
                failures: 0,
                dedupe: HashSet::new(),
                latest_reply_to_message_id,
                max_messages: max_messages.max(1),
            }),
            accepting: AtomicBool::new(true),
        }
    }

    /// 判断同一批发言是否已经成功登记过去重键。
    pub fn is_duplicate(&self, key: &str) -> bool {
        self.lock().dedupe.contains(key)
    }

    /// 在真实发送成功后登记去重键。
    ///
    /// 只有确认已送达（含部分送达）才登记：发送完全失败时不能留下去重键，
    /// 否则模型按原内容重试会被谎报为"已经发送过"。
    pub fn mark_delivered(&self, key: &str) {
        self.lock().dedupe.insert(key.to_owned());
    }

    pub fn record_sent(&self, messages: Vec<SentChatMessage>) {
        let mut inner = self.lock();
        inner.sent.extend(messages);
    }

    pub fn record_failure(&self) {
        self.lock().failures += 1;
    }

    pub fn sent_count(&self) -> usize {
        self.lock().sent.len()
    }

    pub fn failure_count(&self) -> usize {
        self.lock().failures
    }

    pub fn sent_texts(&self) -> Vec<String> {
        self.lock()
            .sent
            .iter()
            .map(|message| message.text.clone())
            .collect()
    }

    pub fn remaining(&self) -> usize {
        let inner = self.lock();
        inner.max_messages.saturating_sub(inner.sent.len())
    }

    /// 插话到达时把被动回复锚点更新为最新入站消息。
    pub fn update_latest_reply_to(&self, message_id: Option<&str>) {
        if let Some(message_id) = message_id.map(str::trim).filter(|value| !value.is_empty()) {
            self.lock().latest_reply_to_message_id = Some(message_id.to_owned());
        }
    }

    pub fn latest_reply_to_message_id(&self) -> Option<String> {
        self.lock().latest_reply_to_message_id.clone()
    }

    /// 回合进入收尾后拒绝新的插话。
    pub fn close(&self) {
        self.accepting.store(false, Ordering::SeqCst);
    }

    pub fn is_accepting(&self) -> bool {
        self.accepting.load(Ordering::SeqCst)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, DeliveryInner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_ledger_counts_and_dedups() {
        let ledger = TurnDelivery::new(3, Some("m1".to_owned()));
        assert!(!ledger.is_duplicate("k1"));
        ledger.mark_delivered("k1");
        assert!(ledger.is_duplicate("k1"));
        ledger.record_sent(vec![SentChatMessage {
            text: "第一句".to_owned(),
            message_id: None,
        }]);
        assert_eq!(ledger.sent_count(), 1);
        assert_eq!(ledger.remaining(), 2);
        ledger.update_latest_reply_to(Some("m2"));
        assert_eq!(ledger.latest_reply_to_message_id().as_deref(), Some("m2"));
    }

    #[test]
    fn closing_turn_rejects_interjections() {
        let ledger = TurnDelivery::new(3, None);
        assert!(ledger.is_accepting());
        ledger.close();
        assert!(!ledger.is_accepting());
    }
}
