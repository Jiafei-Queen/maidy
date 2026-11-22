import { useConsoleStatusQuery } from "../../queries/console-status.js";
import { DataTable } from "../../components/ui/data-table.js";
import { PageHeader, SectionHeading } from "../../components/ui/frame.js";
import { QueryState } from "../../components/ui/query-state.js";
import { StatusBadge, toneForState } from "../../components/ui/status-badge.js";
import { stateLabel, yesNoUnknown } from "../../lib/format.js";

/** 存储页：数据库、缓存、附件等资源行的健康清单。 */
export function StoragePage() {
  const statusQuery = useConsoleStatusQuery();
  const storage = statusQuery.data?.storage;
  const healthyCount = storage ? storage.filter((item) => toneForState(item.state) === "success").length : null;

  return (
    <div className="flex flex-col gap-7">
      <PageHeader
        title="存储与资源"
        subtitle="路径可达性、读写权限与数据库 Schema 迁移状态"
        metrics={
          storage
            ? [
                {
                  label: "健康资源",
                  value: `${healthyCount} / ${storage.length}`,
                  unit: "项",
                },
              ]
            : undefined
        }
        actions={<span className="console-kbd">只读</span>}
      />

      <QueryState query={statusQuery}>
        {storage ? (
          <section aria-label="存储资源清单" className="flex flex-col gap-3">
            <SectionHeading title="资源健康清单" />
            <DataTable
              columns={["资源", "路径", "状态", "存在", "可读", "可写", "错误", "Schema"]}
              rows={storage.map((item) => [
                <strong key="label" className="font-mono font-medium text-ink">{item.label}</strong>,
                <span key="path" className="font-mono text-xs text-muted">{item.pathSummary}</span>,
                <StatusBadge key="state" tone={toneForState(item.state)} value={item.state} />,
                yesNoUnknown(item.exists),
                yesNoUnknown(item.readable),
                yesNoUnknown(item.writable),
                <span key="error" className={item.errorSummary ? "text-warning" : "text-muted"}>
                  {item.errorSummary ? stateLabel(item.errorSummary) : "无"}
                </span>,
                <span key="schema" className="font-mono text-xs text-muted">{item.schemaSummary ?? "不适用"}</span>,
              ])}
              empty="暂无存储资源信息"
            />
          </section>
        ) : null}
      </QueryState>
    </div>
  );
}
