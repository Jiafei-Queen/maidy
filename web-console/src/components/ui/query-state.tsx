import { useQueryClient, type UseQueryResult } from "@tanstack/react-query";
import { Button } from "./button.js";

type QueryStateProps = {
  query: UseQueryResult;
  children: React.ReactNode;
};

/** 查询三态包装：加载中 / 错误（含重试）/ 正常渲染子内容。 */
export function QueryState({ query, children }: QueryStateProps) {
  if (query.isPending) {
    return (
      <p className="console-meta m-0 px-1 py-8" role="status">
        正在加载运行状态…
      </p>
    );
  }
  if (query.isError) {
    return (
      <div className="console-paper mt-4 flex flex-col items-start gap-3 p-5">
        <p role="alert" className="m-0 text-sm font-medium text-error">
          {query.error instanceof Error ? query.error.message : "状态刷新失败"}
        </p>
        <Button variant="secondary" size="sm" onClick={() => void query.refetch()}>
          重试
        </Button>
      </div>
    );
  }
  return <>{children}</>;
}

/** 登出或会话切换后手动失效运行状态缓存时使用。 */
export function useInvalidateConsoleStatus() {
  const queryClient = useQueryClient();
  return () => void queryClient.invalidateQueries({ queryKey: ["console-status"] });
}
