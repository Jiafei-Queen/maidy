//! `send_message` 基础设施 Tool 的 Respond 集成测试。

use qq_maid_common::identity_context::ConversationKind;
use qq_maid_llm::provider::ToolCallingProtocol;

use super::support::*;

fn exposed_tool_names(provider: &MockProvider) -> Vec<String> {
    provider
        .tool_requests()
        .remove(0)
        .tools
        .metadata()
        .into_iter()
        .map(|tool| tool.name)
        .collect()
}

#[tokio::test]
async fn private_chat_exposes_send_message_even_when_whitelist_lacks_it() {
    let provider = MockProvider::new()
        .with_tool_protocol(ToolCallingProtocol::OpenAiResponses)
        .with_tool_call_json(
            "send_message",
            r#"{"messages":["没问题，我先看一下"]}"#,
            "这句最终文本不应外发",
        );
    let (service, sink) = test_service_with_provider_and_message_tool(provider.clone(), None);
    let mut request = private_message("帮我查一下最近的新闻");
    request.account_id = Some("bot-a".to_owned());

    let response = service.respond(request).await.unwrap();

    let calls = sink.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].messages, vec!["没问题，我先看一下".to_owned()]);
    assert_eq!(calls[0].target.target_id, "u1");
    // 发言 Tool 成功后，最终模型正文被丢弃，Gateway 不再重复发送。
    assert!(response.text.is_none());
    assert!(response.markdown.is_none());
    let diagnostics = response.diagnostics.as_ref().unwrap();
    assert_eq!(diagnostics["suppressed"], true);
    assert_eq!(diagnostics["tool_delivery_sent"], 1);
    assert_eq!(diagnostics["tool_delivery_missing"], false);

    let exposed = exposed_tool_names(&provider);
    assert!(exposed.contains(&"send_message".to_owned()));
}

#[tokio::test]
async fn group_empty_whitelist_exposes_only_send_message_as_infrastructure_tool() {
    let provider = MockProvider::new()
        .with_tool_protocol(ToolCallingProtocol::OpenAiResponses)
        .with_tool_call_json("send_message", r#"{"messages":["稍等，我确认一下"]}"#, "");
    // 群聊白名单为空；受限模式下 send_message 仍必须可用。
    let (service, sink) =
        test_service_with_provider_and_message_tool(provider.clone(), Some(Vec::new()));
    let mut request = message("随便说点什么吧");
    request.account_id = Some("bot-a".to_owned());
    request.conversation_kind = ConversationKind::Group;
    request.conversation_id = Some("g1".to_owned());

    let response = service.respond(request).await.unwrap();

    let calls = sink.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].target.target_id, "g1");
    assert!(response.text.is_none());
    let diagnostics = response.diagnostics.as_ref().unwrap();
    assert_eq!(diagnostics["suppressed"], true);
    assert_eq!(diagnostics["agent_mode"], "configured_whitelist");

    let exposed = exposed_tool_names(&provider);
    assert_eq!(exposed, vec!["send_message".to_owned()]);
}

#[tokio::test]
async fn missing_send_message_call_keeps_final_text_as_fallback() {
    let provider = MockProvider::new()
        .with_tool_protocol(ToolCallingProtocol::OpenAiResponses)
        .with_tool_call_json("get_weather", r#"{"city":"杭州"}"#, "杭州今天多云。");
    let (service, sink) = test_service_with_provider_and_message_tool(provider.clone(), None);
    let mut request = private_message("杭州天气怎么样");
    request.account_id = Some("bot-a".to_owned());

    let response = service.respond(request).await.unwrap();

    assert!(sink.calls().is_empty());
    assert!(
        response
            .text
            .as_deref()
            .is_some_and(|text| !text.is_empty())
    );
    let diagnostics = response.diagnostics.as_ref().unwrap();
    assert_eq!(diagnostics["suppressed"], false);
    assert_eq!(diagnostics["tool_delivery_sent"], 0);
    assert_eq!(diagnostics["tool_delivery_missing"], true);
}
