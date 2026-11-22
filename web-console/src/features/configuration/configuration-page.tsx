import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import {
  ConsoleApiError,
  fetchConfiguration,
  requestRestart,
  updateRuntimeConfiguration,
  updateSecretConfiguration,
  validateConfiguration,
} from "../../api.js";
import { Button } from "../../components/ui/button.js";
import { ConfirmDialog } from "../../components/ui/dialog.js";
import { Frame, PageHeader, SectionHeading } from "../../components/ui/frame.js";
import { Field, Input } from "../../components/ui/field.js";
import { QueryState } from "../../components/ui/query-state.js";
import { StatusBadge } from "../../components/ui/status-badge.js";
import { showToast } from "../../stores/toast.js";
import { cn } from "../../lib/utils.js";
import type { ConfigurationSnapshot } from "../../types.js";
import {
  BUSINESS_GROUPS,
  businessGroupOf,
  configFieldLabel,
  groupFieldsBySection,
  type ConfigurationBusinessGroup,
} from "./configuration-navigation.js";
import { configurationSummary, isEmptyInputValue, parseConfigInputValue } from "./configuration-values.js";
import { FirstRunChecklist } from "./first-run-checklist.js";
import { PublicFieldRow } from "./configuration-field-row.js";
import { ProviderConnections } from "./provider-connections.js";
import { AgentEditor } from "./agent-editor.js";
import { ThemePreferencesSection } from "./theme-preferences.js";
import { BackgroundPreferencesSection } from "./background-preferences.js";
import { VoicePreviewCard } from "./voice-preview-card.js";

/** Agent 编辑器覆盖的业务域：这些分组由 snapshot.agent 驱动，不依赖 runtime 字段存在。 */
const AGENT_GROUPS: ReadonlySet<ConfigurationBusinessGroup> = new Set([
  "model-routing",
  "stt-routing",
  "online-tools",
  "memory-knowledge",
]);

/** 配置中心（§7.3 左右分栏重构）：
 * 左侧 196px L1 通透玻璃粘性目录 + 右侧 max-w-[760px] 单列表单工作区 + 仅在有脏数据时浮出的保存条。
 * 契约：只提交真实变更、secret 留空不修改、清除必须显式确认、revision 冲突时保留本地草稿并提示比较。 */
