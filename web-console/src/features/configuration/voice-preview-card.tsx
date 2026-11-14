import { useMutation } from "@tanstack/react-query";
import { useEffect, useMemo, useRef, useState } from "react";
import { generateVoicePreview } from "../../api.js";
import { Button } from "../../components/ui/button.js";
import type { ConfigFieldSnapshot } from "../../types.js";
import {
  DEFAULT_VOICE_PREVIEW_TEXT,
  voicePreviewCharCount,
  voicePreviewSummary,
  voicePreviewTextError,
} from "./voice-preview.js";

type PreviewAudio = { url: string; filename: string };

/** 配置中心“回复与语音”里的 TTS 试听卡片。
 * 试听读取配置中心已保存的配置，保存后即可验证；重启后新配置才会影响实际 QQ 回复。 */
export function VoicePreviewCard({ fields }: { fields: readonly ConfigFieldSnapshot[] }) {
  const summary = useMemo(() => voicePreviewSummary(fields), [fields]);
  const [text, setText] = useState(DEFAULT_VOICE_PREVIEW_TEXT);
  const [audio, setAudio] = useState<PreviewAudio | null>(null);
  const [result, setResult] = useState<{ error: boolean; text: string } | null>(null);
  const audioRef = useRef<PreviewAudio | null>(null);

  useEffect(() => {
    return () => {
      if (audioRef.current) URL.revokeObjectURL(audioRef.current.url);
    };
  }, []);

  const preview = useMutation({
    mutationFn: () => generateVoicePreview(text),
    onMutate: () => {
      // 开始新请求时立即移除旧音频，避免失败后仍播放上一次文本的结果。
      if (audioRef.current) URL.revokeObjectURL(audioRef.current.url);
      audioRef.current = null;
      setAudio(null);
      setResult(null);
    },
    onSuccess: ({ blob, filename }) => {
      // 同一时间只保留一份试听音频，替换时立即释放旧 object URL。
      if (audioRef.current) URL.revokeObjectURL(audioRef.current.url);
      const next = { url: URL.createObjectURL(blob), filename };
      audioRef.current = next;
      setAudio(next);
      setResult({ error: false, text: `已生成试听音频（${formatBytes(blob.size)}）` });
    },
    onError: (cause) => {
      setResult({ error: true, text: cause instanceof Error ? cause.message : "试听生成失败" });
    },
  });

  const textError = voicePreviewTextError(text, summary.maxChars);
  const characterCount = voicePreviewCharCount(text);
  const blockedReason = !summary.available
    ? "当前保存的 TTS Provider 为关闭，请先选择 Provider 并保存。"
    : textError;

  return (
    <section aria-label="语音试听" className="mt-4 rounded-console-lg border border-line bg-glass-muted p-4">
      <div className="flex flex-col gap-1">
        <h4 className="m-0 text-sm font-bold">语音试听</h4>
        <p className="m-0 text-xs leading-relaxed text-muted">
          使用配置中心已保存的 TTS 配置生成音频并直接播放，不经过 QQ 发送。保存后即可试听；重启后新配置才会影响实际 QQ 回复。
        </p>
        <p className="m-0 text-xs text-muted">
          当前保存：{summary.providerLabel}
          {summary.model ? ` · ${summary.model}` : ""}
          {summary.voice ? ` · ${summary.voice}` : ""}
        </p>
      </div>

      <label htmlFor="voice-preview-text" className="mt-3 flex flex-col gap-1.5 text-sm font-semibold text-ink">
        试听文本
        <textarea
          id="voice-preview-text"
          value={text}
          rows={3}
          aria-invalid={textError !== null ? true : undefined}
          aria-describedby="voice-preview-text-hint"
          onChange={(event) => setText(event.target.value)}
          className={`resize-y rounded-console border bg-input px-3 py-2.5 font-normal text-ink outline-none placeholder:text-muted focus-visible:border-accent ${
            textError ? "border-error" : "border-line"
          }`}
        />
      </label>
      <div className="mt-1 flex flex-wrap items-center justify-between gap-2 text-xs">
        <p id="voice-preview-text-hint" className="m-0 text-muted">
          {textError ?? `将朗读这段文本，最多 ${summary.maxChars} 个字符。`}
        </p>
        <span className={characterCount > summary.maxChars ? "text-error" : "text-muted"}>
          {characterCount} / {summary.maxChars}
        </span>
      </div>

      <div className="mt-3 flex flex-wrap items-center gap-3">
        <Button
          variant="secondary"
          disabled={preview.isPending || blockedReason !== null}
          onClick={() => preview.mutate()}
        >
          {preview.isPending ? "正在生成…" : "生成并播放"}
        </Button>
        {result ? (
          <p role={result.error ? "alert" : "status"} className={`m-0 text-sm font-semibold ${result.error ? "text-error" : "text-success"}`}>
            {result.text}
          </p>
        ) : null}
      </div>

      {audio ? (
        <div className="mt-3 flex flex-col gap-2">
          <audio key={audio.url} controls autoPlay src={audio.url} aria-label="TTS 试听音频" className="w-full" />
          <a href={audio.url} download={audio.filename} className="text-xs font-semibold text-accent-strong underline">
            下载试听音频
          </a>
        </div>
      ) : null}
    </section>
  );
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}
