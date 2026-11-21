use super::*;

#[tokio::test]
async fn unavailable_qq_channel_returns_immediate_explicit_error() {
    let sink = GatewayPushSink::unbound();
    sink.mark_qq_official_unavailable("QQ official channel is not bound");
    let intent = PushIntent {
        target: PushTarget::qq_official(PushTargetType::Private, "user-1"),
        mentions: Vec::new(),
        text: "hello".to_owned(),
        fallback_text: None,
        message_type: "text".to_owned(),
        visible_entity_snapshot: None,
    };

    let err = sink.push(intent).await.unwrap_err();

    assert!(err.to_string().contains("QQ official channel is not bound"));
}

#[tokio::test]
async fn onebot_push_routes_independently_when_qq_is_unavailable() {
    let sink = GatewayPushSink::unbound();
    sink.mark_qq_official_unavailable("QQ official channel is not bound");
    let sender = Arc::new(MockOneBotSender::connected("bot-1"));
    sink.bind_onebot_sender(sender.clone(), crate::gateway::ref_index::ref_index());
    let intent = PushIntent {
        target: PushTarget::onebot11("bot-1", PushTargetType::Private, "user-1"),
        mentions: Vec::new(),
        text: "# Markdown".to_owned(),
        fallback_text: Some("纯文本".to_owned()),
        message_type: "markdown".to_owned(),
        visible_entity_snapshot: None,
    };

    let result = sink.push(intent).await.unwrap();

    assert_eq!(result.message_id.as_deref(), Some("ob-private-1"));
    assert_eq!(sender.calls(), vec!["private:user-1:纯文本"]);
}

#[tokio::test]
async fn onebot_group_push_routes_to_group_action_sender() {
    let sink = GatewayPushSink::unbound();
    let sender = Arc::new(MockOneBotSender::connected("bot-1"));
    sink.bind_onebot_sender(sender.clone(), crate::gateway::ref_index::ref_index());

    let result = sink
        .push(PushIntent {
            target: PushTarget::onebot11("bot-1", PushTargetType::Group, "group-1"),
            mentions: Vec::new(),
            text: "group text".to_owned(),
            fallback_text: None,
            message_type: "text".to_owned(),
            visible_entity_snapshot: None,
        })
        .await
        .unwrap();

    assert_eq!(result.message_id.as_deref(), Some("ob-group-1"));
    assert_eq!(sender.calls(), vec!["group:group-1:group text"]);
}

#[tokio::test]
async fn onebot_group_push_uses_native_deduplicated_at_segments() {
    let sink = GatewayPushSink::unbound();
    let sender = Arc::new(MockOneBotSender::connected("bot-1"));
    sink.bind_onebot_sender(sender.clone(), crate::gateway::ref_index::ref_index());

    sink.push(PushIntent {
        target: PushTarget::onebot11("bot-1", PushTargetType::Group, "group-1"),
        mentions: vec![
            PushMention::new("1001", None),
            PushMention::new("1002", None),
            PushMention::new("1001", Some("重复".to_owned())),
            PushMention::new("", None),
        ],
        text: "提醒正文".to_owned(),
        fallback_text: None,
        message_type: "text".to_owned(),
        visible_entity_snapshot: None,
    })
    .await
    .unwrap();

    assert_eq!(sender.calls(), vec!["group:group-1:at=1001,1002:提醒正文"]);
}

#[tokio::test]
async fn onebot_invalid_member_downgrades_by_name_without_losing_body() {
    let sink = GatewayPushSink::unbound();
    let sender = Arc::new(MockOneBotSender::connected("bot-1"));
    sink.bind_onebot_sender(sender.clone(), crate::gateway::ref_index::ref_index());

    sink.push(PushIntent {
        target: PushTarget::onebot11("bot-1", PushTargetType::Group, "group-1"),
        mentions: vec![PushMention::new(
            "invalid-id",
            Some("张三\n伪造行".to_owned()),
        )],
        text: "提醒正文".to_owned(),
        fallback_text: None,
        message_type: "text".to_owned(),
        visible_entity_snapshot: None,
    })
    .await
    .unwrap();

    assert_eq!(
        sender.calls(),
        vec!["group:group-1:提醒成员：张三 伪造行\n\n提醒正文"]
    );
}

