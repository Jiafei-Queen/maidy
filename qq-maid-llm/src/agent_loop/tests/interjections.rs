//! 回合内用户插话在 Agent Loop 中的注入时机测试。

use super::*;

use crate::agent_loop::types::{DynAgentInterjectionSource, USER_INTERJECTION_PREFIX};

struct EchoTool;

#[async_trait]
impl crate::tool::Tool for EchoTool {
    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: "echo".to_owned(),
            description: "echo".to_owned(),
            parameters: json!({"type": "object", "properties": {}}),
        }
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    async fn execute(
        &self,
        _context: ToolContext,
        _arguments: Value,
    ) -> Result<ToolOutput, LlmError> {
        Ok(ToolOutput::json(json!({"ok": true, "value": "done"})))
    }
}

/// 记录每次 advance 时已经排队的插话，验证 Provider 收到插话的轮次。
struct InterjectionRecordingSession {
    script: VecDeque<AgentStep>,
    pending: Vec<String>,
    queued_log: Arc<StdMutex<Vec<String>>>,
    advance_log: Arc<StdMutex<Vec<Vec<String>>>>,
}

#[async_trait]
impl AgentStepSession for InterjectionRecordingSession {
    fn provider(&self) -> &str {
        "test"
    }

    fn model(&self) -> &str {
        "test-model"
    }

    fn queue_user_interjections(&mut self, texts: &[String]) -> usize {
        self.pending.extend(texts.iter().cloned());
        self.queued_log
            .lock()
            .unwrap()
            .extend(texts.iter().cloned());
        texts.len()
    }

    fn take_pending_interjections(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending)
    }

    async fn advance(
        &mut self,
        _results: &[AgentToolResult],
        _allow_tool_calls: bool,
    ) -> Result<AgentStep, LlmError> {
        self.advance_log
            .lock()
            .unwrap()
            .push(std::mem::take(&mut self.pending));
        self.script
            .pop_front()
            .ok_or_else(|| LlmError::provider("script exhausted", "test"))
    }
}

struct ScriptedInterjections {
    batches: StdMutex<VecDeque<Vec<String>>>,
    requeued: StdMutex<Vec<String>>,
}

impl ScriptedInterjections {
    fn new(batches: Vec<Vec<&'static str>>) -> Arc<Self> {
        Arc::new(Self {
            batches: StdMutex::new(
                batches
                    .into_iter()
                    .map(|batch| batch.into_iter().map(str::to_owned).collect())
                    .collect(),
            ),
            requeued: StdMutex::new(Vec::new()),
        })
    }

    fn requeued(&self) -> Vec<String> {
        self.requeued.lock().unwrap().clone()
    }
}

