use std::sync::{Arc, Mutex};

use qq_maid_common::{
    identity_context::{ConversationKind, ExecutionActorContext, ExecutionConversationContext},
    markdown::to_speakable_text,
};
use qq_maid_llm::tool::{Tool, ToolContext, ToolEffect, ToolTimeoutPolicy};
use serde_json::json;

use super::{
    SEND_VOICE_TOOL_NAME, SendVoiceTool, VoiceSendError, VoiceSendIntent, VoiceSendResult,
    VoiceSendSink,
};
use crate::{runtime::push::PushTargetType, runtime::respond::RespondRequest};

struct RecordingSink {
    calls: Mutex<Vec<VoiceSendIntent>>,
    result: Result<VoiceSendResult, VoiceSendError>,
}

impl RecordingSink {
    fn success() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            result: Ok(VoiceSendResult {
                message_id: Some("message-1".to_owned()),
            }),
        }
    }

    fn failure(error: VoiceSendError) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            result: Err(error),
        }
    }

    fn calls(&self) -> Vec<VoiceSendIntent> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl VoiceSendSink for RecordingSink {
    async fn send_voice(&self, intent: VoiceSendIntent) -> Result<VoiceSendResult, VoiceSendError> {
        self.calls.lock().unwrap().push(intent);
        self.result.clone()
    }
}

fn private_request() -> RespondRequest {
    RespondRequest {
        platform: "qq_official".to_owned(),
        account_id: Some("bot-a".to_owned()),
        conversation_kind: ConversationKind::Private,
        conversation_id: Some("user-a".to_owned()),
        message_id: Some("inbound-1".to_owned()),
        ..Default::default()
    }
}

fn group_request() -> RespondRequest {
    RespondRequest {
        platform: "qq_official".to_owned(),
        account_id: Some("bot-a".to_owned()),
        conversation_kind: ConversationKind::Group,
        conversation_id: Some("group-a".to_owned()),
        group_id: Some("group-a".to_owned()),
        message_id: Some("inbound-2".to_owned()),
        ..Default::default()
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
            kind: ConversationKind::Private,
            target_id: Some("user-a".to_owned()),
            scope_id: "scope".to_owned(),
            interaction_scope_id: "scope".to_owned(),
        },
        tool_call_id: None,
        tool_round: None,
        retry_of: None,
        execution_deadline: None,
    }
}

fn scoped_tool(sink: Arc<RecordingSink>, request: &RespondRequest) -> SendVoiceTool {
    SendVoiceTool::new(sink, 600)
        .scoped_for_request(request)
        .expect("request should provide a voice target")
}

#[tokio::test]
async fn private_voice_tool_sends_cleaned_text_to_current_target() {
    let sink = Arc::new(RecordingSink::success());
    let tool = scoped_tool(sink.clone(), &private_request());

    let output = tool
        .execute(
            context(),
            json!({"text": "**晚安**\n[链接](https://example.test)"}),
        )
        .await
        .unwrap();

    assert_eq!(output.value["ok"], true);
    assert!(output.value.get("message_id").is_none());
    let calls = sink.calls();
    assert_eq!(calls.len(), 1);
    let intent = &calls[0];
    assert_eq!(intent.target.target_type, PushTargetType::Private);
    assert_eq!(intent.target.target_id, "user-a");
    assert_eq!(intent.target.account_id.as_deref(), Some("bot-a"));
    assert_eq!(intent.reply_to_message_id.as_deref(), Some("inbound-1"));
    assert_eq!(
        intent.text,
        to_speakable_text("**晚安**\n[链接](https://example.test)")
    );
    assert!(!intent.text.contains("**"));
}

#[tokio::test]
async fn group_voice_target_requires_authoritative_group_id() {
    let sink = Arc::new(RecordingSink::success());
    let tool = scoped_tool(sink.clone(), &group_request());
    let output = tool
        .execute(context(), json!({"text": "群聊里发一条语音"}))
        .await
        .unwrap();
    assert_eq!(output.value["ok"], true);
    let calls = sink.calls();
    assert_eq!(calls[0].target.target_type, PushTargetType::Group);
    assert_eq!(calls[0].target.target_id, "group-a");

    let mut mismatched = group_request();
    mismatched.conversation_id = Some("other-group".to_owned());
    assert!(
        SendVoiceTool::new(Arc::new(RecordingSink::success()), 600)
            .scoped_for_request(&mismatched)
            .is_none()
    );
}

#[tokio::test]
async fn invalid_text_is_rejected_before_the_sink_is_called() {
    let sink = Arc::new(RecordingSink::success());
    let tool = scoped_tool(sink.clone(), &private_request());

    for (text, expected_code) in [("   ", "empty_text"), (&"朗".repeat(601), "text_too_long")] {
        let output = tool
            .execute(context(), json!({ "text": text }))
            .await
            .unwrap();
        assert_eq!(output.value["ok"], false);
        assert_eq!(output.value["error_code"], expected_code);
    }
    assert!(sink.calls().is_empty());
}

