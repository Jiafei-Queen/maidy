//! 模型主动发言领域门面。
//!
//! `send_message` 是基础设施 Tool，不是业务业务域：它不读写 Todo、记忆或 RSS，
//! 只把模型选择的内容交给 Gateway 投递到当前会话。工具始终由 Agent Runtime
//! 无条件注册，不受场景白名单和群聊受限模式影响。

mod delivery;
mod tool;

pub(crate) mod agent_turn;

pub use delivery::{
    ChatSendError, ChatSendIntent, ChatSendProgress, ChatSendResult, ChatSendSink, DynChatSendSink,
    SentChatMessage, TurnDelivery,
};
pub use tool::{SEND_MESSAGE_TOOL_NAME, SendMessageTool};

#[cfg(test)]
mod tests;
