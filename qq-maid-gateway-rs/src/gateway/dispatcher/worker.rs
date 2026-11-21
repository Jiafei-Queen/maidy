//! Dispatcher 会话 worker 执行循环与 idle 回收。
//!
//! `run_worker` 在独立 task 中串行消费某 scope 的队列消息，并在 idle 超时后向
//! actor 询问是否回收。同一 scope 的普通文本消息在 Agent 回合运行期间可以
//! steering 到当前回合；其余消息仍按原串行语义排队，不改变确定性命令、pending
//! 和媒体处理边界。

use std::{collections::VecDeque, sync::Arc, time::Duration};

use tokio::{
    sync::{mpsc, oneshot},
    time::timeout,
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use super::super::logging::mask_scope_key;
// `MessageHandler` 定义在 actor 子模块，actor 通过 `pub(super)` 暴露给 dispatcher
// 后代；worker 作为 dispatcher 的子模块可以直接引用。`HandlerFuture` 仅作为
// `handler.handle` 的返回类型被隐式使用，这里不需要具名导入。
use super::actor::MessageHandler;
use super::types::{DispatcherCommand, IdleDecision, QueuedMessage, WorkerExitReason};

/// 单个 scope worker 的执行上下文，由 actor 在 `spawn_worker` 时构造。
pub(super) struct WorkerContext {
    pub(super) scope_key: String,
    pub(super) generation: u64,
    pub(super) handler: Arc<dyn MessageHandler>,
    pub(super) command_tx: mpsc::Sender<DispatcherCommand>,
    pub(super) rx: mpsc::Receiver<QueuedMessage>,
    pub(super) idle_timeout: Duration,
    pub(super) shutdown_token: CancellationToken,
}

pub(super) async fn run_worker(mut ctx: WorkerContext) -> WorkerExitReason {
    // 已取出但未能 steering 的消息；必须回到队首，保持与入站顺序一致。
    let mut pending: VecDeque<QueuedMessage> = VecDeque::new();
    let mut rx_open = true;
    loop {
        // pending 由 handler 运行期间回流，不经过下面的 shutdown select；
        // 取消时必须在这里拦截，保持"取消后不再处理排队消息"的既有语义。
        if !pending.is_empty() && ctx.shutdown_token.is_cancelled() {
            let dropped_messages = ctx.rx.len() + pending.len();
            warn!(
                scope_key = %mask_scope_key(&ctx.scope_key),
                generation = ctx.generation,
                dropped_messages,
                "dispatcher worker 取消时仍有排队消息"
            );
            return WorkerExitReason::Cancelled;
        }
        let message = match pending.pop_front() {
            Some(message) => message,
            None => {
                if !rx_open {
                    return WorkerExitReason::Completed;
                }
                let next = tokio::select! {
                    _ = ctx.shutdown_token.cancelled() => {
                        let dropped_messages = ctx.rx.len() + pending.len();
                        if dropped_messages > 0 {
                            warn!(
                                scope_key = %mask_scope_key(&ctx.scope_key),
                                generation = ctx.generation,
                                dropped_messages,
                                "dispatcher worker 取消时仍有排队消息"
                            );
                        }
                        return WorkerExitReason::Cancelled;
                    }
                    result = timeout(ctx.idle_timeout, ctx.rx.recv()) => result,
                };
                match next {
                    Ok(Some(message)) => message,
                    Ok(None) => return WorkerExitReason::Completed,
                    Err(_) => {
                        let (reply_tx, reply_rx) = oneshot::channel();
                        if ctx
                            .command_tx
                            .send(DispatcherCommand::WorkerIdleExpired {
                                scope_key: ctx.scope_key.clone(),
                                generation: ctx.generation,
                                reply: reply_tx,
                            })
                            .await
                            .is_err()
                        {
                            return WorkerExitReason::Cancelled;
                        }
                        match reply_rx.await {
                            Ok(IdleDecision::StayActive) => continue,
                            Ok(IdleDecision::RetireNow) => return WorkerExitReason::Completed,
                            Err(_) => return WorkerExitReason::Cancelled,
                        }
                    }
                }
            }
        };
        if ctx
            .command_tx
            .send(DispatcherCommand::WorkerDequeued {
                scope_key: ctx.scope_key.clone(),
                generation: ctx.generation,
            })
            .await
            .is_err()
        {
            warn!(
                scope_key = %mask_scope_key(&ctx.scope_key),
                generation = ctx.generation,
                queued_messages = ctx.rx.len(),
                "dispatcher worker 取出消息时命令通道已关闭"
            );
            return WorkerExitReason::Cancelled;
        }
        let QueuedMessage {
            envelope,
            processed_ack,
            ..
        } = message;
        let result = handle_with_steering(
            &ctx.handler,
            &ctx.command_tx,
            &ctx.scope_key,
            ctx.generation,
            &mut ctx.rx,
            envelope,
            &mut pending,
            &mut rx_open,
        )
        .await;
        if let Some(ack) = processed_ack {
            let _ = ack.send(());
        }
        if let Err(error) = result {
            warn!(
                scope_key = %mask_scope_key(&ctx.scope_key),
                generation = ctx.generation,
                error = %error,
                "dispatcher worker 处理消息失败"
            );
        } else {
            debug!(
                scope_key = %mask_scope_key(&ctx.scope_key),
                generation = ctx.generation,
                "dispatcher worker 已处理消息"
            );
        }
    }
}

/// 等待 handler 完成期间继续读取队列，并尝试把纯文本消息注入当前回合。
///
/// 只有队首没有积压、且消息没有聚合边界 ack 时才允许 steering；其余消息按
/// 原顺序进入 `pending`，在本次 handler 完成后继续处理。
#[allow(clippy::too_many_arguments)]
async fn handle_with_steering(
    handler: &Arc<dyn MessageHandler>,
    command_tx: &mpsc::Sender<DispatcherCommand>,
    scope_key: &str,
    generation: u64,
    rx: &mut mpsc::Receiver<QueuedMessage>,
    envelope: super::types::InboundEnvelope,
    pending: &mut VecDeque<QueuedMessage>,
    rx_open: &mut bool,
) -> anyhow::Result<()> {
    let mut handle_future = Box::pin(handler.handle(envelope));
    // 命令通道关闭时不能中途 drop 正在执行的回合；先让 handler 完成再返回错误。
    let mut command_closed = false;
    loop {
        tokio::select! {
            result = &mut handle_future => {
                if command_closed {
                    return Err(anyhow::anyhow!("dispatcher command channel closed"));
                }
                return result;
            }
            received = rx.recv(), if *rx_open && pending.is_empty() && !command_closed => {
                match received {
                    Some(message) => {
                        let steerable = message.processed_ack.is_none();
                        if steerable && handler.steer(&message.envelope).await {
                            if command_tx
                                .send(DispatcherCommand::WorkerDequeued {
                                    scope_key: scope_key.to_owned(),
                                    generation,
                                })
                                .await
                                .is_err()
                            {
                                warn!(
                                    scope_key = %mask_scope_key(scope_key),
                                    generation,
                                    "dispatcher 命令通道已关闭，等待当前回合结束后退出"
                                );
                                command_closed = true;
                                continue;
                            }
                            debug!(
                                scope_key = %mask_scope_key(scope_key),
                                generation,
                                "dispatcher 已将消息 steering 到当前会话回合"
                            );
                        } else {
                            pending.push_front(message);
                        }
                    }
                    None => *rx_open = false,
                }
            }
        }
    }
}
