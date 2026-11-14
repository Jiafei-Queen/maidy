import { afterEach, describe, expect, it, vi } from "vitest";
import { generateVoicePreview, setCsrfToken } from "../src/api.js";
import {
  DEFAULT_VOICE_PREVIEW_TEXT,
  voicePreviewCharCount,
  voicePreviewSummary,
  voicePreviewTextError,
} from "../src/features/configuration/voice-preview.js";
import type { ConfigFieldSnapshot } from "../src/types.js";

afterEach(() => {
  vi.unstubAllGlobals();
});

function field(key: string, value: unknown, options: { sensitivity?: ConfigFieldSnapshot["sensitivity"]; configured?: boolean; runningValue?: unknown } = {}): ConfigFieldSnapshot {
  return {
    key,
    module: "gateway.tts",
    valueType: typeof value === "number" ? "integer" : "string",
    source: "managed_toml",
    overridden: false,
    editable: true,
    configured: options.configured ?? value !== null,
    valid: true,
    revision: "revision-1",
    sensitivity: options.sensitivity ?? "public",
    applyMode: "restart",
    savedValue: value,
    effectiveValue: value,
    runningValue: options.runningValue === undefined ? value : options.runningValue,
    pendingRestart: false,
  };
}

describe("TTS 试听摘要", () => {
  it("使用运行态 Provider、模型、音色与密钥状态", () => {
    const summary = voicePreviewSummary([
      field("delivery.tts.provider", "openai"),
      field("delivery.tts.openai_model", "tts-1"),
      field("delivery.tts.openai_voice", "alloy"),
      field("delivery.tts.openai_api_key", null, { sensitivity: "secret", configured: true }),
      field("delivery.tts.max_text_chars", 300),
    ]);
    expect(summary).toMatchObject({
      provider: "openai",
      providerLabel: "OpenAI",
      model: "tts-1",
      voice: "alloy",
      maxChars: 300,
      available: true,
    });
  });

  it("试听读取已保存值，不要求先重启", () => {
    const summary = voicePreviewSummary([
      field("delivery.tts.provider", "openai", { runningValue: "disabled" }),
      field("delivery.tts.openai_model", "tts-1"),
      field("delivery.tts.openai_voice", "alloy"),
    ]);
    expect(summary.provider).toBe("openai");
    expect(summary.available).toBe(true);
  });

  it("运行态 Provider 决定是否允许试听，已保存密钥不参与判断", () => {
    const summary = voicePreviewSummary([
      field("delivery.tts.provider", "qwen"),
      field("delivery.tts.qwen_api_key", null, { sensitivity: "secret", configured: false }),
      field("delivery.tts.qwen_model", "qwen3-tts-flash"),
      field("delivery.tts.qwen_voice", "Cherry"),
    ]);
    expect(summary.providerLabel).toBe("千问");
    expect(summary.available).toBe(true);
  });

  it("按 Unicode 字符校验试听文本", () => {
    expect(voicePreviewCharCount("你好🙂")).toBe(3);
    expect(voicePreviewTextError("   ", 600)).toBe("请输入试听文本");
    expect(voicePreviewTextError(DEFAULT_VOICE_PREVIEW_TEXT, 3)).toBe("试听文本最多 3 个字符");
    expect(voicePreviewTextError(DEFAULT_VOICE_PREVIEW_TEXT, 600)).toBeNull();
  });
});

describe("TTS 试听请求", () => {
  it("POST 文本并返回音频 Blob 与文件名", async () => {
    const fetchMock = vi.fn(async (_input: RequestInfo | URL, _init?: RequestInit) =>
      new Response(new Blob(["audio-bytes"], { type: "audio/mpeg" }), {
        status: 200,
        headers: { "Content-Disposition": "inline; filename=\"voice-preview.mp3\"" },
      }));
    vi.stubGlobal("fetch", fetchMock);
    setCsrfToken("test-csrf");

    const result = await generateVoicePreview("试听文本");

    expect(result.filename).toBe("voice-preview.mp3");
    expect(result.blob.size).toBeGreaterThan(0);
    const [url, init] = fetchMock.mock.calls[0]!;
    expect(url).toBe("/api/v1/console/configuration/voice/preview");
    expect(init?.method).toBe("POST");
    expect(init?.credentials).toBe("same-origin");
    expect((init?.headers as Record<string, string>)["X-CSRF-Token"]).toBe("test-csrf");
    expect(JSON.parse(String(init?.body))).toEqual({ text: "试听文本" });
  });

  it("把上游错误映射为 ConsoleApiError 且保留状态码文案", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => new Response(JSON.stringify({
      ok: false,
      error: { code: "http_status", message: "TTS 服务返回 HTTP 400；请检查模型、音色、API Key 和账户余额" },
    }), { status: 502, headers: { "Content-Type": "application/json" } })));
    setCsrfToken("test-csrf");

    await expect(generateVoicePreview("试听")).rejects.toMatchObject({
      name: "ConsoleApiError",
      code: "http_status",
      status: 502,
      message: "TTS 服务返回 HTTP 400；请检查模型、音色、API Key 和账户余额",
    });
  });
});
