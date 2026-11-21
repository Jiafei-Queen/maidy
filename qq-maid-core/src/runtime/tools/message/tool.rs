//! 模型可调用的会话发言工具。
//!
//! 工具只负责参数清洗、条数预算、请求级目标绑定和真实发送结果投影；平台
//! 分段与真实发送全部交给注入的 [`ChatSendSink`]。模型正文不会自动外发，
//! 因此本工具是 Agent 主动发言的唯一入口。

use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    time::Duration,
};

use qq_maid_common::identity_context::ConversationKind;
use qq_maid_llm::tool::{
    Tool, ToolContext, ToolEffect, ToolMetadata, ToolOutput, ToolTimeoutPolicy,
};
use serde_json::{Value, json};
use tokio::time::{Instant, timeout_at};
use tracing::warn;

use crate::{
    error::LlmError,
    runtime::{
        push::{ONEBOT11_PLATFORM, PushTarget, PushTargetType, QQ_OFFICIAL_PLATFORM},
        respond::RespondRequest,
    },
};

use super::delivery::{
    ChatSendError, ChatSendIntent, ChatSendProgress, DynChatSendSink, TurnDelivery,
};

pub const SEND_MESSAGE_TOOL_NAME: &str = "send_message";

/// 单次 Tool 调用最多允许的文本条数；更细的轮次预算由平台决定。
const MAX_MESSAGES_PER_CALL: usize = 4;
/// 单条文本上限；Gateway 仍会按平台限制安全分段。
const MAX_MESSAGE_CHARS: usize = 1200;
/// 工具自身兜底上限；正常路径还会受 Agent 请求截止时间约束。
const SEND_MESSAGE_TIMEOUT: Duration = Duration::from_secs(30);

/// 微信服务号在 Core 中的稳定平台名。
const WECHAT_SERVICE_PLATFORM: &str = "wechat_service";

/// 相邻两条消息之间的拟人停顿范围。
const DEFAULT_GAP: Duration = Duration::from_millis(900);

#[derive(Clone)]
pub struct SendMessageTool {
    sink: DynChatSendSink,
    delivery: Option<std::sync::Arc<TurnDelivery>>,
    target: Option<ChatSendTarget>,
}

#[derive(Debug, Clone)]
struct ChatSendTarget {
    target: PushTarget,
    reply_to_message_id: Option<String>,
    max_messages: usize,
}

impl SendMessageTool {
    /// 构造基础工具；实际执行前必须由请求级 scoped 实例替换。
    pub fn new(sink: DynChatSendSink) -> Self {
        Self {
            sink,
            delivery: None,
            target: None,
        }
    }

    /// 当前请求是否可以绑定会话发送目标。
    pub(crate) fn supports_request(request: &RespondRequest) -> bool {
        target_for_request(request).is_some()
    }

    /// 当前请求允许的单轮消息总条数；平台不支持时返回 0。
    pub(crate) fn max_messages_for_request(request: &RespondRequest) -> usize {
        target_for_request(request)
            .map(|target| target.max_messages)
            .unwrap_or(0)
    }

    /// 绑定当前请求的真实投递目标，并挂上本轮投递账本。
    ///
    /// 目标只来自服务端权威字段，模型参数不参与平台、账号或会话目标选择。
    pub(crate) fn scoped_for_request(
        &self,
        request: &RespondRequest,
        delivery: std::sync::Arc<TurnDelivery>,
    ) -> Option<Self> {
        let target = target_for_request(request)?;
        Some(Self {
            sink: self.sink.clone(),
            delivery: Some(delivery),
            target: Some(target),
        })
    }
}

