//! 随版本审核发布的可信连接模板；模型目录无权覆盖这里的安全元数据。

use serde::Serialize;

use crate::config::agent::AgentProviderKind;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ProviderPreset {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: AgentProviderKind,
    pub base_url: &'static str,
    pub auth_header: &'static str,
    pub auth_scheme: &'static str,
}

pub fn provider_presets() -> Vec<ProviderPreset> {
    use AgentProviderKind::{OpenAiCompatible as Chat, OpenAiResponses as Responses};
    // 所有供应商平等可增删；预设只提供受信协议模板与默认地址，不包含任何凭据。
    [
        (
            "openai",
            "OpenAI (Responses)",
            Responses,
            "https://api.openai.com/v1",
        ),
        (
            "openai_chat",
            "OpenAI (Chat Completions)",
            Chat,
            "https://api.openai.com/v1",
        ),
        (
            "gemini",
            "Gemini",
            Chat,
            "https://generativelanguage.googleapis.com/v1beta/openai",
        ),
        (
            "deepseek",
            "DeepSeek",
            Responses,
            "https://api.deepseek.com",
        ),
        (
            "bigmodel",
            "智谱 BigModel",
            Chat,
            "https://open.bigmodel.cn/api/paas/v4",
        ),
        ("mimo", "MiMo", Chat, "https://api.xiaomimimo.com/v1"),
        (
            "opencode_zen",
            "OpenCode Zen Responses",
            Responses,
            "https://opencode.ai/zen/v1",
        ),
        (
            "opencode_zen_chat",
            "OpenCode Zen Chat",
            Chat,
            "https://opencode.ai/zen/v1",
        ),
        (
            "opencode_go",
            "OpenCode Go",
            Chat,
            "https://opencode.ai/zen/go/v1",
        ),
    ]
    .into_iter()
    .map(|(id, name, kind, base_url)| ProviderPreset {
        id,
        name,
        kind,
        base_url,
        auth_header: "Authorization",
        auth_scheme: "Bearer",
    })
    .collect()
}