#[tokio::test]
async fn private_push_ignores_group_mentions_and_keeps_body_unchanged() {
    let sink = GatewayPushSink::unbound();
    let sender = Arc::new(MockOneBotSender::connected("bot-1"));
    sink.bind_onebot_sender(sender.clone(), crate::gateway::ref_index::ref_index());

    sink.push(PushIntent {
        target: PushTarget::onebot11("bot-1", PushTargetType::Private, "1000"),
        mentions: vec![PushMention::new("bad", Some("不应展示".to_owned()))],
        text: "私聊正文".to_owned(),
        fallback_text: None,
        message_type: "text".to_owned(),
        visible_entity_snapshot: None,
    })
    .await
    .unwrap();

    assert_eq!(sender.calls(), vec!["private:1000:私聊正文"]);
}

#[tokio::test]
async fn onebot_push_records_returned_message_id_in_ref_index() {
    let sink = GatewayPushSink::unbound();
    let sender = Arc::new(MockOneBotSender::connected("bot-1"));
    let ref_index = crate::gateway::ref_index::ref_index();
    sink.bind_onebot_sender(sender, ref_index.clone());

    sink.push(PushIntent {
        target: PushTarget::onebot11("bot-1", PushTargetType::Private, "user-1"),
        mentions: Vec::new(),
        text: "提醒正文".to_owned(),
        fallback_text: None,
        message_type: "text".to_owned(),
        visible_entity_snapshot: None,
    })
    .await
    .unwrap();

    let quoted = quoted_onebot_context(
        &ref_index,
        "bot-1",
        ConversationTarget::Private {
            target_id: "user-1".to_owned(),
        },
        "ob-private-1",
    );
    assert!(quoted.lookup_found);
    assert_eq!(quoted.text_summary.as_deref(), Some("提醒正文"));
    assert_eq!(quoted.from_bot, Some(true));
}

#[tokio::test]
async fn onebot_push_rejects_offline_missing_or_wrong_account_without_sending() {
    let missing_account_sender = Arc::new(MockOneBotSender::connected("bot-1"));
    let runtime = OneBotPushRuntime {
        sender: missing_account_sender.clone(),
        ref_index: crate::gateway::ref_index::ref_index(),
    };
    let base = PushIntent {
        target: PushTarget::new(ONEBOT11_PLATFORM, None, PushTargetType::Group, "group-1"),
        mentions: Vec::new(),
        text: "hello".to_owned(),
        fallback_text: None,
        message_type: "text".to_owned(),
        visible_entity_snapshot: None,
    };
    let missing = runtime.push(base.clone()).await.unwrap_err();
    assert!(missing.to_string().contains("account_id is required"));

    let wrong = runtime
        .push(PushIntent {
            target: PushTarget::new(
                ONEBOT11_PLATFORM,
                Some("bot-2".to_owned()),
                PushTargetType::Group,
                "group-1",
            ),
            ..base.clone()
        })
        .await
        .unwrap_err();
    assert!(wrong.to_string().contains("does not match"));
    assert!(missing_account_sender.calls().is_empty());

    let offline = OneBotPushRuntime {
        sender: Arc::new(MockOneBotSender {
            account_id: None,
            calls: Mutex::new(Vec::new()),
            fail: false,
        }),
        ref_index: crate::gateway::ref_index::ref_index(),
    }
    .push(PushIntent {
        target: PushTarget::new(
            ONEBOT11_PLATFORM,
            Some("bot-1".to_owned()),
            PushTargetType::Group,
            "group-1",
        ),
        ..base
    })
    .await
    .unwrap_err();
    assert!(offline.to_string().contains("offline"));
}

#[tokio::test]
async fn qq_target_never_falls_through_to_bound_onebot_sender() {
    let sink = GatewayPushSink::unbound();
    sink.mark_qq_official_unavailable("QQ official channel is not bound");
    let sender = Arc::new(MockOneBotSender::connected("bot-1"));
    sink.bind_onebot_sender(sender.clone(), crate::gateway::ref_index::ref_index());

    let err = sink
        .push(PushIntent {
            target: PushTarget::qq_official(PushTargetType::Private, "user-1"),
            mentions: Vec::new(),
            text: "hello".to_owned(),
            fallback_text: None,
            message_type: "text".to_owned(),
            visible_entity_snapshot: None,
        })
        .await
        .unwrap_err();

    assert!(err.to_string().contains("QQ official channel is not bound"));
    assert!(sender.calls().is_empty());
}

