//! 模型可调用的语音发送工具。
//!
//! 工具只负责参数清洗、长度校验、请求级目标绑定和真实发送结果投影；TTS、
//! 平台上传与消息发送全部交给注入的 [`VoiceSendSink`]。

use std::time::Duration;

use qq_maid_common::{identity_context::ConversationKind, markdown::to_speakable_text};
use qq_maid_llm::tool::{
    Tool, ToolContext, ToolEffect, ToolMetadata, ToolOutput, ToolTimeoutPolicy,
};
use serde_json::{Value, json};
use tokio::time::{Instant, timeout_at};
use tracing::warn;

use crate::{
    config::ChatSendPacingConfig,
    error::LlmError,
    runtime::{
        push::{ONEBOT11_PLATFORM, PushTarget, PushTargetType, QQ_OFFICIAL_PLATFORM},
        respond::RespondRequest,
    },
};

use super::delivery::{DynVoiceSendSink, VoiceSendError, VoiceSendIntent};

pub const SEND_VOICE_TOOL_NAME: &str = "send_voice";

/// 工具自身兜底上限；正常路径还会受 Agent 请求截止时间约束。
const SEND_VOICE_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Clone)]
pub struct SendVoiceTool {
    sink: DynVoiceSendSink,
    max_text_chars: usize,
    target: Option<VoiceSendTarget>,
    /// 语音复用 `send_message` 的拟人延迟区间；群聊不使用。
    pacing: ChatSendPacingConfig,
}

#[derive(Debug, Clone)]
struct VoiceSendTarget {
    target: PushTarget,
    reply_to_message_id: Option<String>,
}

impl SendVoiceTool {
    /// 构造基础工具；实际执行前必须由请求级 scoped 实例替换。
    pub fn new(sink: DynVoiceSendSink, max_text_chars: usize) -> Self {
        Self {
            sink,
            max_text_chars,
            target: None,
            pacing: ChatSendPacingConfig::disabled(),
        }
    }

    /// 注入服务端采样配置；模型不能控制语音停顿区间。
    pub fn with_pacing(mut self, pacing: ChatSendPacingConfig) -> Self {
        self.pacing = pacing;
        self
    }

    /// 当前请求是否可以绑定语音发送目标。
    ///
    /// 只有 QQ 官方入口具备语音发送能力；其他平台不暴露工具，避免模型调用后
    /// 才收到“不支持”的失败回执。
    pub(crate) fn supports_request(request: &RespondRequest) -> bool {
        target_for_request(request).is_some()
    }

    /// 绑定当前请求的真实投递目标。
    ///
    /// 目标只来自服务端权威字段，模型参数不参与平台、账号或会话目标选择。
    pub(crate) fn scoped_for_request(&self, request: &RespondRequest) -> Option<Self> {
        let target = target_for_request(request)?;
        Some(Self {
            sink: self.sink.clone(),
            max_text_chars: self.max_text_chars,
            target: Some(target),
            pacing: self.pacing.clone(),
        })
    }
}

