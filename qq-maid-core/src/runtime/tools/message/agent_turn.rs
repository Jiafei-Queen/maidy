//! 发言 Tool 结果到通用 Agent outcome 的领域适配。
//!
//! `send_message` 已经在工具内完成用户可见交付，没有确定性文字回执。这里用
//! `OutOfBand` 表示“产物已独立交付”，避免通用层再把模型空正文替换成
//! “工具结果未生成确定性展示”之类的回执；真实成功/失败仍保留给诊断和验真。

use qq_maid_llm::provider::ToolExecutionResult;

use crate::runtime::respond::agent_outcome::{
    OutcomePresentation, ToolEffect, ToolExecutionOutcome, ToolOutcomeStatus,
};

use super::SEND_MESSAGE_TOOL_NAME;

pub(crate) fn tool_outcome_from_result(
    result: &ToolExecutionResult,
) -> Option<ToolExecutionOutcome> {
    if result.name != SEND_MESSAGE_TOOL_NAME {
        return None;
    }
    Some(ToolExecutionOutcome {
        tool_name: result.name.clone(),
        domain: "message".to_owned(),
        status: ToolOutcomeStatus::from_tool_result(result),
        effect: ToolEffect::ExternalSideEffect,
        presentation: OutcomePresentation::OutOfBand,
        blocks: Vec::new(),
        error_code: result
            .output
            .get("error_code")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        command: Some("message".to_owned()),
    })
}
