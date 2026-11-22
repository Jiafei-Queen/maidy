import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { deleteTodo, listTodoTargets, listTodos, updateTodo } from "../../api.js";
import { Button } from "../../components/ui/button.js";
import { ConfirmDialog } from "../../components/ui/dialog.js";
import { Frame, PageHeader } from "../../components/ui/frame.js";
import { Input, Select } from "../../components/ui/field.js";
import { QueryState } from "../../components/ui/query-state.js";
import { showToast } from "../../stores/toast.js";
import type { TodoStatus } from "../../types.js";
import { TodoCard } from "./todo-card.js";
import { TodoCreateDialog } from "./todo-create-dialog.js";
import { TodoEditDialog } from "./todo-edit-dialog.js";

const TARGET_PAGE_SIZE = 100;

/** Todo 列表过滤条件；空串与 "all" 表示不参与查询参数。 */
type TodoFilters = {
  status: "all" | TodoStatus;
  keyword: string;
  timeFilter: string;
  recurring: string;
  targetRef: string;
  platform: string;
  accountId: string;
  userId: string;
  scopeType: string;
  dateStart: string;
  dateEnd: string;
};

const DEFAULT_FILTERS: TodoFilters = {
  status: "all",
  keyword: "",
  timeFilter: "all",
  recurring: "all",
  targetRef: "",
  platform: "",
  accountId: "",
  userId: "",
  scopeType: "",
  dateStart: "",
  dateEnd: "",
};

function buildListParams(filters: TodoFilters, page: number): Record<string, unknown> {
  return {
    page,
    ...(filters.status === "all" ? {} : { status: filters.status }),
    ...(filters.keyword.trim() ? { keyword: filters.keyword.trim() } : {}),
    ...(filters.timeFilter === "all" ? {} : { time_filter: filters.timeFilter }),
    ...(filters.recurring === "all" ? {} : { recurring: filters.recurring === "true" }),
    ...(filters.targetRef ? { target_ref: filters.targetRef } : {}),
    ...(filters.platform.trim() ? { platform: filters.platform.trim() } : {}),
    ...(filters.accountId.trim() ? { account_id: filters.accountId.trim() } : {}),
    ...(filters.userId.trim() ? { user_id: filters.userId.trim() } : {}),
    ...(filters.scopeType ? { scope_type: filters.scopeType } : {}),
    ...(filters.dateStart && filters.dateEnd ? { date_start: filters.dateStart, date_end: filters.dateEnd } : {}),
  };
}

