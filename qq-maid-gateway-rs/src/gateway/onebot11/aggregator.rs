//! OneBot 11 私聊短窗口消息聚合。
//!
//! 聚合发生在 `OneBotScopeDispatcher` 之前：等待用户停止输入时不占用 scope worker。
//! 命令、pending 和媒体消息仍通过 Core 的轻量分类接口判定后立即分发；群聊不聚合。

use std::{collections::HashMap, sync::Arc, time::Duration};

use qq_maid_core::service::CoreInboundKind;
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::{Instant, sleep_until, timeout},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, trace, warn};

use super::{
    OneBotSendError, OneBotSender,
    scope_dispatcher::{OneBotEnqueueError, OneBotEnqueueOutcome, OneBotScopeDispatcher},
};
use crate::{
    config::MessageAggregationConfig,
    gateway::{
        dedupe::{MessageDedupe, MessageReservation},
        logging::mask_scope_key,
        ping::is_ping_command,
        platform::{self, ConversationTarget, InboundMessage},
    },
    respond::RespondClient,
};

const AGGREGATION_FAILURE_TEXT: &str = "当前服务暂时不可用，请稍后再试。";
const AGGREGATOR_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// 聚合器只依赖 `enqueue` 这一条投递边界，便于测试直接观察批次和失败路径。
pub(super) trait AggregationSink: Send + Sync {
    fn enqueue_batch(
        &self,
        inbound: InboundMessage,
        reservations: Option<Vec<MessageReservation>>,
    ) -> Result<OneBotEnqueueOutcome, OneBotEnqueueError>;
}

impl AggregationSink for OneBotScopeDispatcher {
    fn enqueue_batch(
        &self,
        inbound: InboundMessage,
        reservations: Option<Vec<MessageReservation>>,
    ) -> Result<OneBotEnqueueOutcome, OneBotEnqueueError> {
        self.enqueue_reserved(inbound, reservations)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlushReason {
    QuietTimeout,
    MaxWait,
    MaxMessages,
    MaxChars,
    Immediate,
    Shutdown,
}

impl FlushReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::QuietTimeout => "quiet_timeout",
            Self::MaxWait => "max_wait",
            Self::MaxMessages => "max_messages",
            Self::MaxChars => "max_chars",
            Self::Immediate => "immediate",
            Self::Shutdown => "shutdown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AggregationKey {
    account_id: String,
    user_id: String,
}

struct PendingBatch {
    generation: u64,
    received_at: Instant,
    quiet_deadline: Instant,
    hard_deadline: Option<Instant>,
    messages: Vec<InboundMessage>,
    reservations: Vec<MessageReservation>,
    total_chars: usize,
}

enum AggregatorCommand {
    Enqueue {
        inbound: Box<InboundMessage>,
        reservation: Option<MessageReservation>,
    },
    Timer {
        key: AggregationKey,
        generation: u64,
    },
    Shutdown {
        ack: oneshot::Sender<()>,
    },
}

/// 非阻塞入队句柄；读循环只做去重和 `try_send`，不会等待 Core 分类或 scope worker。
#[derive(Clone)]
pub(super) struct OneBotMessageAggregatorHandle {
    command_tx: mpsc::Sender<AggregatorCommand>,
    dispatcher: Arc<dyn AggregationSink>,
    dedupe: Arc<MessageDedupe>,
    shutdown: CancellationToken,
}

impl OneBotMessageAggregatorHandle {
    pub(super) fn enqueue(
        &self,
        inbound: InboundMessage,
    ) -> Result<OneBotEnqueueOutcome, OneBotEnqueueError> {
        if self.shutdown.is_cancelled() {
            return Err(OneBotEnqueueError::Shutdown);
        }
        let reservation = match inbound.dedupe_message_key() {
            Some(key) => match self.dedupe.reserve_many([key], std::time::Instant::now()) {
                Ok(reservation) => Some(reservation),
                Err(_) => {
                    trace!(
                        scope_key = %self.masked_scope(&inbound),
                        "重复的 OneBot 11 消息在聚合前被忽略"
                    );
                    return Ok(OneBotEnqueueOutcome::Duplicate);
                }
            },
            None => None,
        };
        match self.command_tx.try_send(AggregatorCommand::Enqueue {
            inbound: Box::new(inbound),
            reservation,
        }) {
            Ok(()) => Ok(OneBotEnqueueOutcome::Accepted),
            Err(mpsc::error::TrySendError::Closed(command)) => {
                drop(command);
                Err(OneBotEnqueueError::Shutdown)
            }
            Err(mpsc::error::TrySendError::Full(AggregatorCommand::Enqueue {
                inbound,
                reservation,
            })) => {
                // 聚合器积压时不能丢用户消息：退回原有的立即分发路径，仅放弃本次防抖。
                warn!(
                    scope_key = %self.masked_scope(&inbound),
                    "OneBot 11 聚合队列已满，改走立即分发"
                );
                self.dispatcher
                    .enqueue_batch(*inbound, Some(reservation.into_iter().collect()))
            }
            Err(mpsc::error::TrySendError::Full(_)) => Err(OneBotEnqueueError::WorkerUnavailable),
        }
    }