export function ConfigurationPage() {
  const queryClient = useQueryClient();
  const snapshotQuery = useQuery({ queryKey: ["configuration"], queryFn: fetchConfiguration, staleTime: 0 });
  const [activeGroup, setActiveGroup] = useState<ConfigurationBusinessGroup>("models-providers");
  // draft 键为字段 key；值 undefined 表示“用户未触碰”，不参与变更收集。
  const [publicDraft, setPublicDraft] = useState<Record<string, string>>({});
  const [secretDraft, setSecretDraft] = useState<Record<string, { value: string; clear: boolean }>>({});
  const [result, setResult] = useState<{ error: boolean; text: string } | null>(null);
  const [clearingKey, setClearingKey] = useState<string | null>(null);

  const snapshot = snapshotQuery.data;
  const invalidate = () => void queryClient.invalidateQueries({ queryKey: ["configuration"] });
  // 切换业务域时清空上一次保存的结果提示，避免跨分组误导。
  const changeGroup = (group: ConfigurationBusinessGroup) => {
    setActiveGroup(group);
    setResult(null);
  };

  const runtimeSave = useMutation({
    mutationFn: ({ revision, changes }: { revision: string; changes: unknown[] }) => updateRuntimeConfiguration(revision, changes),
    onSuccess: (next) => {
      const pending = next.fields.filter((field) => field.pendingRestart).length;
      setPublicDraft({});
      setResult({
        error: false,
        text: pending > 0 ? `配置已真实持久化，${pending} 项需重启后生效` : "配置已保存并立即生效",
      });
      invalidate();
    },
    onError: (cause) => setResult({ error: true, text: saveErrorMessage(cause) }),
  });

  const secretSave = useMutation({
    mutationFn: ({ changes }: { changes: unknown[] }) => updateSecretConfiguration(changes),
    onSuccess: () => {
      setResult({ error: false, text: "密钥已保存，原文不会再次显示" });
      // 服务端不回传 secret 原文：保存成功后立即清空对应输入，恢复密文状态。
      setSecretDraft({});
      invalidate();
    },
    onError: (cause) => setResult({ error: true, text: saveErrorMessage(cause) }),
  });

  const validateMutation = useMutation({
    mutationFn: validateConfiguration,
    onSuccess: (res) => setResult({
      error: !res.valid,
      text: res.valid ? "配置校验通过，未执行外部网络请求" : res.message || "配置未通过启动预检，未保存任何变更",
    }),
    onError: (cause) => setResult({ error: true, text: cause instanceof Error ? cause.message : "配置校验失败" }),
  });

  const restartMutation = useMutation({
    mutationFn: requestRestart,
    onSuccess: (message) => {
      showToast("info", message || "重启请求已提交，服务会短暂离线");
      setResult({ error: false, text: "重启请求已提交，服务会短暂离线" });
    },
    onError: (cause) => setResult({ error: true, text: cause instanceof Error ? cause.message : "重启请求失败" }),
  });

  const availableGroups = useMemo(() => {
    const present = new Set((snapshot?.fields ?? []).map((field) => businessGroupOf(field.key)));
    // “模型路由”等分组承载 agent.toml 编辑器而非 runtime 字段；agent 可用时必须始终出现。
    if (snapshot?.agent?.fileExists) {
      for (const group of AGENT_GROUPS) present.add(group);
    }
    return BUSINESS_GROUPS.filter((group) => present.has(group.id));
  }, [snapshot]);

  const publicFields = useMemo(
    () => (snapshot?.fields ?? []).filter((field) => field.sensitivity !== "secret"),
    [snapshot],
  );
  const secretFields = useMemo(
    () => (snapshot?.fields ?? []).filter((field) => field.sensitivity === "secret"),
    [snapshot],
  );
  // provider.* 公开字段由供应商连接卡片承载（含内置连接卡片），避免同页重复渲染。
  const providerFields = useMemo(
    () => (snapshot?.fields ?? []).filter((field) => field.key.startsWith("provider.")),
    [snapshot],
  );
  // provider.* secret（内置连接 API Key）内联到供应商详情面板；其余 secret 仍走页面级「密钥凭据」分节。
  const providerSecrets = useMemo(
    () => secretFields.filter((field) => field.key.startsWith("provider.")),
    [secretFields],
  );

  // 统计当前已修改的普通字段与密钥草稿数，驱动底部未保存确认条显隐。
  const dirtyPublicCount = useMemo(() => {
    if (!snapshot) return 0;
    let count = 0;
    for (const field of publicFields) {
      if (field.sensitivity !== "public" || !field.editable || !(field.key in publicDraft)) continue;
      const raw = publicDraft[field.key] ?? "";
      const baseline = field.savedValue ?? field.effectiveValue;
      if ((baseline === null || baseline === undefined) && isEmptyInputValue(raw)) continue;
      try {
        if (JSON.stringify(parseConfigInputValue(field, raw)) !== JSON.stringify(baseline)) count += 1;
      } catch {
        count += 1;
      }
    }
    return count;
  }, [publicDraft, publicFields, snapshot]);

  const dirtySecretCount = useMemo(
    () => Object.values(secretDraft).filter((draft) => draft.clear || draft.value.length > 0).length,
    [secretDraft],
  );

  const totalDirtyCount = dirtyPublicCount + dirtySecretCount;

  if (!snapshot) {
    return (
      <div className="flex flex-col gap-6">
        <PageHeader title="配置中心" subtitle="按业务域查看与修改运行配置与凭据" />
        <QueryState query={snapshotQuery}>{null}</QueryState>
      </div>
    );
  }

  const summary = configurationSummary(snapshot);
  const activeGroupMeta = BUSINESS_GROUPS.find((group) => group.id === activeGroup);
  const activePublic = publicFields.filter(
    (field) => businessGroupOf(field.key) === activeGroup && !(activeGroup === "models-providers" && field.key.startsWith("provider.")),
  );
  const activeSecret = secretFields.filter(
    (field) => businessGroupOf(field.key) === activeGroup && !(activeGroup === "models-providers" && field.key.startsWith("provider.")),
  );
  const activeAgent =
    activeGroup === "models-providers"
    || activeGroup === "online-tools"
    || activeGroup === "memory-knowledge"
    || activeGroup === "model-routing"
    || activeGroup === "stt-routing";

  const savePublic = () => {
    const changes: Array<Record<string, unknown>> = [];
    for (const field of publicFields.filter((candidate) => candidate.sensitivity === "public" && candidate.editable)) {
      if (!(field.key in publicDraft)) continue;
      const raw = publicDraft[field.key] ?? "";
      const baseline = field.savedValue ?? field.effectiveValue;
      if ((baseline === null || baseline === undefined) && isEmptyInputValue(raw)) continue;
      let value: unknown;
      try {
        value = parseConfigInputValue(field, raw);
      } catch (cause) {
        setResult({ error: true, text: cause instanceof Error ? cause.message : "字段格式不正确" });
        return;
      }
      if (JSON.stringify(value) !== JSON.stringify(baseline)) {
        changes.push({ action: "set", key: field.key, value });
      }
    }
    if (changes.length === 0) {
      setResult({ error: false, text: "没有需要保存的普通配置。" });
      return;
    }
    runtimeSave.mutate({ revision: snapshot.revision, changes });
  };

  const saveSecrets = () => {
    const changes: Array<Record<string, unknown>> = [];
    for (const field of secretFields.filter((candidate) => candidate.sensitivity === "secret" && candidate.editable)) {
      const draft = secretDraft[field.key];
      if (!draft) continue;
      if (draft.clear) {
        changes.push({ action: "clear", key: field.key, expected_revision: field.revision ?? "missing" });
      } else if (draft.value.length > 0) {
        changes.push({ action: "replace", key: field.key, value: draft.value, expected_revision: field.revision ?? "missing" });
      }
    }
    if (changes.length === 0) {
      setResult({ error: false, text: "没有需要保存的密钥变更。" });
      return;
    }
    secretSave.mutate({ changes });
  };

  const resetDrafts = () => {
    setPublicDraft({});
    setSecretDraft({});
    setResult(null);
  };

  return (
    <div className="flex flex-col gap-6 pb-20">
      <PageHeader
        title="配置中心"
        subtitle="按业务域查看与修改运行配置；保存以服务端返回的 revision 与真实持久化结果为准"
        metrics={[{ label: "待重启项", value: summary.pendingCount, unit: "项" }]}
        actions={
          <>
            <Button variant="secondary" size="sm" onClick={() => validateMutation.mutate()} disabled={validateMutation.isPending}>
              {validateMutation.isPending ? "校验中…" : "校验配置"}
            </Button>
            <Button
              variant="danger"
              size="sm"
              disabled={!snapshot.restartAvailable || restartMutation.isPending}
              title={snapshot.restartAvailable ? undefined : "当前运行目录没有可用的 botctl 重启脚本"}
              onClick={() => restartMutation.mutate()}
            >
              {restartMutation.isPending ? "提交中…" : "重启服务"}
            </Button>
          </>
        }
      />

      {/* 配置摘要徽章 */}
      <div className="flex flex-wrap items-center gap-2" aria-label="配置摘要">
        <StatusBadge tone={snapshot.fileExists ? "success" : "warning"} label={snapshot.fileExists ? "runtime.toml 已建立" : "runtime.toml 尚未建立"} />
        <StatusBadge tone={summary.invalidCount === 0 ? "success" : "warning"} label={summary.invalidCount === 0 ? "本地预检通过" : "需要完成配置"} />
        <StatusBadge tone={summary.pendingCount === 0 ? "neutral" : "warning"} label={summary.pendingCount === 0 ? "无待重启变更" : `${summary.pendingCount} 项重启后生效`} />
        {!snapshot.restartAvailable ? (
          <span className="text-xs text-muted">当前运行目录没有可用的 botctl 重启脚本</span>
        ) : null}
      </div>

      <QueryState query={snapshotQuery}>
        <div className="grid grid-cols-1 items-start gap-5 lg:grid-cols-[240px_minmax(0,1fr)]">
          {/* 左侧 240px 通高粘性玻璃侧栏：撑满视口剩余高度，上方为配置目录，底部锚定首次配置检查单 */}
          <aside className="console-shell flex flex-col overflow-hidden rounded-console-lg border border-line lg:sticky lg:top-16 lg:h-[calc(100dvh-5rem)]">
            <nav
              aria-label="配置业务域"
              className="flex flex-row gap-1 overflow-x-auto p-2 lg:flex-1 lg:flex-col lg:overflow-x-visible lg:overflow-y-auto"
            >
              {availableGroups.map((group) => {
                const selected = activeGroup === group.id;
                return (
                  <button
                    key={group.id}
                    type="button"
                    role="tab"
                    aria-selected={selected}
                    title={group.description}
                    onClick={() => changeGroup(group.id)}
                    className={cn(
                      "relative shrink-0 cursor-pointer rounded-console-sm px-3 py-2 text-left text-[13px] font-semibold whitespace-nowrap transition-colors duration-[120ms] ease-out",
                      selected
                        ? "bg-accent-soft text-ink"
                        : "text-muted hover:bg-accent-soft/60 hover:text-ink",
                    )}
                  >
                    {selected ? (
                      <span
                        aria-hidden="true"
                        className="absolute inset-x-2 bottom-0 h-[2px] rounded-full bg-accent lg:inset-x-auto lg:inset-y-1.5 lg:left-0 lg:h-auto lg:w-[2px]"
                      />
                    ) : null}
                    {group.label}
                  </button>
                );
              })}
            </nav>

            <div className="shrink-0 lg:max-h-[52%] lg:overflow-y-auto">
              <FirstRunChecklist invalidCount={summary.invalidCount} onSelectGroup={changeGroup} embedded />
            </div>
          </aside>

          {/* 右侧主表单工作区：完整利用剩余宽度，保持左右视觉平衡 */}
          <div className="flex min-w-0 flex-col gap-6">
            {activeGroupMeta ? (
              <SectionHeading title={activeGroupMeta.label} description={activeGroupMeta.description} />
            ) : null}

            {result ? (
              <div
                aria-live="polite"
                role={result.error ? "alert" : "status"}
                className={cn(
                  "rounded-console border px-3.5 py-2.5 text-[13px] font-medium",
                  result.error
                    ? "border-error/40 bg-error-soft text-error"
                    : "border-success/40 bg-success-soft text-success",
                )}
              >
                {result.text}
              </div>
            ) : null}

            {activeGroup === "models-providers" ? (
              <ProviderConnections
                snapshot={snapshot}
                fields={providerFields}
                publicDraft={publicDraft}
                onDraftChange={(key, value) => setPublicDraft((current) => ({ ...current, [key]: value }))}
                onResult={setResult}
                secretFields={providerSecrets}
                secretDraft={secretDraft}
                onSecretDraftChange={(key, draft) => setSecretDraft((current) => ({ ...current, [key]: draft }))}
                onRequestSecretClear={setClearingKey}
              />
            ) : null}

            {groupFieldsBySection(activePublic).map((section) => (
              <section key={section.label} aria-label={section.label} className="flex flex-col gap-2.5">
                <div>
                  <h3 className="m-0 text-sm font-semibold text-ink">{section.label}</h3>
                  {section.description ? (
                    <p className="m-0 mt-0.5 text-xs leading-relaxed text-muted">{section.description}</p>
                  ) : null}
                </div>
                <Frame variant="card" className="divide-y divide-line-inner py-0.5">
                  {section.fields.map((field) => (
                    <PublicFieldRow
                      key={field.key}
                      field={field}
                      value={publicDraft[field.key]}
                      onChange={(value) => setPublicDraft((current) => ({ ...current, [field.key]: value }))}
                      onRemove={() => {
                        runtimeSave.mutate({ revision: snapshot.revision, changes: [{ action: "remove", key: field.key }] });
                      }}
                      busy={runtimeSave.isPending}
                    />
                  ))}
                </Frame>
                {section.label === "语音回复" ? (
                  <VoicePreviewCard fields={snapshot.fields.filter((field) => field.key.startsWith("delivery.tts."))} />
                ) : null}
              </section>
            ))}

            {activeSecret.length > 0 ? (
              <section aria-label="密钥凭据" className="flex flex-col gap-2.5">
                <h3 className="m-0 text-sm font-semibold text-ink">密钥凭据</h3>
                <Frame variant="card" className="divide-y divide-line-inner py-0.5">
                  {activeSecret.map((field) => {
                    const draft = secretDraft[field.key];
                    return (
                      <div key={field.key} className="py-1">
                        <Field
                          layout="row"
                          label={configFieldLabel(field.key)}
                          id={`secret-${field.key}`}
                          hint={field.configured ? "已配置；留空表示不修改" : "尚未配置，输入后添加"}
                          aside={
                            <div className="flex flex-wrap items-center justify-between gap-2 pt-0.5">
                              <label className="flex items-center gap-1.5 text-xs text-muted">
                                <input
                                  type="checkbox"
                                  disabled={!field.editable || !field.configured}
                                  checked={draft?.clear ?? false}
                                  onChange={(event) =>
                                    setSecretDraft((current) => ({
                                      ...current,
                                      [field.key]: {
                                        value: event.target.checked ? "" : current[field.key]?.value ?? "",
                                        clear: event.target.checked,
                                      },
                                    }))
                                  }
                                  className="size-3.5 accent-[var(--console-error)]"
                                />
                                显式清除密钥
                              </label>
                              {field.configured && field.editable ? (
                                <Button variant="ghost" size="sm" onClick={() => setClearingKey(field.key)}>
                                  清除…
                                </Button>
                              ) : null}
                            </div>
                          }
                        >
                          {(props) => (
                            <Input
                              {...props}
                              type="password"
                              autoComplete="new-password"
                              placeholder={field.configured ? "已配置；留空表示不修改" : "尚未配置"}
                              disabled={!field.editable}
                              value={draft?.value ?? ""}
                              onChange={(event) =>
                                setSecretDraft((current) => ({
                                  ...current,
                                  [field.key]: { value: event.target.value, clear: false },
                                }))
                              }
                            />
                          )}
                        </Field>
                      </div>
                    );
                  })}
                </Frame>
              </section>
            ) : null}

            {activeAgent && snapshot.agent ? (
              <div className="flex flex-col gap-5">
                <AgentStatusCard agent={snapshot.agent} />
                <AgentEditor snapshot={snapshot} group={activeGroup} />
              </div>
            ) : null}

            {activeGroup === "system-security" ? (
              <div className="flex flex-col gap-5">
                <ThemePreferencesSection />
                <BackgroundPreferencesSection />
              </div>
            ) : null}

            {/* 底部悬浮保存条：仅在存在未保存更改时滑出（L1 玻璃态确认条） */}
            {totalDirtyCount > 0 ? (
              <div
                role="region"
                aria-label="未保存的配置更改"
                className="console-shell sticky bottom-4 z-20 flex flex-wrap items-center justify-between gap-3 rounded-console-lg border border-line px-4 py-3 shadow-console animate-fade-in"
              >
                <span className="text-[13px] font-medium text-ink">
                  有 <strong className="font-mono text-warmth">{totalDirtyCount}</strong> 项未保存更改
                </span>
                <div className="flex flex-wrap items-center gap-2">
                  <Button variant="ghost" size="sm" onClick={resetDrafts} disabled={runtimeSave.isPending || secretSave.isPending}>
                    重置
                  </Button>
                  {dirtyPublicCount > 0 ? (
                    <Button size="sm" onClick={savePublic} disabled={runtimeSave.isPending}>
                      {runtimeSave.isPending ? "保存中…" : "保存普通配置"}
                    </Button>
                  ) : null}
                  {dirtySecretCount > 0 ? (
                    <Button size="sm" onClick={saveSecrets} disabled={secretSave.isPending}>
                      {secretSave.isPending ? "保存中…" : "保存密钥变更"}
                    </Button>
                  ) : null}
                </div>
              </div>
            ) : null}
          </div>
        </div>
      </QueryState>

      <ConfirmDialog
        open={clearingKey !== null}
        onOpenChange={(open) => {
          if (!open) setClearingKey(null);
        }}
        title="清除密钥"
        description="确认清除这个密钥吗？清除后依赖它的功能可能无法使用。"
        confirmLabel="清除"
        danger
        onConfirm={() => {
          if (clearingKey === null) return;
          setSecretDraft((current) => ({ ...current, [clearingKey]: { value: "", clear: true } }));
          setClearingKey(null);
        }}
      />
    </div>
  );
}