#[tokio::test]
async fn onebot_transport_failure_is_propagated_for_outbox_retry() {
    let sink = GatewayPushSink::unbound();
    sink.bind_onebot_sender(
        Arc::new(MockOneBotSender {
            account_id: Some("bot-1".to_owned()),
            calls: Mutex::new(Vec::new()),
            fail: true,
        }),
        crate::gateway::ref_index::ref_index(),
    );

    let err = sink
        .push(PushIntent {
            target: PushTarget::onebot11("bot-1", PushTargetType::Private, "user-1"),
            mentions: Vec::new(),
            text: "hello".to_owned(),
            fallback_text: None,
            message_type: "text".to_owned(),
            visible_entity_snapshot: None,
        })
        .await
        .unwrap_err();

    assert!(err.to_string().contains("outbound queue is closed"));
}

#[tokio::test]
async fn onebot_voice_tool_sends_record_segment_for_private_and_group_targets() {
    use qq_maid_core::runtime::tools::voice::{VoiceSendIntent, VoiceSendSink};

    let sink = GatewayPushSink::unbound();
    let sender = Arc::new(MockOneBotSender::connected("bot-1"));
    sink.bind_onebot_sender(sender.clone(), crate::gateway::ref_index::ref_index());
    sink.set_voice_provider(Some(Arc::new(StaticTtsProvider)));

    let private = sink
        .send_voice(VoiceSendIntent {
            target: PushTarget::onebot11("bot-1", PushTargetType::Private, "20002"),
            reply_to_message_id: None,
            text: "晚安".to_owned(),
        })
        .await
        .unwrap();
    assert_eq!(private.message_id.as_deref(), Some("ob-private-voice-1"));

    let group = sink
        .send_voice(VoiceSendIntent {
            target: PushTarget::onebot11("bot-1", PushTargetType::Group, "30003"),
            reply_to_message_id: None,
            text: "群里晚安".to_owned(),
        })
        .await
        .unwrap();
    assert_eq!(group.message_id.as_deref(), Some("ob-group-voice-1"));
    assert_eq!(
        sender.calls(),
        vec![
            "private-voice:20002:https://audio.example.test/voice.wav",
            "group-voice:30003:https://audio.example.test/voice.wav",
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn onebot_private_send_delay_shows_typing_before_message() {
    use qq_maid_core::runtime::tools::message::{ChatSendIntent, ChatSendSink};
    use std::time::Duration;

    let sink = GatewayPushSink::unbound();
    let sender = Arc::new(MockOneBotSender::connected("bot-1"));
    sink.bind_onebot_sender(sender.clone(), crate::gateway::ref_index::ref_index());
    let sink = Arc::new(sink);
    let sender_for_task = sink.clone();
    let task = tokio::spawn(async move {
        sender_for_task
            .send_messages(ChatSendIntent {
                target: PushTarget::onebot11("bot-1", PushTargetType::Private, "user-1"),
                reply_to_message_id: None,
                messages: vec!["稍等".to_owned()],
                initial_delay: Some(Duration::from_millis(1000)),
                gap: None,
                on_sent: None,
            })
            .await
    });

    tokio::task::yield_now().await;
    assert_eq!(sender.calls(), vec!["typing:user-1"]);
    tokio::time::advance(Duration::from_millis(999)).await;
    tokio::task::yield_now().await;
    assert_eq!(sender.calls(), vec!["typing:user-1"]);

    tokio::time::advance(Duration::from_millis(1)).await;
    let result = task.await.expect("delay task should not panic").unwrap();
    assert_eq!(result.sent.len(), 1);
    assert_eq!(sender.calls(), vec!["typing:user-1", "private:user-1:稍等"]);
}

#[tokio::test(start_paused = true)]
async fn onebot_group_send_delay_does_not_send_typing() {
    use qq_maid_core::runtime::tools::message::{ChatSendIntent, ChatSendSink};
    use std::time::Duration;

    let sink = GatewayPushSink::unbound();
    let sender = Arc::new(MockOneBotSender::connected("bot-1"));
    sink.bind_onebot_sender(sender.clone(), crate::gateway::ref_index::ref_index());
    let sink = Arc::new(sink);
    let sender_for_task = sink.clone();
    let task = tokio::spawn(async move {
        sender_for_task
            .send_messages(ChatSendIntent {
                target: PushTarget::onebot11("bot-1", PushTargetType::Group, "group-1"),
                reply_to_message_id: None,
                messages: vec!["群里回复".to_owned()],
                initial_delay: Some(Duration::from_millis(500)),
                gap: None,
                on_sent: None,
            })
            .await
    });

    tokio::task::yield_now().await;
    assert!(sender.calls().is_empty());
    tokio::time::advance(Duration::from_millis(500)).await;
    task.await.expect("delay task should not panic").unwrap();
    assert_eq!(sender.calls(), vec!["group:group-1:群里回复"]);
}