    fn masked_scope(&self, inbound: &InboundMessage) -> String {
        platform::core_scope_key(inbound)
            .map(|scope| mask_scope_key(&scope))
            .unwrap_or_else(|_| "invalid_scope".to_owned())
    }

    fn shutdown(&self) -> oneshot::Receiver<()> {
        let (ack, reply) = oneshot::channel();
        let _ = self
            .command_tx
            .try_send(AggregatorCommand::Shutdown { ack });
        reply
    }
}

pub(super) struct OneBotMessageAggregator {
    handle: OneBotMessageAggregatorHandle,
    join_handle: JoinHandle<()>,
    shutdown: CancellationToken,
}

impl OneBotMessageAggregator {
    pub(super) fn new(
        config: MessageAggregationConfig,
        respond: RespondClient,
        dispatcher: Arc<dyn AggregationSink>,
        dedupe: Arc<MessageDedupe>,
        sender: OneBotSender,
        shutdown: CancellationToken,
    ) -> Self {
        let capacity = config.max_active_keys.saturating_mul(2).max(8);
        let (command_tx, command_rx) = mpsc::channel(capacity);
        let actor = AggregatorActor {
            config,
            respond,
            dispatcher: dispatcher.clone(),
            sender,
            command_rx,
            command_tx: command_tx.clone(),
            batches: HashMap::new(),
            shutdown: shutdown.clone(),
            shutting_down: false,
        };
        let handle = OneBotMessageAggregatorHandle {
            command_tx,
            dispatcher,
            dedupe,
            shutdown: shutdown.clone(),
        };
        Self {
            handle,
            join_handle: tokio::spawn(actor.run()),
            shutdown,
        }
    }

    pub(super) fn handle(&self) -> OneBotMessageAggregatorHandle {
        self.handle.clone()
    }

    pub(super) async fn shutdown(mut self) {
        let reply = self.handle.shutdown();
        if timeout(AGGREGATOR_SHUTDOWN_TIMEOUT, reply).await.is_err() {
            warn!("OneBot 11 消息聚合器关闭超时，正在强制停止");
            self.shutdown.cancel();
        }
        let _ = timeout(AGGREGATOR_SHUTDOWN_TIMEOUT, &mut self.join_handle).await;
    }
}

impl Drop for OneBotMessageAggregator {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

struct AggregatorActor {
    config: MessageAggregationConfig,
    respond: RespondClient,
    dispatcher: Arc<dyn AggregationSink>,
    sender: OneBotSender,
    command_rx: mpsc::Receiver<AggregatorCommand>,
    command_tx: mpsc::Sender<AggregatorCommand>,
    batches: HashMap<AggregationKey, PendingBatch>,
    shutdown: CancellationToken,
    shutting_down: bool,
}

impl AggregatorActor {
    async fn run(mut self) {
        loop {
            tokio::select! {
                _ = self.shutdown.cancelled() => break,
                command = self.command_rx.recv() => {
                    let Some(command) = command else {
                        break;
                    };
                    if self.handle_command(command).await {
                        return;
                    }
                }
            }
        }
        self.shutting_down = true;
        self.flush_all(FlushReason::Shutdown).await;
    }

