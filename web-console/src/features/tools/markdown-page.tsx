import { markdown } from "@codemirror/lang-markdown";
import CodeMirror from "@uiw/react-codemirror";
import { useMutation } from "@tanstack/react-query";
import { useAtomValue } from "jotai";
import { useState } from "react";
import { renderMarkdown } from "../../api.js";
import { Button } from "../../components/ui/button.js";
import { Frame, PageHeader } from "../../components/ui/frame.js";
import { themeModeAtom } from "../../stores/theme.js";

const MAX_LENGTH = 65_536;

/** Tools 页：Markdown 编辑与后端清理预览。
 * 编辑器使用 CodeMirror 6；预览 HTML 只允许来自 Rust 后端 ammonia 清理结果，
 * 不在浏览器端复制 sanitization 逻辑。 */
export function MarkdownPage() {
  const themeMode = useAtomValue(themeModeAtom);
  const [source, setSource] = useState("");
  const [sanitizedHtml, setSanitizedHtml] = useState("");
  const [error, setError] = useState<string | null>(null);

  const renderMutation = useMutation({
    mutationFn: () => renderMarkdown(source),
    onSuccess: (html) => {
      setError(null);
      setSanitizedHtml(html);
    },
    onError: (cause) => {
      setSanitizedHtml("");
      setError(cause instanceof Error ? cause.message : "Markdown 渲染失败");
    },
  });

  return (
    <div className="flex flex-col gap-7">
      <PageHeader
        title="Markdown 预览"
        subtitle="编辑器只提交当前文本；预览内容由后端解析并清理后再显示，不在浏览器端复制清理逻辑"
        metrics={[
          {
            label: "当前字数",
            value: source.length.toLocaleString(),
            unit: `/ ${MAX_LENGTH.toLocaleString()}`,
          },
        ]}
        actions={<span className="console-kbd">Ctrl / ⌘ + Enter</span>}
      />

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <Frame variant="card" className="flex flex-col gap-3">
          <div className="flex items-center justify-between border-b border-line-inner pb-2.5">
            <label htmlFor="markdown-input" className="text-[13px] font-semibold text-ink">
              Markdown 输入
            </label>
            <span className="font-mono text-xs text-muted">上限 {MAX_LENGTH.toLocaleString()} 字符</span>
          </div>
          <div
            className="overflow-hidden rounded-console-sm border border-line"
            onKeyDown={(event) => {
              if ((event.ctrlKey || event.metaKey) && event.key === "Enter") {
                event.preventDefault();
                renderMutation.mutate();
              }
            }}
          >
            <CodeMirror
              id="markdown-input"
              value={source}
              height="380px"
              theme={themeMode === "dark" ? "dark" : "light"}
              extensions={[markdown()]}
              basicSetup={{ foldGutter: false, searchKeymap: false, highlightSelectionMatches: false }}
              onChange={setSource}
              aria-label="Markdown 输入"
            />
          </div>
          <div className="flex flex-wrap items-center justify-between gap-3">
            <Button onClick={() => renderMutation.mutate()} disabled={renderMutation.isPending}>
              {renderMutation.isPending ? "渲染中…" : "渲染预览"}
            </Button>
            <span className="text-xs text-muted">按快捷键即可立即提交渲染</span>
          </div>
          {error ? (
            <p role="alert" className="m-0 text-xs font-medium text-error">
              {error}
            </p>
          ) : null}
        </Frame>

        <Frame variant="card" className="flex flex-col gap-3">
          <div className="flex items-center justify-between border-b border-line-inner pb-2.5">
            <span className="text-[13px] font-semibold text-ink">安全预览</span>
            <span className="font-mono text-xs text-muted">后端清理结果</span>
          </div>
          {sanitizedHtml ? (
            // 安全：内容仅来自后端 ammonia 清理后的 HTML，禁止将本地输入直接写入 innerHTML。
            <article
              className="markdown-output min-h-[380px] flex-1 overflow-auto text-sm leading-relaxed text-ink"
              dangerouslySetInnerHTML={{ __html: sanitizedHtml }}
            />
          ) : (
            <p className="m-0 flex min-h-[380px] flex-1 items-center justify-center text-sm text-muted" role="status">
              渲染后在此显示后端清理结果。
            </p>
          )}
        </Frame>
      </div>
    </div>
  );
}
