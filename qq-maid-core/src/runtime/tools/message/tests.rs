use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use qq_maid_common::identity_context::{
    ConversationKind, ExecutionActorContext, ExecutionConversationContext,
};
use qq_maid_llm::tool::{Tool, ToolContext};
use serde_json::json;

use super::{
    ChatSendError, ChatSendIntent, ChatSendResult, ChatSendSink, SEND_MESSAGE_TOOL_NAME,
    SendMessageTool, SentChatMessage, TurnDelivery,
};
use crate::runtime::respond::RespondRequest;

#[derive(Default)]
struct RecordingSink {
    calls: Mutex<Vec<ChatSendIntent>>,
    fail_after: Option<usize>,
}

#[async_trait]
impl ChatSendSink for RecordingSink {
    async fn send_messages(&self, intent: ChatSendIntent) -> Result<ChatSendResult, ChatSendError> {
        let fail_after = self.fail_after;
        self.calls.lock().unwrap().push(intent.clone());
        if let Some(limit) = fail_after {
            let sent = intent
                .messages
                .iter()
                .take(limit)
                .enumerate()
                .map(|(index, text)| SentChatMessage {
                    text: text.clone(),
                    message_id: Some(format!("m-{index}")),
                })
                .collect();
            return Err(ChatSendError::Partial { sent });
        }
        Ok(ChatSendResult {
            sent: intent
                .messages
                .iter()
                .enumerate()
                .map(|(index, text)| SentChatMessage {
                    text: text.clone(),
                    message_id: Some(format!("m-{index}")),
                })
                .collect(),
        })
    }
}

fn context() -> ToolContext {
    ToolContext {
        task_id: "task-1".to_owned(),
        actor: ExecutionActorContext {
            user_id: Some("user-a".to_owned()),
            group_member_role: None,
        },
        conversation: ExecutionConversationContext {
            platform: "qq_official".to_owned(),
            account_id: Some("bot-a".to_owned()),
            kind: ConversationKind::Group,
            target_id: Some("g1".to_owned()),
            scope_id: "group:g1".to_owned(),
            interaction_scope_id: "group:g1".to_owned(),
        },
        tool_call_id: None,
        tool_round: None,
        retry_of: None,
        execution_deadline: None,
    }
}

fn request() -> RespondRequest {
    RespondRequest {
        content: "帮我看看".to_owned(),
        conversation_kind: ConversationKind::Group,
        conversation_id: Some("g1".to_owned()),
        group_id: Some("g1".to_owned()),
        user_id: Some("user-a".to_owned()),
        platform: "qq_official".to_owned(),
        account_id: Some("bot-a".to_owned()),
        message_id: Some("inbound-1".to_owned()),
        ..RespondRequest::default()
    }
}

fn scoped_tool(sink: Arc<dyn ChatSendSink>, budget: usize) -> (SendMessageTool, Arc<TurnDelivery>) {
    let tool = SendMessageTool::new(sink);
    let delivery = Arc::new(TurnDelivery::new(budget, Some("inbound-1".to_owned())));
    (
        tool.scoped_for_request(&request(), delivery.clone())
            .expect("group request should bind a send target"),
        delivery,
    )
}

#[tokio::test]
async fn sends_multiple_messages_and_binds_server_target() {
    let sink = Arc::new(RecordingSink::default());
    let (tool, delivery) = scoped_tool(sink.clone(), 4);

    let output = tool
        .execute(context(), json!({"messages": ["第一句", "第二句"]}))
        .await
        .unwrap();

    assert_eq!(output.value["ok"], true);
    assert_eq!(output.value["sent_count"], 2);
    assert_eq!(output.value["remaining"], 2);
    assert_eq!(delivery.sent_count(), 2);
    let calls = sink.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].messages,
        vec!["第一句".to_owned(), "第二句".to_owned()]
    );
    assert_eq!(calls[0].target.target_id, "g1");
    assert_eq!(calls[0].reply_to_message_id.as_deref(), Some("inbound-1"));
    assert_eq!(tool.metadata().name, SEND_MESSAGE_TOOL_NAME);
}