/** Agent 策略的保存状态摘要：revision、来源与待重启标记；编辑能力见 agent-editor.tsx。 */
function AgentStatusCard({ agent }: { agent: NonNullable<ConfigurationSnapshot["agent"]> }) {
  return (
    <Frame variant="panel" aria-label="Agent 策略状态" className="py-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="m-0 text-sm font-semibold text-ink">Agent 策略状态</h3>
        {agent.pendingRestart ? <StatusBadge tone="warning" label="等待重启生效" /> : null}
      </div>
      {!agent.fileExists ? (
        <p className="m-0 mt-2 text-xs text-warning">Agent 策略文件尚不可用；请检查默认 config/agent.toml 是否可写。</p>
      ) : (
        <div className="mt-2 flex flex-wrap gap-x-6 gap-y-1 text-xs text-muted">
          <span>
            Revision：<span className="font-mono text-ink">{agent.revision}</span>
          </span>
          <span>
            来源：<span className="font-mono text-ink">{agent.source}</span>
            {agent.editable ? "" : " · 只读"}
          </span>
        </div>
      )}
    </Frame>
  );
}

function saveErrorMessage(cause: unknown): string {
  if (cause instanceof ConsoleApiError && (cause.code === "config_conflict" || cause.status === 409)) {
    return "配置已被其他操作修改，未覆盖服务器版本。请刷新后比较本地修改和服务器当前值；本地输入已保留。";
  }
  return cause instanceof Error ? cause.message : "配置保存失败";
}