#[tokio::test]
async fn sink_failure_is_reported_without_fake_success() {
    let sink = Arc::new(RecordingSink::failure(VoiceSendError::Tts {
        code: "http_status",
    }));
    let tool = scoped_tool(sink, &private_request());

    let output = tool
        .execute(context(), json!({"text": "晚安"}))
        .await
        .unwrap();

    assert_eq!(output.value["ok"], false);
    assert_eq!(output.value["error_code"], "tts_error");
    assert!(output.value["message"].as_str().unwrap().contains("未发送"));
}

#[test]
fn metadata_is_side_effecting_and_limited_to_one_call() {
    let tool = SendVoiceTool::new(Arc::new(RecordingSink::success()), 600);
    let metadata = tool.metadata();
    assert_eq!(metadata.name, SEND_VOICE_TOOL_NAME);
    assert_eq!(metadata.parameters["required"], json!(["text"]));
    assert_eq!(tool.effect(), ToolEffect::SideEffecting);
    assert_eq!(tool.timeout_policy(), ToolTimeoutPolicy::ToolManaged);
    assert_eq!(tool.max_calls_per_request(), Some(1));
}

#[tokio::test]
async fn onebot_requests_bind_a_normalized_voice_target() {
    let sink = Arc::new(RecordingSink::success());
    let tool = SendVoiceTool::new(sink.clone(), 600);
    let mut request = private_request();
    request.platform = "onebot".to_owned();
    request.account_id = Some("10001".to_owned());
    request.conversation_id = Some("20002".to_owned());
    let scoped = tool
        .scoped_for_request(&request)
        .expect("onebot request should provide a voice target");
    assert!(SendVoiceTool::supports_request(&request));

    let output = scoped
        .execute(context(), json!({"text": "晚安"}))
        .await
        .unwrap();
    assert_eq!(output.value["ok"], true);
    let calls = sink.calls();
    assert_eq!(calls[0].target.platform, "onebot11");
    assert_eq!(calls[0].target.account_id.as_deref(), Some("10001"));
    assert_eq!(calls[0].target.target_id, "20002");
}

#[test]
fn unsupported_platforms_cannot_bind_a_voice_target() {
    let sink = Arc::new(RecordingSink::success());
    let tool = SendVoiceTool::new(sink, 600);
    let mut request = private_request();
    request.platform = "wechat_service".to_owned();
    assert!(tool.scoped_for_request(&request).is_none());
    assert!(!SendVoiceTool::supports_request(&request));
}

#[tokio::test]
async fn private_voice_delay_includes_estimated_speech_duration() {
    use crate::config::ChatSendPacingConfig;
    use crate::runtime::tools::voice::estimated_speech_duration;
    use std::time::Duration;

    let sink = Arc::new(RecordingSink::success());
    let pacing = ChatSendPacingConfig {
        delay_min: Duration::from_millis(1000),
        delay_max: Duration::from_millis(1000),
        gap: Duration::ZERO,
    };
    let tool = SendVoiceTool::new(sink.clone(), 600)
        .with_pacing(pacing)
        .scoped_for_request(&private_request())
        .expect("request should provide a voice target");

    let text = "晚安，做个好梦";
    tool.execute(context(), json!({ "text": text }))
        .await
        .unwrap();

    let calls = sink.calls();
    // 停顿目标 = 1 秒基础延迟 + 估算语音时长，让语音整体晚一个朗读周期。
    assert_eq!(
        calls[0].target_delay,
        Some(Duration::from_millis(1000) + estimated_speech_duration(text))
    );
}

#[tokio::test]
async fn group_voice_never_requests_pacing_delay() {
    use crate::config::ChatSendPacingConfig;
    use std::time::Duration;

    let sink = Arc::new(RecordingSink::success());
    let pacing = ChatSendPacingConfig {
        delay_min: Duration::from_millis(1500),
        delay_max: Duration::from_millis(1500),
        gap: Duration::ZERO,
    };
    // 群聊没有“正在输入”，延迟只会让语音无故变慢。
    let tool = SendVoiceTool::new(sink.clone(), 600)
        .with_pacing(pacing)
        .scoped_for_request(&group_request())
        .expect("request should provide a voice target");

    tool.execute(context(), json!({"text": "群里晚安"}))
        .await
        .unwrap();

    assert_eq!(sink.calls()[0].target_delay, None);
}

#[tokio::test]
async fn voice_pacing_is_dropped_when_tool_budget_is_short() {
    use crate::config::ChatSendPacingConfig;
    use std::time::Duration;

    let sink = Arc::new(RecordingSink::success());
    let pacing = ChatSendPacingConfig {
        delay_min: Duration::from_secs(20),
        delay_max: Duration::from_secs(20),
        gap: Duration::ZERO,
    };
    let tool = SendVoiceTool::new(sink.clone(), 600)
        .with_pacing(pacing)
        .scoped_for_request(&private_request())
        .expect("request should provide a voice target");
    let mut ctx = context();
    // 语音链路要留 8 秒真实发送预算；剩余 5 秒时连“延迟 + 语音时长”都放不下。
    ctx.execution_deadline = Some(tokio::time::Instant::now() + Duration::from_secs(5));

    tool.execute(ctx, json!({"text": "晚安"})).await.unwrap();

    assert_eq!(sink.calls()[0].target_delay, None);
}
