import { useConsoleStatusQuery } from "../../queries/console-status.js";
import { DataTable } from "../../components/ui/data-table.js";
import { PageHeader, SectionHeading } from "../../components/ui/frame.js";
import { QueryState } from "../../components/ui/query-state.js";
import { StatusBadge, toneForState } from "../../components/ui/status-badge.js";
import { formatMarker, yesNoUnknown } from "../../lib/format.js";

/** 平台页：已接入平台状态表与收发消息能力矩阵。 */
export function PlatformsPage() {
  const statusQuery = useConsoleStatusQuery();
  const platforms = statusQuery.data?.platforms;
  const onlineCount = platforms ? platforms.filter((item) => item.state === "online").length : null;

  return (
    <div className="flex flex-col gap-7">
      <PageHeader
        title="平台与能力"
        subtitle="各协议入口的在线状态、最近事件与收发消息能力矩阵"
        metrics={
          platforms
            ? [
                {
                  label: "在线入口",
                  value: `${onlineCount} / ${platforms.length}`,
                  unit: "平台",
                },
              ]
            : undefined
        }
        actions={<span className="console-kbd">只读</span>}
      />

      <QueryState query={statusQuery}>
        {platforms ? (
          <div className="flex flex-col gap-8">
            <section aria-label="平台状态" className="flex flex-col gap-3">
              <SectionHeading title="平台状态" />
              <DataTable
                columns={["平台", "已配置", "已启用", "状态", "最近事件", "最近错误"]}
                rows={platforms.map((platform) => [
                  <strong key="label" className="font-mono font-medium text-ink">{platform.label}</strong>,
                  yesNoUnknown(platform.configured),
                  yesNoUnknown(platform.enabled),
                  <StatusBadge key="state" tone={toneForState(platform.state)} value={platform.state} />,
                  <span key="event" className="font-mono text-xs text-muted">{formatMarker(platform.lastEventAt)}</span>,
                  <span key="error" className={platform.lastErrorSummary ? "text-error" : "text-muted"}>
                    {platform.lastErrorSummary ?? "无"}
                  </span>,
                ])}
                empty="尚未接入任何平台"
              />
            </section>

            <section aria-label="能力矩阵" className="flex flex-col gap-3">
              <SectionHeading title="能力矩阵" description="按平台与会话范围列出的接收与发送消息类型支持度" />
              <DataTable
                columns={["平台", "范围", "方向", "文本", "Markdown", "图片", "文件", "混合", "流式"]}
                rows={platforms.flatMap((platform) =>
                  platform.capabilityScopes.flatMap((scope) =>
                    (["接收", "发送"] as const).map((direction) => {
                      const capabilities = direction === "接收" ? scope.capabilities.inbound : scope.capabilities.outbound;
                      return [
                        <strong key={`${platform.id}-label`} className="font-mono font-medium text-ink">{platform.label}</strong>,
                        scope.label,
                        direction,
                        ...(["text", "markdown", "image", "file", "mixedMessage", "streaming"] as const).map((key) => (
                          <StatusBadge key={`${platform.id}-${scope.label}-${direction}-${key}`} tone={toneForState(capabilities[key])} value={capabilities[key]} />
                        )),
                      ];
                    }),
                  ),
                )}
                empty="暂无能力数据"
              />
            </section>
          </div>
        ) : null}
      </QueryState>
    </div>
  );
}
