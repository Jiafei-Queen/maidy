import type { ConfigFieldSnapshot } from "../../types.js";

/** TTS 试听的默认文本，方便首次打开时直接验证音色。 */
export const DEFAULT_VOICE_PREVIEW_TEXT =
  "你好，我是小女仆。这是一段语音试听，用来确认 TTS 配置和音色是否正常。";

const PROVIDER_LABELS: Readonly<Record<string, string>> = {
  disabled: "关闭",
  qwen: "千问",
  openai: "OpenAI",
};

export type VoicePreviewSummary = {
  provider: string;
  providerLabel: string;
  model: string;
  voice: string;
  maxChars: number;
  /** 已保存 Provider 可试用时才允许发起试听；密钥有效性由服务端判断。 */
  available: boolean;
};

function previewValue(fields: readonly ConfigFieldSnapshot[], key: string): unknown {
  const field = fields.find((candidate) => candidate.key === key);
  return field?.effectiveValue ?? field?.savedValue ?? field?.runningValue ?? null;
}

function textValue(fields: readonly ConfigFieldSnapshot[], key: string): string {
  const value = previewValue(fields, key);
  return typeof value === "string" ? value.trim() : value === null || value === undefined ? "" : String(value).trim();
}

/** 读取配置中心已保存的字段值；试听不要求先重启。 */
export function voicePreviewSummary(fields: readonly ConfigFieldSnapshot[]): VoicePreviewSummary {
  const provider = textValue(fields, "delivery.tts.provider").toLowerCase() || "disabled";
  const active = provider === "qwen" || provider === "openai";
  const modelKey = provider === "qwen" ? "delivery.tts.qwen_model" : "delivery.tts.openai_model";
  const voiceKey = provider === "qwen" ? "delivery.tts.qwen_voice" : "delivery.tts.openai_voice";
  const maxCharsValue = Number(previewValue(fields, "delivery.tts.max_text_chars"));
  const maxChars = Number.isInteger(maxCharsValue) && maxCharsValue >= 1 && maxCharsValue <= 600
    ? maxCharsValue
    : 600;
  return {
    provider,
    providerLabel: PROVIDER_LABELS[provider] ?? provider,
    model: active ? textValue(fields, modelKey) : "",
    voice: active ? textValue(fields, voiceKey) : "",
    maxChars,
    // 密钥是否已保存由配置中心判断；这里只根据已保存 Provider 决定是否允许发起试听。
    available: active,
  };
}

/** 按 Unicode 字符数计数，避免中文与 emoji 被 UTF-16 长度误判。 */
export function voicePreviewCharCount(text: string): number {
  return [...text].length;
}

export function voicePreviewTextError(text: string, maxChars: number): string | null {
  if (text.trim().length === 0) return "请输入试听文本";
  const count = voicePreviewCharCount(text);
  if (count > maxChars) return `试听文本最多 ${maxChars} 个字符`;
  return null;
}
