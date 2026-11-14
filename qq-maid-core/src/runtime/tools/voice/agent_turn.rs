//! 语音 Tool 结果到通用 Agent outcome 的领域适配。
//!
//! 语音会作为独立消息立即发送，最终文字是模型的补充说明，不应被通用层替换成
//! “工具结果未生成确定性展示”或副作用回执。这里用 `OutOfBand` 表示“产物已在
//! 工具内独立交付”，同时保留真实成功/失败状态供诊断和验真。

use qq_maid_llm::provider::ToolExecutionResult;

use crate::runtime::respond::agent_outcome::{
    OutcomePresentation, ToolEffect, ToolExecutionOutcome, ToolOutcomeStatus,
};

use super::SEND_VOICE_TOOL_NAME;

pub(crate) fn tool_outcome_from_result(
    result: &ToolExecutionResult,
) -> Option<ToolExecutionOutcome> {
    if result.name != SEND_VOICE_TOOL_NAME {
        return None;
    }
    Some(ToolExecutionOutcome {
        tool_name: result.name.clone(),
        domain: "voice".to_owned(),
        status: ToolOutcomeStatus::from_tool_result(result),
        effect: ToolEffect::ExternalSideEffect,
        presentation: OutcomePresentation::OutOfBand,
        blocks: Vec::new(),
        error_code: result
            .output
            .get("error_code")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        command: Some("voice".to_owned()),
    })
}