    async fn handle_command(&mut self, command: AggregatorCommand) -> bool {
        match command {
            AggregatorCommand::Enqueue {
                inbound,
                reservation,
            } => {
                self.handle_inbound(*inbound, reservation).await;
                false
            }
            AggregatorCommand::Timer { key, generation } => {
                self.handle_timer(key, generation).await;
                false
            }
            AggregatorCommand::Shutdown { ack } => {
                self.shutting_down = true;
                self.command_rx.close();
                while let Ok(command) = self.command_rx.try_recv() {
                    if let AggregatorCommand::Enqueue {
                        inbound,
                        reservation,
                    } = command
                    {
                        self.handle_inbound(*inbound, reservation).await;
                    }
                }
                self.flush_all(FlushReason::Shutdown).await;
                let _ = ack.send(());
                true
            }
        }
    }

    async fn handle_inbound(
        &mut self,
        inbound: InboundMessage,
        reservation: Option<MessageReservation>,
    ) {
        let key = self.key_for(&inbound);
        if !self.should_aggregate(&inbound).await {
            self.flush_key(&key, FlushReason::Immediate).await;
            self.dispatch_single(inbound, reservation).await;
            return;
        }

        let chars = inbound.text.chars().count();
        if chars > self.config.max_chars {
            self.flush_key(&key, FlushReason::Immediate).await;
            self.dispatch_single(inbound, reservation).await;
            return;
        }

        if !self.batches.contains_key(&key) && self.batches.len() >= self.config.max_active_keys {
            warn!(
                scope_key = %mask_scope_key(&format!("private:{}", key.user_id)),
                active_keys = self.batches.len(),
                max_active_keys = self.config.max_active_keys,
                "OneBot 11 聚合活动键达到上限，将立即分发"
            );
            self.dispatch_single(inbound, reservation).await;
            return;
        }

        if let Some(batch) = self.batches.get(&key) {
            let projected_count = batch.messages.len() + 1;
            let projected_chars = batch.total_chars + chars;
            if projected_count > self.config.max_messages || projected_chars > self.config.max_chars
            {
                let reason = if projected_count > self.config.max_messages {
                    FlushReason::MaxMessages
                } else {
                    FlushReason::MaxChars
                };
                self.flush_key(&key, reason).await;
            }
        }

        let now = Instant::now();
        let mut flush_reason = None;
        let generation = if let Some(batch) = self.batches.get_mut(&key) {
            append_to_batch(batch, inbound, reservation, chars, now, &self.config);
            if batch.messages.len() >= self.config.max_messages {
                flush_reason = Some(FlushReason::MaxMessages);
            } else if batch.total_chars >= self.config.max_chars {
                flush_reason = Some(FlushReason::MaxChars);
            }
            batch.generation
        } else {
            let mut batch = PendingBatch {
                generation: 1,
                received_at: now,
                quiet_deadline: now + self.config.quiet,
                hard_deadline: (!self.config.max_wait.is_zero())
                    .then(|| now + self.config.max_wait),
                messages: Vec::new(),
                reservations: Vec::new(),
                total_chars: 0,
            };
            append_to_batch(&mut batch, inbound, reservation, chars, now, &self.config);
            if batch.messages.len() >= self.config.max_messages {
                flush_reason = Some(FlushReason::MaxMessages);
            } else if batch.total_chars >= self.config.max_chars {
                flush_reason = Some(FlushReason::MaxChars);
            }
            let generation = batch.generation;
            self.batches.insert(key.clone(), batch);
            generation
        };

        if let Some(reason) = flush_reason {
            self.flush_key(&key, reason).await;
        } else {
            self.spawn_timer(key, generation);
        }
    }