#[async_trait::async_trait]
impl Tool for SendMessageTool {
    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: SEND_MESSAGE_TOOL_NAME.to_owned(),
            description: format!(
                "向当前会话发送一条或多条消息。最终文字回复不会自动发送给用户，需要让用户看到内容时必须调用本工具。任务耗时较长时，先发送一句简短进度说明，再继续调用其他工具；任务完成后再发送结果。一次可以传多条消息，平台会按顺序发送；每条最多 {MAX_MESSAGE_CHARS} 个字符。不要用本工具重复已经发送过的内容。发送是否成功以本工具返回的 ok 为准；ok 不是 true 时不得声称已经发送。"
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "messages": {
                        "type": "array",
                        "items": {"type": "string"},
                        "minItems": 1,
                        "maxItems": MAX_MESSAGES_PER_CALL,
                        "description": "要依次发送给用户的纯文本消息；可以是多条短句，也可以是一条完整回复。"
                    }
                },
                "required": ["messages"],
                "additionalProperties": false
            }),
        }
    }

    fn timeout_policy(&self) -> ToolTimeoutPolicy {
        ToolTimeoutPolicy::ToolManaged
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::SideEffecting
    }

    async fn execute(
        &self,
        context: ToolContext,
        arguments: Value,
    ) -> Result<ToolOutput, LlmError> {
        let messages = match clean_messages(&arguments) {
            Ok(messages) => messages,
            Err(()) => {
                return Ok(failure_output(
                    "invalid_messages",
                    "消息内容为空、过长或条数超限，本次未发送。",
                ));
            }
        };
        let Some(target) = self.target.as_ref() else {
            return Ok(failure_output(
                ChatSendError::TargetMissing.code(),
                ChatSendError::TargetMissing.user_message(),
            ));
        };
        let Some(delivery) = self.delivery.as_ref() else {
            return Ok(failure_output(
                ChatSendError::Unavailable.code(),
                ChatSendError::Unavailable.user_message(),
            ));
        };
        let remaining = delivery.remaining();
        if messages.len() > remaining {
            return Ok(failure_output(
                "budget_exceeded",
                &format!("本轮最多还能发送 {remaining} 条消息，本次未发送。"),
            ));
        }
        let dedupe_key = dedupe_key(target, &messages);
        if delivery.is_duplicate(&dedupe_key) {
            return Ok(ToolOutput::json(json!({
                "ok": true,
                "duplicate": true,
                "message": "同一批消息已经发送过，本次未重复发送。"
            })));
        }

        // 超时/取消会丢弃 sink future；用增量回调即时登记真实送达结果，
        // 即使 execute 中途被取消，账本也不会少计。
        let sent_before = delivery.sent_count();
        let progress_delivery = delivery.clone();
        let intent = ChatSendIntent {
            target: target.target.clone(),
            reply_to_message_id: delivery
                .latest_reply_to_message_id()
                .or_else(|| target.reply_to_message_id.clone()),
            messages: messages.clone(),
            gap: Some(DEFAULT_GAP),
            on_sent: Some(ChatSendProgress::new(move |message| {
                progress_delivery.record_sent(vec![message]);
            })),
        };
        let deadline = context
            .execution_deadline
            .unwrap_or_else(|| Instant::now() + SEND_MESSAGE_TIMEOUT);
        let result = match timeout_at(deadline, self.sink.send_messages(intent)).await {
            Ok(result) => result,
            Err(_) => Err(ChatSendError::Timeout),
        };
        match result {
            Ok(result) => {
                // 未实现进度回调的 sink 在这里补齐；已回调的部分不重复登记。
                let notified = delivery.sent_count().saturating_sub(sent_before);
                if notified < result.sent.len() {
                    delivery.record_sent(result.sent[notified..].to_vec());
                }
                let sent_count = delivery.sent_count().saturating_sub(sent_before);
                if sent_count > 0 {
                    delivery.mark_delivered(&dedupe_key);
                }
                Ok(ToolOutput::json(json!({
                    "ok": true,
                    "sent_count": sent_count,
                    "remaining": delivery.remaining(),
                    "message": "消息已发送。"
                })))
            }
            Err(error) => {
                // 部分送达优先取错误携带的结果；超时/取消时使用进度回调的登记。
                let partial = error.partial_sent();
                let notified = delivery.sent_count().saturating_sub(sent_before);
                if notified < partial.len() {
                    delivery.record_sent(partial[notified..].to_vec());
                }
                let sent_count = delivery.sent_count().saturating_sub(sent_before);
                if sent_count > 0 {
                    // 已送达部分不能因失败被模型原样重发。
                    delivery.mark_delivered(&dedupe_key);
                }
                delivery.record_failure();
                warn!(
                    error_code = error.code(),
                    tool = SEND_MESSAGE_TOOL_NAME,
                    sent_count,
                    "发言工具发送失败"
                );
                Ok(ToolOutput::json(json!({
                    "ok": false,
                    "sent_count": sent_count,
                    "error_code": error.code(),
                    "message": error.user_message(),
                })))
            }
        }
    }
}

