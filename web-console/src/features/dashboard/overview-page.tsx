import { useConsoleStatusQuery } from "../../queries/console-status.js";
import { Frame, PageHeader, SectionHeading } from "../../components/ui/frame.js";
import { QueryState } from "../../components/ui/query-state.js";
import { StatusBadge, toneForState } from "../../components/ui/status-badge.js";
import { formatDuration, formatMarker, stateLabel, yesNoUnknown } from "../../lib/format.js";

/** 总览：服务健康 + 运行指标 + 上游摘要的只读运行快照。 */
export function OverviewPage() {
  const statusQuery = useConsoleStatusQuery();
  const data = statusQuery.data;
  const onlinePlatforms = data ? data.platforms.filter((platform) => platform.state === "online").length : null;
  const totalPlatforms = data ? data.platforms.length : null;

  return (
    <div className="flex flex-col gap-7">
      <PageHeader
        title="总览"
        subtitle="本地运行状态、服务健康与模型上游链路实时观测"
        metrics={
          data
            ? [
                {
                  label: "运行时长",
                  value: formatDuration(data.runtime.uptimeSeconds),
                },
                {
                  label: "在线平台",
                  value: `${onlinePlatforms} / ${totalPlatforms}`,
                  unit: "个入口",
                },
              ]
            : undefined
        }
      />

      <QueryState query={statusQuery}>
        {data ? <OverviewContent status={data} /> : null}
      </QueryState>
    </div>
  );
}

function OverviewContent({ status }: { status: NonNullable<ReturnType<typeof useConsoleStatusQuery>["data"]> }) {
  const setupRequired = status.runtime.state === "setup_required";
  const healthTone = setupRequired ? "warning" : status.runtime.ok ? "success" : "error";
  const healthLabel = setupRequired ? "等待首次配置" : status.runtime.ok ? "运行正常" : "运行异常";

  return (
    <div className="flex flex-col gap-6">
      {/* 服务健康主卡片 */}
      <Frame variant="panel" className="flex flex-wrap items-center justify-between gap-4">
        <div className="min-w-0">
          <h2 className="m-0 font-display text-[20px] leading-[1.3] font-semibold text-ink">服务健康</h2>
          <p className="m-0 mt-1 text-xs text-muted">运行时状态与基础配置检查结果</p>
        </div>
        <div className="flex items-center gap-3">
          <span className="console-kbd">只读</span>
          <StatusBadge tone={healthTone} label={healthLabel} className="px-2.5 py-1 text-xs" />
        </div>
      </Frame>

      {/* 运行指标 */}
      <section aria-labelledby="runtime-metrics-heading" className="flex flex-col gap-3">
        <SectionHeading id="runtime-metrics-heading" title="运行指标" />
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-4">
          <MetricCard label="当前版本" value={status.runtime.version} serif />
          <MetricCard label="启动时间" value={formatMarker(status.runtime.startedAt)} />
          <MetricCard label="运行时长" value={formatDuration(status.runtime.uptimeSeconds)} serif />
          <MetricCard label="监听摘要" value={status.configuration.listen} />
        </div>
      </section>

      {/* 上游模型摘要 */}
      <section aria-labelledby="provider-summary-heading" className="flex flex-col gap-3">
        <SectionHeading
          id="provider-summary-heading"
          title="上游模型链路"
          aside={<span className="font-mono text-xs text-muted">状态：{stateLabel(status.provider.upstreamState)}</span>}
        />
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-4">
          <MetricCard label="Provider" value={status.provider.name} />
          <MetricCard label="当前模型" value={status.provider.model} />
          <MetricCard label="流式传输" value={yesNoUnknown(status.provider.streaming)} />
          <Frame variant="card" className="flex flex-col justify-between gap-2">
            <span className="text-xs text-muted">上游连接</span>
            <div className="flex flex-wrap items-center gap-2">
              <StatusBadge tone={toneForState(status.provider.upstreamState)} value={status.provider.upstreamState} />
            </div>
            <span className="font-mono text-[11px] text-muted">检查于 {formatMarker(status.provider.lastCheckedAt)}</span>
            {status.provider.errorSummary ? (
              <span className="font-mono text-xs break-all text-error">{status.provider.errorSummary}</span>
            ) : null}
          </Frame>
        </div>
      </section>
    </div>
  );
}

function MetricCard({ label, value, serif = false }: { label: string; value: string; serif?: boolean }) {
  return (
    <Frame variant="card" className="flex flex-col justify-between gap-2">
      <span className="text-xs text-muted">{label}</span>
      <strong
        className={
          serif
            ? "font-display text-[24px] leading-[1.15] font-semibold break-all text-ink"
            : "font-mono text-[13px] leading-snug font-semibold break-all text-ink"
        }
      >
        {value}
      </strong>
    </Frame>
  );
}
