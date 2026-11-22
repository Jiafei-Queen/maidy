import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import {
  ConsoleApiError,
  deleteKnowledgeFile,
  downloadKnowledgeFile,
  fetchKnowledgeCapabilities,
  listKnowledgeFiles,
  retryKnowledgeFile,
  uploadKnowledgeFile,
} from "../../api.js";
import { Button } from "../../components/ui/button.js";
import { ConfirmDialog } from "../../components/ui/dialog.js";
import { Frame, PageHeader } from "../../components/ui/frame.js";
import { Input, Select } from "../../components/ui/field.js";
import { QueryState } from "../../components/ui/query-state.js";
import { StatusBadge } from "../../components/ui/status-badge.js";
import type { KnowledgeFileItem, KnowledgeFileStatus } from "../../types.js";
import {
  formatBytes,
  formatDateTime,
  knowledgeStatusMeta,
  latestProcessingTime,
  shortContentType,
  validateKnowledgeFile,
} from "./knowledge-format.js";

const PAGE_SIZE = 20;
const POLL_INTERVAL_MS = 5_000;
const MAX_POLL_FAILURES = 3;

type StatusFilter = KnowledgeFileStatus | "all";

/** 知识库文件管理：查看、筛选、上传、下载、删除与失败重试。
 * 轮询语义：仅页面可见且存在非终态行时每 5s 刷新，全部终态即停止，
 * 连续失败达到上限后停止并提示手动刷新；终态转换以提示行报告。 */