/** Todo 页：任务清单、过滤、分页与受控写入。 */
export function TodoPage() {
  const queryClient = useQueryClient();
  const [filters, setFilters] = useState<TodoFilters>(DEFAULT_FILTERS);
  const [applied, setApplied] = useState<TodoFilters>(DEFAULT_FILTERS);
  const [page, setPage] = useState(1);
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [deleting, setDeleting] = useState<{ id: string; title: string } | null>(null);
  const [statusMessage, setStatusMessage] = useState<{ error: boolean; text: string } | null>(null);

  const listQuery = useQuery({
    queryKey: ["todos", page, applied],
    queryFn: () => listTodos(buildListParams(applied, page)),
  });

  useEffect(() => {
    const result = listQuery.data;
    if (result && page > result.totalPages && page > 1) {
      setPage(Math.max(1, result.totalPages));
    }
  }, [listQuery.data, page]);

  const targetsQuery = useInfiniteQuery({
    queryKey: ["todo-targets"],
    queryFn: ({ pageParam }) => listTodoTargets(pageParam, TARGET_PAGE_SIZE),
    initialPageParam: 1,
    getNextPageParam: (last) => (last.page < last.totalPages ? last.page + 1 : undefined),
    staleTime: 60_000,
  });
  const targets = targetsQuery.data?.pages.flatMap((result) => result.items) ?? [];

  const invalidate = () => void queryClient.invalidateQueries({ queryKey: ["todos"] });

  const statusMutation = useMutation({
    mutationFn: ({ id, status }: { id: string; status: TodoStatus }) => updateTodo(id, { status }),
    onSuccess: () => {
      invalidate();
      setStatusMessage(null);
    },
    onError: (cause) => setStatusMessage({ error: true, text: cause instanceof Error ? cause.message : "Todo 更新失败" }),
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => deleteTodo(id),
    onSuccess: () => {
      invalidate();
      setDeleting(null);
      showToast("success", "Todo 已删除");
    },
    onError: (cause) => {
      setDeleting(null);
      setStatusMessage({ error: true, text: cause instanceof Error ? cause.message : "Todo 删除失败" });
    },
  });

  const advancedActive = [
    filters.timeFilter, filters.recurring, filters.targetRef, filters.platform,
    filters.accountId, filters.userId, filters.scopeType, filters.dateStart, filters.dateEnd,
  ].some((value) => value !== "" && value !== "all");

  const applyFilters = () => {
    setPage(1);
    setApplied(filters);
  };

  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        title="Todo 管理"
        subtitle="查看、创建和更新当前运行目录中的待办事项，目标与提醒状态来自管理 API"
        metrics={listQuery.data ? [{ label: "当前匹配", value: listQuery.data.total, unit: "项" }] : undefined}
        actions={
          <>
            <Button variant="secondary" size="sm" onClick={() => void listQuery.refetch()} disabled={listQuery.isFetching}>
              刷新
            </Button>
            <Button size="sm" onClick={() => setCreateOpen(true)}>
              创建 Todo
            </Button>
          </>
        }
      />

      <section aria-label="任务清单" className="flex flex-col gap-4">
        <Frame variant="panel" className="flex flex-col gap-3 p-3.5 sm:p-4">
          <div className="flex flex-wrap items-end gap-2.5 [&>*]:min-w-0">
            <label className="flex flex-col gap-1 text-xs font-medium text-muted">
              状态
              <Select
                value={filters.status}
                onChange={(event) => setFilters({ ...filters, status: event.target.value as TodoFilters["status"] })}
              >
                <option value="all">全部</option>
                <option value="pending">待处理</option>
                <option value="completed">已完成</option>
              </Select>
            </label>
            <label className="flex flex-1 flex-col gap-1 text-xs font-medium text-muted sm:max-w-64">
              关键词
              <Input
                type="search"
                placeholder="搜索标题或内容"
                value={filters.keyword}
                onChange={(event) => setFilters({ ...filters, keyword: event.target.value })}
              />
            </label>
            <Button variant="secondary" size="sm" onClick={() => setAdvancedOpen((open) => !open)} aria-expanded={advancedOpen}>
              {advancedOpen ? "收起高级筛选" : advancedActive ? "高级筛选 · 已启用" : "高级筛选"}
            </Button>
            <Button size="sm" onClick={applyFilters}>应用筛选</Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => {
                setFilters(DEFAULT_FILTERS);
                setApplied(DEFAULT_FILTERS);
                setPage(1);
              }}
            >
              重置
            </Button>
          </div>

          {advancedOpen ? (
            <div className="grid grid-cols-2 gap-3 border-t border-line-inner pt-3 md:grid-cols-3 lg:grid-cols-5">
              <FilterSelect
                label="时间"
                value={filters.timeFilter}
                onChange={(value) => setFilters({ ...filters, timeFilter: value })}
                options={[["all", "全部"], ["overdue", "已逾期"], ["no_due_date", "无截止日期"]]}
              />
              <FilterSelect
                label="重复"
                value={filters.recurring}
                onChange={(value) => setFilters({ ...filters, recurring: value })}
                options={[["all", "全部"], ["true", "仅重复"], ["false", "非重复"]]}
              />
              <FilterSelect
                label="目标"
                value={filters.targetRef}
                onChange={(value) => setFilters({ ...filters, targetRef: value })}
                options={[["", "全部目标"], ...targets.map((target) => [target.targetRef, `${target.platform} · ${target.scopeType}`] as const)]}
              />
              <FilterText label="平台" placeholder="qq_official" value={filters.platform} onChange={(value) => setFilters({ ...filters, platform: value })} />
              <FilterText label="账号" placeholder="可选" value={filters.accountId} onChange={(value) => setFilters({ ...filters, accountId: value })} />
              <FilterText label="用户/群组" placeholder="可选" value={filters.userId} onChange={(value) => setFilters({ ...filters, userId: value })} />
              <FilterSelect
                label="范围"
                value={filters.scopeType}
                onChange={(value) => setFilters({ ...filters, scopeType: value })}
                options={[["", "全部"], ["private", "私聊"], ["group", "群聊"]]}
              />
              <FilterDate label="日期起" value={filters.dateStart} onChange={(value) => setFilters({ ...filters, dateStart: value })} />
              <FilterDate label="日期止" value={filters.dateEnd} onChange={(value) => setFilters({ ...filters, dateEnd: value })} />
            </div>
          ) : null}
        </Frame>

        {statusMessage ? (
          <p aria-live="polite" role={statusMessage.error ? "alert" : "status"} className={`m-0 text-xs font-medium ${statusMessage.error ? "text-error" : "text-muted"}`}>
            {statusMessage.text}
          </p>
        ) : null}

        <div aria-live="polite" className="flex flex-col gap-2.5">
          <QueryState query={listQuery}>
            {listQuery.data && listQuery.data.items.length > 0 ? (
              listQuery.data.items.map((todo) => (
                <TodoCard
                  key={todo.id}
                  todo={todo}
                  busy={statusMutation.isPending || deleteMutation.isPending}
                  onChangeStatus={(item, status) => statusMutation.mutate({ id: item.id, status })}
                  onEdit={(item) => setEditingId(item.id)}
                  onDelete={(item) => setDeleting({ id: item.id, title: item.title })}
                />
              ))
            ) : listQuery.data ? (
              <Frame variant="card" className="py-8 text-center text-sm text-muted">
                当前筛选没有 Todo。
              </Frame>
            ) : null}
          </QueryState>
        </div>

        {listQuery.data && listQuery.data.totalPages > 1 ? (
          <div className="flex items-center justify-center gap-4 text-xs text-muted">
            <Button variant="secondary" size="sm" disabled={page <= 1} onClick={() => setPage((current) => Math.max(1, current - 1))}>
              上一页
            </Button>
            <span className="font-mono">{listQuery.data.page} / {listQuery.data.totalPages}</span>
            <Button variant="secondary" size="sm" disabled={page >= listQuery.data.totalPages} onClick={() => setPage((current) => current + 1)}>
              下一页
            </Button>
          </div>
        ) : null}
      </section>

      <TodoCreateDialog
        open={createOpen}
        onOpenChange={setCreateOpen}
        targets={targets}
        targetsLoading={targetsQuery.isLoading}
        hasMoreTargets={targetsQuery.hasNextPage ?? false}
        onLoadMoreTargets={() => void targetsQuery.fetchNextPage()}
      />
      <TodoEditDialog todoId={editingId} onClose={() => setEditingId(null)} />
      <ConfirmDialog
        open={deleting !== null}
        onOpenChange={(open) => {
          if (!open) setDeleting(null);
        }}
        title="删除 Todo"
        description={deleting ? `确定删除 Todo「${deleting.title}」吗？` : ""}
        confirmLabel="删除"
        danger
        busy={deleteMutation.isPending}
        onConfirm={() => {
          if (deleting) deleteMutation.mutate(deleting.id);
        }}
      />
    </div>
  );
}

function FilterSelect({ label, value, onChange, options }: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  options: ReadonlyArray<readonly [string, string]>;
}) {
  return (
    <label className="flex flex-col gap-1 text-xs font-medium text-muted">
      {label}
      <Select value={value} onChange={(event) => onChange(event.target.value)}>
        {options.map(([optionValue, optionLabel]) => (
          <option key={optionValue} value={optionValue}>{optionLabel}</option>
        ))}
      </Select>
    </label>
  );
}

function FilterText({ label, value, onChange, placeholder }: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
}) {
  return (
    <label className="flex flex-col gap-1 text-xs font-medium text-muted">
      {label}
      <Input value={value} placeholder={placeholder} onChange={(event) => onChange(event.target.value)} />
    </label>
  );
}

function FilterDate({ label, value, onChange }: {
  label: string;
  value: string;
  onChange: (value: string) => void;
}) {
  return (
    <label className="flex flex-col gap-1 text-xs font-medium text-muted">
      {label}
      <Input type="date" value={value} onChange={(event) => onChange(event.target.value)} />
    </label>
  );
}
