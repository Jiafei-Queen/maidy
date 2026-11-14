//! 语音工具领域门面。
//!
//! Core 只在这里暴露模型可调用的 `send_voice`、跨层发送契约和结果投影；
//! 具体 TTS、音频上传与平台发送由 Gateway 实现，旧会话偏好表只保留 migration
//! 兼容常量，不再承载运行时状态。

mod agent_turn;
mod delivery;
mod legacy;
mod tool;

#[cfg(test)]
mod tests;

pub(crate) use agent_turn::tool_outcome_from_result;

pub use delivery::{
    DynVoiceSendSink, VoiceSendError, VoiceSendIntent, VoiceSendResult, VoiceSendSink,
};
pub use legacy::VOICE_PREFERENCE_SCHEMA_V1;
pub use tool::{SEND_VOICE_TOOL_NAME, SendVoiceTool};