export function KnowledgePage() {
  const queryClient = useQueryClient();
  const [searchInput, setSearchInput] = useState("");
  const [statusInput, setStatusInput] = useState<StatusFilter>("all");
  const [filters, setFilters] = useState<{ search: string; status: StatusFilter }>({ search: "", status: "all" });
  const [statusMessage, setStatusMessage] = useState<{ error: boolean; text: string } | null>(null);
  const [deleting, setDeleting] = useState<KnowledgeFileItem | null>(null);
  const previousStatusesRef = useRef(new Map<string, KnowledgeFileStatus>());
  const pollStoppedRef = useRef(false);

  const capabilitiesQuery = useQuery({
    queryKey: ["knowledge-capabilities"],
    queryFn: fetchKnowledgeCapabilities,
    staleTime: Infinity,
  });

  const listQuery = useInfiniteQuery({
    queryKey: ["knowledge", filters],
    queryFn: ({ pageParam }) =>
      listKnowledgeFiles({
        page_size: PAGE_SIZE,
        search: filters.search,
        status: filters.status,
        sort: "updated_at",
        order: "desc",
        page: pageParam,
      }),
    initialPageParam: 1,
    getNextPageParam: (last) => (last.page < last.total_pages ? last.page + 1 : undefined),
    refetchInterval: (query) => {
      const items = query.state.data?.pages.flatMap((page) => page.items) ?? [];
      const hasActive = items.some((item) => item.status === "pending" || item.status === "processing");
      if (!hasActive) return false;
      if (query.state.fetchFailureCount >= MAX_POLL_FAILURES) {
        pollStoppedRef.current = true;
        return false;
      }
      return POLL_INTERVAL_MS;
    },
    refetchIntervalInBackground: false,
  });

  const items = listQuery.data?.pages.flatMap((page) => page.items) ?? [];
  const totalFiles = listQuery.data?.pages[0]?.total;

  useEffect(() => {
    if (listQuery.data === undefined) return;
    const previous = previousStatusesRef.current;
    for (const item of items) {
      const before = previous.get(item.file_id ?? item.filename);
      if ((before === "pending" || before === "processing") && item.status === "ready") {
        setStatusMessage({ error: false, text: "文件处理完成" });
      }
      if ((before === "pending" || before === "processing") && item.status === "failed") {
        setStatusMessage({ error: true, text: "文件处理失败" });
      }
    }
    previousStatusesRef.current = new Map(items.map((item) => [item.file_id ?? item.filename, item.status]));
  }, [listQuery.data, items]);

  useEffect(() => {
    if (pollStoppedRef.current) {
      setStatusMessage({ error: true, text: "状态刷新多次失败，请手动刷新" });
    }
  }, [pollStoppedRef.current]);

  const invalidate = () => void queryClient.invalidateQueries({ queryKey: ["knowledge"] });

  const uploadMutation = useMutation({
    mutationFn: async (file: File) => {
      const capabilities = capabilitiesQuery.data;
      if (capabilities) {
        const validation = validateKnowledgeFile(file, capabilities);
        if (!validation.ok) throw new Error(`上传已阻止：${validation.reason}`);
      }
      return uploadKnowledgeFile(file);
    },
    onSuccess: () => {
      setStatusMessage({ error: false, text: "文件已上传，正在等待处理" });
      invalidate();
    },
    onError: (cause) => {
      setStatusMessage({ error: true, text: cause instanceof Error ? cause.message : "文件上传失败" });
    },
  });

  const downloadMutation = useMutation({
    mutationFn: (item: KnowledgeFileItem) => downloadKnowledgeFile(item),
    onSuccess: ({ blob, filename }) => {
      const href = URL.createObjectURL(blob);
      const anchor = document.createElement("a");
      anchor.href = href;
      anchor.download = filename;
      anchor.style.display = "none";
      document.body.append(anchor);
      anchor.click();
      anchor.remove();
      window.setTimeout(() => URL.revokeObjectURL(href), 60_000);
    },
    onError: (cause) => setStatusMessage({ error: true, text: cause instanceof Error ? cause.message : "操作失败，请稍后重试" }),
  });

  const retryMutation = useMutation({
    mutationFn: (fileId: string) => retryKnowledgeFile(fileId),
    onSuccess: () => {
      setStatusMessage({ error: false, text: "已重新提交处理" });
      invalidate();
    },
    onError: (cause) => setStatusMessage({ error: true, text: cause instanceof Error ? cause.message : "操作失败，请稍后重试" }),
  });

  const deleteMutation = useMutation({
    mutationFn: (fileId: string) => deleteKnowledgeFile(fileId),
    onSuccess: () => {
      setDeleting(null);
      setStatusMessage({ error: false, text: "文件已删除" });
      invalidate();
    },
    onError: (cause) => {
      setDeleting(null);
      setStatusMessage({
        error: true,
        text: cause instanceof ConsoleApiError && cause.status === 409 ? "文件正在处理中，暂不能删除" : cause instanceof Error ? cause.message : "操作失败，请稍后重试",
      });
    },
  });

  const busy = uploadMutation.isPending || downloadMutation.isPending || retryMutation.isPending || deleteMutation.isPending;

  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        title="知识库文件"
        subtitle="查看、筛选与上传后端处理的知识库文件；目录来源文件只读，托管文件可下载、重试或删除"
        metrics={totalFiles !== undefined ? [{ label: "已收录文件", value: totalFiles, unit: "份" }] : undefined}
        actions={
          <label className="inline-flex">
            <input
              type="file"
              hidden
              accept={capabilitiesQuery.data?.supported_extensions.join(",")}
              onChange={(event) => {
                const file = event.target.files?.[0];
                event.target.value = "";
                if (file) uploadMutation.mutate(file);
              }}
            />
            <Button
              disabled={busy || capabilitiesQuery.isLoading}
              onClick={(event) => {
                const input = event.currentTarget.parentElement?.querySelector<HTMLInputElement>("input[type=file]");
                input?.click();
              }}
            >
              {uploadMutation.isPending ? "上传中…" : "上传文件"}
            </Button>
          </label>
        }
      />

      <Frame variant="panel" className="flex flex-wrap items-center gap-2.5 p-3.5 sm:p-4">
        <Input
          type="search"
          placeholder="搜索文件名"
          value={searchInput}
          onChange={(event) => setSearchInput(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              setFilters({ search: searchInput.trim(), status: statusInput });
            }
          }}
          className="w-full sm:w-64"
        />
        <Select value={statusInput} onChange={(event) => setStatusInput(event.target.value as StatusFilter)}>
          <option value="all">全部状态</option>
          <option value="pending">等待处理</option>
          <option value="processing">处理中</option>
          <option value="ready">已完成</option>
          <option value="failed">处理失败</option>
        </Select>
        <Button size="sm" onClick={() => setFilters({ search: searchInput.trim(), status: statusInput })}>
          应用筛选
        </Button>
        <Button
          variant="secondary"
          size="sm"
          onClick={() => {
            setSearchInput("");
            setStatusInput("all");
            setFilters({ search: "", status: "all" });
          }}
        >
          重置
        </Button>
        <Button variant="ghost" size="sm" onClick={() => void listQuery.refetch()} disabled={listQuery.isFetching}>
          刷新
        </Button>
      </Frame>

      <p aria-live="polite" role="status" className={`m-0 text-xs ${statusMessage?.error ? "font-medium text-error" : "text-muted"}`}>
        {statusMessage?.text ?? (capabilitiesQuery.data ? `支持 ${capabilitiesQuery.data.supported_extensions.join(" / ")}，单文件上限 ${formatBytes(capabilitiesQuery.data.max_file_bytes)}` : "")}
      </p>

      <QueryState query={listQuery}>
        {items.length > 0 ? (
          <div className="console-paper overflow-x-auto">
            <table className="w-full border-collapse text-[13px] leading-[1.45] tabular-nums">
              <thead>
                <tr className="border-b border-line">
                  {["文件名", "类型", "大小", "上传时间", "最近处理", "状态", "失败原因", "操作"].map((label) => (
                    <th key={label} scope="col" className="px-3 py-2.5 text-left text-xs font-medium whitespace-nowrap text-muted">
                      {label}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {items.map((item) => {
                  const meta = knowledgeStatusMeta(item.status);
                  return (
                    <tr
                      key={item.file_id ?? item.filename}
                      className="border-b border-line-inner transition-colors duration-[120ms] ease-out last:border-b-0 hover:bg-accent-soft/60"
                    >
                      <td className="px-3 py-2.5 whitespace-nowrap">
                        <span className="font-medium text-ink">{item.filename}</span>
                        <span className="console-kbd ml-2 text-[11px]">
                          {item.source === "managed" ? "托管" : "目录"}
                        </span>
                      </td>
                      <td className="px-3 py-2.5 font-mono text-xs whitespace-nowrap text-muted">{shortContentType(item.content_type)}</td>
                      <td className="px-3 py-2.5 font-mono text-xs whitespace-nowrap text-muted">{formatBytes(item.size)}</td>
                      <td className="px-3 py-2.5 font-mono text-xs whitespace-nowrap text-muted">{formatDateTime(item.uploaded_at)}</td>
                      <td className="px-3 py-2.5 font-mono text-xs whitespace-nowrap text-muted">{formatDateTime(latestProcessingTime(item))}</td>
                      <td className="px-3 py-2.5 whitespace-nowrap">
                        <StatusBadge tone={meta.tone} label={meta.label} />
                      </td>
                      <td className="max-w-52 truncate px-3 py-2.5 text-xs text-error" title={item.error_summary ?? undefined}>
                        {item.error_summary ?? "—"}
                      </td>
                      <td className="px-3 py-2.5 whitespace-nowrap">
                        {item.source === "directory" ? (
                          <span className="text-muted">—</span>
                        ) : (
                          <div className="flex gap-1.5">
                            {item.downloadable ? (
                              <Button variant="secondary" size="sm" disabled={busy} onClick={() => downloadMutation.mutate(item)}>
                                下载
                              </Button>
                            ) : null}
                            {item.status === "failed" ? (
                              <Button variant="secondary" size="sm" disabled={busy} onClick={() => item.file_id !== null && retryMutation.mutate(item.file_id)}>
                                重新处理
                              </Button>
                            ) : null}
                            {item.status !== "processing" ? (
                              <Button variant="danger" size="sm" disabled={busy} onClick={() => setDeleting(item)}>
                                删除
                              </Button>
                            ) : null}
                          </div>
                        )}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        ) : listQuery.data ? (
          <Frame variant="card" className="py-8 text-center text-sm text-muted">
            暂无知识库文件
          </Frame>
        ) : null}
      </QueryState>

      {listQuery.hasNextPage ? (
        <div className="text-center">
          <Button variant="secondary" size="sm" disabled={listQuery.isFetchingNextPage} onClick={() => void listQuery.fetchNextPage()}>
            {listQuery.isFetchingNextPage ? "加载中…" : "加载更多"}
          </Button>
        </div>
      ) : null}

      <ConfirmDialog
        open={deleting !== null}
        onOpenChange={(open) => {
          if (!open) setDeleting(null);
        }}
        title={deleting ? `删除文件：${deleting.filename}` : "删除文件"}
        description="删除后，该文件及对应的知识库解析和索引数据将被移除，且无法继续被检索。此操作不可恢复。"
        confirmLabel="删除"
        danger
        busy={deleteMutation.isPending}
        onConfirm={() => {
          if (deleting?.file_id !== null && deleting?.file_id !== undefined) deleteMutation.mutate(deleting.file_id);
        }}
      />
    </div>
  );
}