fn clean_messages(arguments: &Value) -> Result<Vec<String>, ()> {
    let raw = arguments
        .get("messages")
        .and_then(Value::as_array)
        .ok_or(())?;
    let mut messages = Vec::new();
    for item in raw {
        let text = item.as_str().ok_or(())?.trim();
        if text.is_empty() {
            return Err(());
        }
        if text.chars().count() > MAX_MESSAGE_CHARS {
            return Err(());
        }
        messages.push(text.to_owned());
        if messages.len() > MAX_MESSAGES_PER_CALL {
            return Err(());
        }
    }
    if messages.is_empty() {
        return Err(());
    }
    Ok(messages)
}

fn dedupe_key(target: &ChatSendTarget, messages: &[String]) -> String {
    let mut hasher = DefaultHasher::new();
    target.target.platform.hash(&mut hasher);
    target.target.account_id.hash(&mut hasher);
    target.target.target_type.as_str().hash(&mut hasher);
    target.target.target_id.hash(&mut hasher);
    for message in messages {
        message.hash(&mut hasher);
    }
    format!("send_message:{:x}", hasher.finish())
}

fn target_for_request(request: &RespondRequest) -> Option<ChatSendTarget> {
    let platform = request.platform.trim();
    let max_messages = match platform {
        QQ_OFFICIAL_PLATFORM => {
            if request.conversation_kind == ConversationKind::Group {
                // QQ 群被动回复预算 5，留给最终文本兜底 1 条。
                4
            } else {
                // QQ C2C 被动回复预算 4。
                3
            }
        }
        ONEBOT11_PLATFORM | "onebot" => 5,
        WECHAT_SERVICE_PLATFORM => 3,
        _ => return None,
    };
    let account_id = clean_optional(request.account_id.as_deref());
    let reply_to_message_id = clean_optional(request.message_id.as_deref()).map(str::to_owned);
    let (target_type, target_id) = match request.conversation_kind {
        ConversationKind::Private => (
            PushTargetType::Private,
            clean_optional(request.conversation_id.as_deref())?,
        ),
        // 微信服务号是单用户客服会话，平台无关投递类型仍按私聊处理。
        ConversationKind::ServiceAccount => (
            PushTargetType::Private,
            clean_optional(request.conversation_id.as_deref())?,
        ),
        ConversationKind::Group => {
            let conversation_id = clean_optional(request.conversation_id.as_deref());
            let group_id = clean_optional(request.group_id.as_deref());
            // 群聊目标只信任 Gateway 归一化的 group_id；两者同时存在但不一致时
            // 宁可关闭工具，也不能把消息发到错误会话。
            let target_id = match (conversation_id, group_id) {
                (Some(conversation), Some(group)) if conversation == group => group,
                (None, Some(group)) => group,
                _ => return None,
            };
            (PushTargetType::Group, target_id)
        }
        _ => return None,
    };
    // 微信服务号是私聊客服消息，主动推送路由尚未支持，但仍需要保留目标类型。
    Some(ChatSendTarget {
        target: PushTarget::new(
            platform,
            account_id.map(str::to_owned),
            target_type,
            target_id,
        ),
        reply_to_message_id,
        max_messages,
    })
}

fn clean_optional(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn failure_output(error_code: &str, message: &str) -> ToolOutput {
    ToolOutput::json(json!({
        "ok": false,
        "error_code": error_code,
        "message": message,
    }))
}
