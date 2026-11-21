
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

use async_trait::async_trait;
use qq_maid_common::{
    identity_context::IdentitySource, input_part::MessageInputPart, output_part::AssistantOutput,
};
use qq_maid_core::service::{
    CoreError, CoreHealthSnapshot, CoreInboundKind, CoreRequest, CoreRespondOutput, CoreResponse,
    CoreService, UpstreamStatusSnapshot,
};
use tokio::time::{advance, pause};

use super::*;
use crate::gateway::platform::{
    Actor, ConversationTarget, InboundMessage, Platform as GatewayPlatform,
};

#[derive(Default)]
struct RecordingSink {
    messages: Mutex<Vec<InboundMessage>>,
    reservations: Mutex<Vec<usize>>,
    fail: AtomicBool,
}

impl RecordingSink {
    fn messages(&self) -> Vec<InboundMessage> {
        self.messages.lock().unwrap().clone()
    }
}

impl AggregationSink for RecordingSink {
    fn enqueue_batch(
        &self,
        inbound: InboundMessage,
        reservations: Option<Vec<MessageReservation>>,
    ) -> Result<OneBotEnqueueOutcome, OneBotEnqueueError> {
        if self.fail.load(Ordering::Relaxed) {
            return Err(OneBotEnqueueError::WorkerUnavailable);
        }
        self.reservations
            .lock()
            .unwrap()
            .push(reservations.map(|items| items.len()).unwrap_or(0));
        self.messages.lock().unwrap().push(inbound);
        Ok(OneBotEnqueueOutcome::Accepted)
    }
}

struct MockCore;

#[async_trait]
impl CoreService for MockCore {
    async fn respond(&self, _request: CoreRequest) -> Result<CoreRespondOutput, CoreError> {
        Ok(CoreRespondOutput::Complete(Box::new(CoreResponse {
            output: Some(AssistantOutput::text("ok")),
            handled: Some(true),
            session_id: None,
            command: None,
            diagnostics: None,
            visible_entity_snapshot: None,
        })))
    }

    async fn classify_inbound(
        &self,
        request: CoreRequest,
    ) -> Result<qq_maid_core::service::CoreInboundClassification, CoreError> {
        let text = request.text.trim();
        let immediate = text.starts_with('/') || text.contains("立即") || request.quoted.is_some();
        Ok(qq_maid_core::service::CoreInboundClassification {
            kind: if immediate {
                CoreInboundKind::Immediate
            } else {
                CoreInboundKind::NormalChat
            },
        })
    }

    async fn upstream_check(&self) -> Result<(), CoreError> {
        Ok(())
    }

    fn health_snapshot(&self) -> CoreHealthSnapshot {
        CoreHealthSnapshot {
            ok: true,
            provider: "mock".to_owned(),
            model: "mock".to_owned(),
            stream: false,
            upstream: UpstreamStatusSnapshot::default(),
        }
    }
}

struct Harness {
    aggregator: OneBotMessageAggregator,
    handle: OneBotMessageAggregatorHandle,
    sink: Arc<RecordingSink>,
}

fn config() -> MessageAggregationConfig {
    MessageAggregationConfig {
        private_enabled: true,
        group_enabled: false,
        quiet: Duration::from_millis(100),
        max_wait: Duration::from_millis(500),
        max_messages: 10,
        max_chars: 12000,
        max_active_keys: 64,
    }
}

fn harness(config: MessageAggregationConfig) -> Harness {
    let sink = Arc::new(RecordingSink::default());
    let dispatcher: Arc<dyn AggregationSink> = sink.clone();
    let sender = OneBotSender::new(super::super::OneBotConnectionContext::new(
        Duration::from_millis(10),
    ));
    let aggregator = OneBotMessageAggregator::new(
        config,
        RespondClient::new(Arc::new(MockCore)),
        dispatcher,
        Arc::new(MessageDedupe::new(Duration::from_secs(60))),
        sender,
        CancellationToken::new(),
    );
    let handle = aggregator.handle();
    Harness {
        aggregator,
        handle,
        sink,
    }
}

fn inbound(id: &str, text: &str) -> InboundMessage {
    InboundMessage {
        platform: GatewayPlatform::OneBot11,
        account_id: Some("bot".to_owned()),
        conversation: ConversationTarget::Private {
            target_id: "user".to_owned(),
        },
        actor: Actor {
            sender_id: Some("user".to_owned()),
            union_id: None,
            display_name: None,
            group_member_role: None,
            is_bot: false,
            source: IdentitySource::Event,
        },
        message_id: id.to_owned(),
        current_msg_idx: None,
        timestamp: Some(format!("2026-11-22T12:00:{id}+08:00")),
        text: text.to_owned(),
        input_parts: vec![MessageInputPart::text(text)],
        attachments: Vec::new(),
        quoted: None,
        visible_entity_snapshot: None,
        mentions: Vec::new(),
        mentioned_bot: false,
    }
}

