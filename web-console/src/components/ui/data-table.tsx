import type { ReactNode } from "react";
import { cn } from "../../lib/utils.js";

type DataTableProps = {
  columns: readonly string[];
  /** 每行单元格；空态时渲染 empty。 */
  rows: readonly (readonly ReactNode[])[];
  empty?: ReactNode;
  className?: string;
};

/**
 * 高密度数据表原语：禁止斑马纹，统一用 1px 发丝线与 hover 微弱底色区分行，
 * 启用 tabular-nums 保证数字列对齐。
 */
export function DataTable({ columns, rows, empty, className }: DataTableProps) {
  if (rows.length === 0 && empty !== undefined) {
    return (
      <div className={cn("console-paper px-4 py-8 text-center text-sm text-muted", className)}>
        {empty}
      </div>
    );
  }
  return (
    <div className={cn("console-paper overflow-x-auto", className)}>
      <table className="w-full border-collapse text-[13px] leading-[1.45] tabular-nums">
        <thead>
          <tr className="border-b border-line">
            {columns.map((column) => (
              <th
                key={column}
                scope="col"
                className="px-3 py-2.5 text-left text-xs font-semibold whitespace-nowrap text-muted"
              >
                {column}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, rowIndex) => (
            <tr
              key={rowIndex}
              className="border-b border-line-inner transition-colors duration-[120ms] ease-out last:border-b-0 hover:bg-accent-soft/60"
            >
              {row.map((cellValue, cellIndex) => (
                <td key={cellIndex} className="px-3 py-2.5 align-middle whitespace-nowrap text-ink">
                  {cellValue}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
