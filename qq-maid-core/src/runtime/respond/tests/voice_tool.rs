use qq_maid_common::identity_context::ConversationKind;
use qq_maid_llm::provider::ToolCallingProtocol;

use super::support::*;

#[tokio::test]
async fn group_restricted_voice_tool_sends_voice_and_keeps_model_reply() {
    let provider = MockProvider::new()
        .with_tool_protocol(ToolCallingProtocol::OpenAiResponses)
        .with_tool_call_json("send_voice", r#"{"text":"晚安，做个好梦"}"#, "晚安呀");
    let (service, sink) = test_service_with_provider_and_voice_tool(
        provider.clone(),
        Some(vec!["save_memory".to_owned(), "send_voice".to_owned()]),
    );
    let mut request = message("用语音说晚安");
    request.account_id = Some("bot-a".to_owned());
    request.conversation_kind = ConversationKind::Group;
    request.conversation_id = Some("g1".to_owned());

    let response = service.respond(request).await.unwrap();

    assert!(response.text.as_deref().unwrap().contains("晚安呀"));
    let calls = sink.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].text, "晚安，做个好梦");
    assert_eq!(calls[0].target.target_id, "g1");

    let tool_request = provider.tool_requests().remove(0);
    let exposed = tool_request
        .tools
        .metadata()
        .into_iter()
        .map(|tool| tool.name)
        .collect::<Vec<_>>();
    assert!(exposed.contains(&"send_voice".to_owned()));
    assert!(!exposed.contains(&"get_weather".to_owned()));
}