#[async_trait::async_trait]
impl Tool for SendVoiceTool {
    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: SEND_VOICE_TOOL_NAME.to_owned(),
            description: format!(
                "把一段口语化纯文本合成语音并发送到当前会话。仅在你判断适合用语音表达时调用，每次回复最多调用一次；不要用于代码、表格、长链接或技术细节。text 会直接朗读，最多 {} 个字符。最终文字回复应与语音内容互补，不要逐字重复。发送是否成功必须以本工具返回的 ok 为准；ok 不是 true 时不得声称已经发送语音。",
                self.max_text_chars
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "要朗读的口语化纯文本，不包含 Markdown、代码、链接、平台 ID 或内部标识。"
                    }
                },
                "required": ["text"],
                "additionalProperties": false
            }),
        }
    }

    fn timeout_policy(&self) -> ToolTimeoutPolicy {
        ToolTimeoutPolicy::ToolManaged
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::SideEffecting
    }

    fn max_calls_per_request(&self) -> Option<usize> {
        Some(1)
    }

    async fn execute(
        &self,
        context: ToolContext,
        arguments: Value,
    ) -> Result<ToolOutput, LlmError> {
        let raw_text = arguments
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let text = to_speakable_text(raw_text);
        let text = text.trim();
        if text.is_empty() {
            return Ok(failure_output(
                VoiceSendError::EmptyText.code(),
                "语音内容为空，本次未发送。",
            ));
        }
        let text_chars = text.chars().count();
        if text_chars > self.max_text_chars {
            return Ok(failure_output(
                VoiceSendError::TextTooLong {
                    max_chars: self.max_text_chars,
                }
                .code(),
                &format!(
                    "语音内容过长，最多 {} 个字符，本次未发送。",
                    self.max_text_chars
                ),
            ));
        }
        let Some(target) = self.target.as_ref() else {
            return Ok(failure_output(
                VoiceSendError::TargetMissing.code(),
                "当前会话缺少语音发送目标，本次未发送。",
            ));
        };

        let deadline = context
            .execution_deadline
            .unwrap_or_else(|| Instant::now() + SEND_VOICE_TIMEOUT);
        // 只有私聊有“正在输入”配合，群聊语音保持即时发送。
        //
        // 停顿目标 = 配置延迟 + 估算语音时长，让语音整体晚于纯文本回复一段
        // “朗读时间”；Gateway 会再扣掉真实 TTS 耗时，所以这里只需表达目标值。
        // 语音链路还要预留 TTS 合成和上传时间，预算不足时直接放弃停顿。
        let speech = super::estimated_speech_duration(text);
        let target_delay = (target.target.target_type == PushTargetType::Private)
            .then(|| {
                self.pacing.effective_voice_delay(
                    deadline.saturating_duration_since(Instant::now()),
                    speech,
                )
            })
            .filter(|delay| !delay.is_zero());
        let intent = VoiceSendIntent {
            target: target.target.clone(),
            reply_to_message_id: target.reply_to_message_id.clone(),
            text: text.to_owned(),
            target_delay,
        };
        let result = match timeout_at(deadline, self.sink.send_voice(intent)).await {
            Ok(result) => result,
            Err(_) => Err(VoiceSendError::Timeout),
        };
        match result {
            Ok(_) => Ok(ToolOutput::json(json!({
                "ok": true,
                "message": "语音消息已发送。"
            }))),
            Err(error) => {
                warn!(
                    error_code = error.code(),
                    tool = SEND_VOICE_TOOL_NAME,
                    "语音工具发送失败"
                );
                Ok(failure_output(error.code(), &error.user_message()))
            }
        }
    }
}

fn target_for_request(request: &RespondRequest) -> Option<VoiceSendTarget> {
    let platform = request.platform.trim();
    // OneBot 的 Core 平台名是 `onebot`，Gateway 投递路由名是 `onebot11`；
    // PushTarget::new 会在边界统一规范化，这里两个值都接受。
    if !matches!(
        platform,
        QQ_OFFICIAL_PLATFORM | ONEBOT11_PLATFORM | "onebot"
    ) {
        return None;
    }
    let account_id = clean_optional(request.account_id.as_deref())?;
    let reply_to_message_id = clean_optional(request.message_id.as_deref()).map(str::to_owned);
    let (target_type, target_id) = match request.conversation_kind {
        ConversationKind::Private => (
            PushTargetType::Private,
            clean_optional(request.conversation_id.as_deref())?,
        ),
        ConversationKind::Group => {
            let conversation_id = clean_optional(request.conversation_id.as_deref());
            let group_id = clean_optional(request.group_id.as_deref());
            // 群聊目标只信任 Gateway 归一化的 group_id；两者同时存在但不一致时
            // 宁可关闭工具，也不能把语音发到错误会话。
            let target_id = match (conversation_id, group_id) {
                (Some(conversation), Some(group)) if conversation == group => group,
                (None, Some(group)) => group,
                _ => return None,
            };
            (PushTargetType::Group, target_id)
        }
        _ => return None,
    };
    Some(VoiceSendTarget {
        target: PushTarget::new(
            platform,
            Some(account_id.to_owned()),
            target_type,
            target_id,
        ),
        reply_to_message_id,
    })
}

fn clean_optional(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn failure_output(error_code: &str, message: &str) -> ToolOutput {
    ToolOutput::json(json!({
        "ok": false,
        "error_code": error_code,
        "message": message,
    }))
}