async fn settle() {
    advance(Duration::ZERO).await;
    tokio::task::yield_now().await;
}

async fn enqueue(handle: &OneBotMessageAggregatorHandle, message: InboundMessage) {
    handle.enqueue(message).unwrap();
    settle().await;
}

#[tokio::test]
async fn consecutive_private_text_is_merged_after_quiet_window() {
    pause();
    let h = harness(config());
    enqueue(&h.handle, inbound("1", "第一句")).await;
    enqueue(&h.handle, inbound("2", "第二句")).await;
    advance(Duration::from_millis(101)).await;
    settle().await;
    let messages = h.sink.messages();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].text, "第一句\n第二句");
    assert_eq!(messages[0].input_parts.len(), 2);
    h.aggregator.shutdown().await;
}

#[tokio::test]
async fn new_message_resets_quiet_window() {
    pause();
    let h = harness(config());
    enqueue(&h.handle, inbound("1", "a")).await;
    advance(Duration::from_millis(80)).await;
    enqueue(&h.handle, inbound("2", "b")).await;
    advance(Duration::from_millis(80)).await;
    settle().await;
    assert!(h.sink.messages().is_empty());
    advance(Duration::from_millis(21)).await;
    settle().await;
    assert_eq!(h.sink.messages().len(), 1);
    h.aggregator.shutdown().await;
}

#[tokio::test]
async fn immediate_message_flushes_pending_batch_first() {
    pause();
    let h = harness(config());
    enqueue(&h.handle, inbound("1", "普通消息")).await;
    enqueue(&h.handle, inbound("2", "/todo 立即")).await;
    settle().await;
    let messages = h.sink.messages();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].text, "普通消息");
    assert_eq!(messages[1].text, "/todo 立即");
    h.aggregator.shutdown().await;
}

#[tokio::test]
async fn media_message_is_not_aggregated() {
    pause();
    let h = harness(config());
    let mut media = inbound("1", "");
    media.input_parts = vec![MessageInputPart::image(Default::default())];
    enqueue(&h.handle, media).await;
    enqueue(&h.handle, inbound("2", "文字")).await;
    settle().await;
    let messages = h.sink.messages();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].message_id, "1");
    h.aggregator.shutdown().await;
}

#[tokio::test]
async fn zero_max_wait_keeps_batch_open() {
    pause();
    let mut cfg = config();
    cfg.max_wait = Duration::ZERO;
    let h = harness(cfg);
    enqueue(&h.handle, inbound("1", "a")).await;
    for id in ["2", "3", "4"] {
        advance(Duration::from_millis(90)).await;
        enqueue(&h.handle, inbound(id, "b")).await;
    }
    advance(Duration::from_millis(101)).await;
    settle().await;
    assert_eq!(h.sink.messages().len(), 1);
    assert_eq!(h.sink.messages()[0].text, "a\nb\nb\nb");
    h.aggregator.shutdown().await;
}

#[tokio::test]
async fn max_messages_flushes_immediately() {
    pause();
    let mut cfg = config();
    cfg.max_messages = 2;
    let h = harness(cfg);
    enqueue(&h.handle, inbound("1", "a")).await;
    enqueue(&h.handle, inbound("2", "b")).await;
    settle().await;
    assert_eq!(h.sink.messages().len(), 1);
    assert_eq!(h.sink.messages()[0].text, "a\nb");
    h.aggregator.shutdown().await;
}

#[tokio::test]
async fn duplicate_physical_message_is_ignored() {
    pause();
    let h = harness(config());
    enqueue(&h.handle, inbound("1", "a")).await;
    assert_eq!(
        h.handle.enqueue(inbound("1", "a")),
        Ok(OneBotEnqueueOutcome::Duplicate)
    );
    advance(Duration::from_millis(101)).await;
    settle().await;
    assert_eq!(h.sink.messages().len(), 1);
    h.aggregator.shutdown().await;
}

#[tokio::test]
async fn shutdown_flushes_pending_batch() {
    pause();
    let h = harness(config());
    enqueue(&h.handle, inbound("1", "a")).await;
    h.aggregator.shutdown().await;
    assert_eq!(h.sink.messages().len(), 1);
}

#[tokio::test]
async fn flush_failure_notifies_without_panicking() {
    pause();
    let h = harness(config());
    h.sink.fail.store(true, Ordering::Relaxed);
    enqueue(&h.handle, inbound("1", "a")).await;
    advance(Duration::from_millis(101)).await;
    settle().await;
    assert!(h.sink.messages().is_empty());
    h.aggregator.shutdown().await;
}