impl crate::agent_loop::AgentInterjectionSource for ScriptedInterjections {
    fn drain(&self) -> Vec<String> {
        self.batches
            .lock()
            .unwrap()
            .pop_front()
            .map(|batch| {
                batch
                    .into_iter()
                    // 模仿真实来源：Provider 侧统一加标记，这里只校验原文。
                    .inspect(|text| {
                        assert!(!text.contains(USER_INTERJECTION_PREFIX));
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn requeue(&self, texts: Vec<String>) {
        self.requeued.lock().unwrap().extend(texts);
    }
}

fn final_reply(reply: &str) -> AgentStep {
    AgentStep::FinalAnswer {
        reply: reply.to_owned(),
        output_parts: Vec::new(),
        usage: None,
    }
}

fn tool_call() -> AgentStep {
    AgentStep::ToolCalls {
        calls: vec![AgentToolCall {
            name: "echo".to_owned(),
            call_id: "call-1".to_owned(),
            arguments: "{}".to_owned(),
        }],
        usage: None,
    }
}

fn session(
    script: Vec<AgentStep>,
    queued_log: Arc<StdMutex<Vec<String>>>,
    advance_log: Arc<StdMutex<Vec<Vec<String>>>>,
) -> InterjectionRecordingSession {
    InterjectionRecordingSession {
        script: script.into(),
        pending: Vec::new(),
        queued_log,
        advance_log,
    }
}

#[tokio::test]
async fn interjection_is_queued_after_tool_results_and_before_next_model_round() {
    let queued_log = Arc::new(StdMutex::new(Vec::new()));
    let advance_log = Arc::new(StdMutex::new(Vec::new()));
    let interjections: DynAgentInterjectionSource =
        ScriptedInterjections::new(vec![vec!["好了吗"]]);
    let registry = ToolRegistry::new().register(EchoTool).unwrap();

    let outcome = crate::agent_loop::run_agent_loop_with_handle_and_interjections(
        Box::new(session(
            vec![tool_call(), final_reply("已经查好了")],
            queued_log.clone(),
            advance_log.clone(),
        )),
        registry,
        test_context(),
        3,
        None,
        None,
        None,
        Some(interjections),
    )
    .await
    .unwrap();

    assert_eq!(outcome.reply, "已经查好了");
    assert_eq!(*queued_log.lock().unwrap(), vec!["好了吗".to_owned()]);
    let advance = advance_log.lock().unwrap();
    assert_eq!(advance.len(), 2);
    assert!(advance[0].is_empty(), "首轮不应提前注入插话");
    assert_eq!(advance[1], vec!["好了吗".to_owned()]);
    assert_eq!(outcome.agent.interjections_received, 1);
    assert_eq!(outcome.agent.interjections_injected, 1);
}

#[tokio::test]
async fn interjection_arriving_during_final_answer_triggers_one_extra_round() {
    let queued_log = Arc::new(StdMutex::new(Vec::new()));
    let advance_log = Arc::new(StdMutex::new(Vec::new()));
    let interjections: DynAgentInterjectionSource =
        ScriptedInterjections::new(vec![vec!["等一下"]]);
    let registry = ToolRegistry::new().register(EchoTool).unwrap();

    let outcome = crate::agent_loop::run_agent_loop_with_handle_and_interjections(
        Box::new(session(
            vec![final_reply("第一版"), final_reply("补充后的回答")],
            queued_log.clone(),
            advance_log.clone(),
        )),
        registry,
        test_context(),
        2,
        None,
        None,
        None,
        Some(interjections),
    )
    .await
    .unwrap();

    assert_eq!(outcome.reply, "补充后的回答");
    let advance = advance_log.lock().unwrap();
    assert_eq!(advance.len(), 2);
    assert!(advance[0].is_empty());
    assert_eq!(advance[1], vec!["等一下".to_owned()]);
    assert_eq!(outcome.agent.interjections_received, 1);
    assert_eq!(outcome.agent.interjections_injected, 1);
}

#[tokio::test]
async fn interjection_on_last_allowed_round_is_returned_instead_of_dropped() {
    let queued_log = Arc::new(StdMutex::new(Vec::new()));
    let advance_log = Arc::new(StdMutex::new(Vec::new()));
    // 第一轮插话触发跟进轮；最后一轮到达的插话已无预算，只能退回来源。
    let interjections = ScriptedInterjections::new(vec![vec!["继续"], vec!["等一下"]]);
    let registry = ToolRegistry::new().register(EchoTool).unwrap();

    let outcome = crate::agent_loop::run_agent_loop_with_handle_and_interjections(
        Box::new(session(
            vec![final_reply("第一版"), final_reply("第二版")],
            queued_log.clone(),
            advance_log.clone(),
        )),
        registry,
        test_context(),
        1,
        None,
        None,
        None,
        Some(interjections.clone()),
    )
    .await
    .unwrap();

    assert_eq!(outcome.reply, "第二版");
    assert_eq!(interjections.requeued(), vec!["等一下".to_owned()]);
    assert_eq!(outcome.agent.interjections_injected, 1);
}

#[tokio::test]
async fn provider_without_interjection_support_gets_texts_requeued() {
    let session = ScriptedSession::new("mock", "m", vec![final_reply("直接回答")]);
    let interjections = ScriptedInterjections::new(vec![vec!["在吗"]]);
    let registry = ToolRegistry::new().register(EchoTool).unwrap();

    let outcome = crate::agent_loop::run_agent_loop_with_handle_and_interjections(
        Box::new(session),
        registry,
        test_context(),
        2,
        None,
        None,
        None,
        Some(interjections.clone()),
    )
    .await
    .unwrap();

    assert_eq!(outcome.reply, "直接回答");
    assert_eq!(interjections.requeued(), vec!["在吗".to_owned()]);
    assert_eq!(outcome.agent.interjections_injected, 0);
}