    async fn should_aggregate(&self, inbound: &InboundMessage) -> bool {
        if !self.config.private_enabled
            || !matches!(inbound.conversation, ConversationTarget::Private { .. })
            || inbound.text.trim().is_empty()
            || inbound.quoted.is_some()
            || is_ping_command(&inbound.text)
            || !inbound.attachments.is_empty()
            || inbound
                .input_parts
                .iter()
                .any(qq_maid_common::input_part::MessageInputPart::is_non_text)
        {
            return false;
        }
        let content = platform::render_text_for_core(inbound);
        match self.respond.classify_inbound(inbound, content).await {
            Ok(classification) => classification.kind == CoreInboundKind::NormalChat,
            Err(error) => {
                warn!(
                    scope_key = %mask_scope_key(
                        &platform::core_scope_key(inbound)
                            .unwrap_or_else(|_| "invalid_scope".to_owned())
                    ),
                    message_id = %inbound.message_id,
                    error = %error.log_summary(),
                    "OneBot 11 消息聚合分类失败，将立即分发"
                );
                false
            }
        }
    }

    async fn dispatch_single(
        &self,
        inbound: InboundMessage,
        reservation: Option<MessageReservation>,
    ) {
        let scope_key = platform::core_scope_key(&inbound)
            .map(|scope| mask_scope_key(&scope))
            .unwrap_or_else(|_| "invalid_scope".to_owned());
        match self
            .dispatcher
            .enqueue_batch(inbound.clone(), Some(reservation.into_iter().collect()))
        {
            Ok(OneBotEnqueueOutcome::Accepted) | Ok(OneBotEnqueueOutcome::Duplicate) => {}
            Err(error) => {
                warn!(
                    scope_key = %scope_key,
                    error = %error,
                    "OneBot 11 聚合立即分发失败"
                );
                self.notify_failure(&inbound).await;
            }
        }
    }

    async fn handle_timer(&mut self, key: AggregationKey, generation: u64) {
        let Some(batch) = self.batches.get(&key) else {
            return;
        };
        if batch.generation != generation {
            return;
        }
        let now = Instant::now();
        let reason = if batch.hard_deadline.is_some_and(|deadline| now >= deadline) {
            FlushReason::MaxWait
        } else if now >= batch.quiet_deadline {
            FlushReason::QuietTimeout
        } else {
            self.spawn_timer(key, generation);
            return;
        };
        self.flush_key(&key, reason).await;
    }

    async fn flush_key(&mut self, key: &AggregationKey, reason: FlushReason) {
        let Some(batch) = self.batches.remove(key) else {
            return;
        };
        let batch_size = batch.messages.len();
        let message = merge_batch(&batch, reason);
        let scope_key = platform::core_scope_key(&message)
            .map(|scope| mask_scope_key(&scope))
            .unwrap_or_else(|_| "invalid_scope".to_owned());
        let result = self
            .dispatcher
            .enqueue_batch(message.clone(), Some(batch.reservations));
        match result {
            Ok(OneBotEnqueueOutcome::Accepted) | Ok(OneBotEnqueueOutcome::Duplicate) => {
                debug!(
                    scope_key = %scope_key,
                    aggregation_batch_size = batch_size,
                    aggregation_wait_ms = batch.received_at.elapsed().as_millis() as u64,
                    aggregation_flush_reason = reason.as_str(),
                    "OneBot 11 消息聚合批次已刷新"
                );
            }
            Err(error) => {
                warn!(
                    scope_key = %scope_key,
                    error = %error,
                    stage = reason.as_str(),
                    aggregation_batch_size = batch_size,
                    "OneBot 11 消息聚合刷新失败"
                );
                if !self.shutting_down {
                    self.notify_failure(&message).await;
                }
            }
        }
    }