#[tokio::test]
async fn duplicate_batch_is_not_sent_twice() {
    let sink = Arc::new(RecordingSink::default());
    let (tool, _delivery) = scoped_tool(sink.clone(), 4);
    let arguments = json!({"messages": ["同一句"]});

    let first = tool.execute(context(), arguments.clone()).await.unwrap();
    let second = tool.execute(context(), arguments).await.unwrap();

    assert_eq!(first.value["ok"], true);
    assert_eq!(second.value["duplicate"], true);
    assert_eq!(sink.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn per_turn_budget_is_enforced_before_sending() {
    let sink = Arc::new(RecordingSink::default());
    let (tool, _delivery) = scoped_tool(sink.clone(), 2);

    let output = tool
        .execute(context(), json!({"messages": ["一", "二", "三"]}))
        .await
        .unwrap();

    assert_eq!(output.value["ok"], false);
    assert_eq!(output.value["error_code"], "budget_exceeded");
    assert!(sink.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn wechat_service_account_binds_private_customer_target() {
    let sink = Arc::new(RecordingSink::default());
    let tool = SendMessageTool::new(sink.clone());
    let mut request = request();
    request.platform = "wechat_service".to_owned();
    request.account_id = None;
    request.conversation_kind = ConversationKind::ServiceAccount;
    request.group_id = None;
    request.conversation_id = Some("openid-1".to_owned());
    let delivery = Arc::new(TurnDelivery::new(3, None));

    let scoped = tool.scoped_for_request(&request, delivery).unwrap();
    let output = scoped
        .execute(context(), json!({"messages": ["在的"]}))
        .await
        .unwrap();

    assert_eq!(output.value["ok"], true);
    let calls = sink.calls.lock().unwrap();
    assert_eq!(calls[0].target.platform, "wechat_service");
    assert_eq!(calls[0].target.target_id, "openid-1");
    assert_eq!(
        calls[0].target.target_type,
        crate::runtime::push::PushTargetType::Private
    );
}

#[tokio::test]
async fn partial_failure_records_real_sent_count() {
    let sink = Arc::new(RecordingSink {
        calls: Mutex::new(Vec::new()),
        fail_after: Some(1),
    });
    let (tool, delivery) = scoped_tool(sink, 4);

    let output = tool
        .execute(context(), json!({"messages": ["一", "二"]}))
        .await
        .unwrap();

    assert_eq!(output.value["ok"], false);
    assert_eq!(output.value["error_code"], "partial_failure");
    assert_eq!(output.value["sent_count"], 1);
    assert_eq!(delivery.sent_count(), 1);
    assert_eq!(delivery.failure_count(), 1);
}

#[tokio::test]
async fn retry_after_total_failure_is_sent_again_instead_of_reported_as_duplicate() {
    struct FailOnceSink {
        attempts: Mutex<usize>,
    }

    #[async_trait]
    impl ChatSendSink for FailOnceSink {
        async fn send_messages(
            &self,
            intent: ChatSendIntent,
        ) -> Result<ChatSendResult, ChatSendError> {
            let mut attempts = self.attempts.lock().unwrap();
            *attempts += 1;
            if *attempts == 1 {
                return Err(ChatSendError::Send);
            }
            Ok(ChatSendResult {
                sent: intent
                    .messages
                    .iter()
                    .enumerate()
                    .map(|(index, text)| SentChatMessage {
                        text: text.clone(),
                        message_id: Some(format!("m-{index}")),
                    })
                    .collect(),
            })
        }
    }

    let sink = Arc::new(FailOnceSink {
        attempts: Mutex::new(0),
    });
    let (tool, delivery) = scoped_tool(sink, 4);
    let arguments = json!({"messages": ["同一句"]});

    let first = tool.execute(context(), arguments.clone()).await.unwrap();
    assert_eq!(first.value["ok"], false);
    assert_eq!(delivery.sent_count(), 0);

    // 完整失败不能留下去重键：同样的内容重试必须真的再发一次。
    let second = tool.execute(context(), arguments).await.unwrap();
    assert_eq!(second.value["ok"], true);
    assert_eq!(second.value["sent_count"], 1);
    assert!(second.value.get("duplicate").is_none());
    assert_eq!(delivery.sent_count(), 1);
}

#[tokio::test]
async fn timeout_keeps_delivered_messages_in_ledger() {
    struct NotifyThenHangSink;

    #[async_trait]
    impl ChatSendSink for NotifyThenHangSink {
        async fn send_messages(
            &self,
            intent: ChatSendIntent,
        ) -> Result<ChatSendResult, ChatSendError> {
            if let Some(progress) = intent.on_sent.as_ref() {
                progress.notify(SentChatMessage {
                    text: intent.messages[0].clone(),
                    message_id: None,
                });
            }
            std::future::pending::<()>().await;
            unreachable!("hang sink never returns")
        }
    }

    let (tool, delivery) = scoped_tool(Arc::new(NotifyThenHangSink), 4);
    let mut ctx = context();
    ctx.execution_deadline =
        Some(tokio::time::Instant::now() + std::time::Duration::from_millis(20));

    let output = tool
        .execute(ctx, json!({"messages": ["已送达的第一句"]}))
        .await
        .unwrap();

    // 超时前已经真实送达的消息必须保留在账本里，否则最终正文会重复外发。
    assert_eq!(output.value["ok"], false);
    assert_eq!(output.value["error_code"], "timeout");
    assert_eq!(output.value["sent_count"], 1);
    assert_eq!(delivery.sent_count(), 1);
    assert_eq!(delivery.failure_count(), 1);

    // 已送达内容已登记去重键，模型同样的重试不会再发一遍。
    let retry = tool
        .execute(context(), json!({"messages": ["已送达的第一句"]}))
        .await
        .unwrap();
    assert_eq!(retry.value["duplicate"], true);
    assert_eq!(delivery.sent_count(), 1);
}