    async fn flush_all(&mut self, reason: FlushReason) {
        let keys = self.batches.keys().cloned().collect::<Vec<_>>();
        for key in keys {
            self.flush_key(&key, reason).await;
        }
    }

    fn spawn_timer(&self, key: AggregationKey, generation: u64) {
        let Some(batch) = self.batches.get(&key) else {
            return;
        };
        let deadline = match batch.hard_deadline {
            Some(hard_deadline) => batch.quiet_deadline.min(hard_deadline),
            None => batch.quiet_deadline,
        };
        let command_tx = self.command_tx.clone();
        let shutdown = self.shutdown.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = shutdown.cancelled() => {}
                _ = sleep_until(deadline) => {
                    let _ = command_tx
                        .send(AggregatorCommand::Timer { key, generation })
                        .await;
                }
            }
        });
    }

    async fn notify_failure(&self, inbound: &InboundMessage) {
        let ConversationTarget::Private { target_id } = &inbound.conversation else {
            return;
        };
        match self
            .sender
            .send_private_text(target_id, AGGREGATION_FAILURE_TEXT)
            .await
        {
            Ok(_) => {}
            Err(OneBotSendError::Transport(error)) => {
                debug!(error = %error, "OneBot 11 聚合失败提示发送失败");
            }
            Err(error) => {
                debug!(error = %error, "OneBot 11 聚合失败提示未发送");
            }
        }
    }

    fn key_for(&self, inbound: &InboundMessage) -> AggregationKey {
        AggregationKey {
            account_id: inbound
                .account_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("-")
                .to_owned(),
            user_id: inbound.conversation.target_id().to_owned(),
        }
    }
}

fn append_to_batch(
    batch: &mut PendingBatch,
    message: InboundMessage,
    reservation: Option<MessageReservation>,
    chars: usize,
    now: Instant,
    config: &MessageAggregationConfig,
) {
    batch.messages.push(message);
    if let Some(reservation) = reservation {
        batch.reservations.push(reservation);
    }
    batch.total_chars += chars;
    batch.quiet_deadline = match batch.hard_deadline {
        Some(hard_deadline) => (now + config.quiet).min(hard_deadline),
        None => now + config.quiet,
    };
    batch.generation = batch.generation.saturating_add(1);
}

fn merge_batch(batch: &PendingBatch, reason: FlushReason) -> InboundMessage {
    let all_messages = &batch.messages;
    let mut merged = all_messages
        .last()
        .cloned()
        .expect("aggregation batch should never be empty when flushed");
    merged.text = all_messages
        .iter()
        .map(|message| message.text.as_str())
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    merged.input_parts = all_messages
        .iter()
        .flat_map(|message| message.input_parts.clone())
        .collect();
    merged.attachments = all_messages
        .iter()
        .flat_map(|message| message.attachments.clone())
        .collect();
    merged.mentions = all_messages
        .iter()
        .flat_map(|message| message.mentions.clone())
        .collect();
    merged.mentioned_bot = all_messages.iter().any(|message| message.mentioned_bot);
    merged.quoted = all_messages
        .iter()
        .find_map(|message| message.quoted.clone());
    merged.timestamp = all_messages
        .last()
        .and_then(|message| message.timestamp.clone());
    trace!(
        aggregation_batch_size = all_messages.len(),
        aggregation_total_chars = batch.total_chars,
        aggregation_flush_reason = reason.as_str(),
        "OneBot 11 消息聚合批次已合并"
    );
    merged
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };

    use async_trait::async_trait;
    use qq_maid_common::{
        identity_context::IdentitySource, input_part::MessageInputPart,
        output_part::AssistantOutput,
    };
    use qq_maid_core::service::{
        CoreError, CoreHealthSnapshot, CoreInboundKind, CoreRequest, CoreRespondOutput,
        CoreResponse, CoreService, UpstreamStatusSnapshot,
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
            let immediate =
                text.starts_with('/') || text.contains("立即") || request.quoted.is_some();
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
}
