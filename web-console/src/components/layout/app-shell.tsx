import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link, Outlet, useNavigate, useRouterState } from "@tanstack/react-router";
import { useAtom, useAtomValue } from "jotai";
import { useEffect, useState } from "react";
import { requestRestart } from "../../api.js";
import { hydrateUserData } from "../../stores/user-data.js";
import { autoRefreshAtom, useConsoleStatusQuery } from "../../queries/console-status.js";
import { logout, sessionAtom } from "../../stores/auth.js";
import { themeModeAtom, toggleThemeMode } from "../../stores/theme.js";
import { showToast } from "../../stores/toast.js";
import { cn } from "../../lib/utils.js";
import { Button } from "../ui/button.js";
import { ConfirmDialog } from "../ui/dialog.js";
import { ConsoleIcon, type ConsoleIconName } from "../ui/icons.js";
import { StatusBadge } from "../ui/status-badge.js";

type NavPage = {
  readonly id: string;
  readonly label: string;
  readonly icon: ConsoleIconName;
  readonly to: string;
};

/** 系统运维分组：运行态势与部署设置。 */
const OPS_PAGES: readonly NavPage[] = [
  { id: "overview", label: "总览", icon: "overview", to: "/" },
  { id: "platforms", label: "平台", icon: "platforms", to: "/platforms" },
  { id: "configuration", label: "配置", icon: "configuration", to: "/configuration" },
  { id: "storage", label: "存储", icon: "storage", to: "/storage" },
];

/** 核心能力分组：记忆、待办、知识库与本地工具。 */
const CAPABILITY_PAGES: readonly NavPage[] = [
  { id: "memory", label: "Memory", icon: "memory", to: "/memory" },
  { id: "todo", label: "Todo", icon: "todo", to: "/todo" },
  { id: "knowledge", label: "知识库", icon: "knowledge", to: "/knowledge" },
  { id: "tools", label: "工具", icon: "tools", to: "/tools" },
];

/**
 * 已认证的应用壳（Atelier Glass 工作台骨架）：
 * - 桌面端（>=1024px）：左侧固定 220px L1 通透玻璃侧边栏；
 * - 平板端（768px–1023px）：折叠为 60px 图标轨；
 * - 移动端（<768px）：顶部菜单按钮唤起玻璃抽屉；
 * - 路由切换零阻塞，内容区按路由 key 执行 120ms 静默淡入。
 */
export function AppShell() {
  const [restartOpen, setRestartOpen] = useState(false);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const pathname = useRouterState({ select: (state) => state.location.pathname });

  // 认证完成后加载用户界面偏好：主题自定义色、背景状态与旧 cookie 迁移以服务端为权威。
  useEffect(() => {
    void hydrateUserData();
  }, []);

  // 移动端抽屉打开时支持 Esc 关闭。
  useEffect(() => {
    if (!drawerOpen) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setDrawerOpen(false);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [drawerOpen]);

  return (
    <div className="relative z-1 flex min-h-dvh">
      {/* 桌面端 220px / 平板端 60px 图标轨：固定 L1 通透玻璃侧边栏 */}
      <aside
        aria-label="主导航栏"
        className="console-shell fixed inset-y-0 left-0 z-30 hidden border-r border-line md:flex md:w-[60px] md:flex-col lg:w-[220px]"
      >
        <SidebarContent pathname={pathname} collapsedTablet />
      </aside>

      {/* 移动端玻璃抽屉 */}
      {drawerOpen ? (
        <div className="fixed inset-0 z-40 md:hidden">
          <div
            aria-hidden="true"
            onClick={() => setDrawerOpen(false)}
            className="fixed inset-0 bg-scrim backdrop-blur-[2px] animate-fade-in"
          />
          <aside
            aria-label="主导航抽屉"
            className="console-shell relative z-1 flex h-full w-[240px] flex-col border-r border-line shadow-console animate-page-in"
          >
            <div className="flex h-12 items-center justify-end border-b border-line-inner px-3">
              <button
                type="button"
                aria-label="关闭导航菜单"
                onClick={() => setDrawerOpen(false)}
                className="inline-flex size-8 cursor-pointer items-center justify-center rounded-console-sm text-muted transition-colors duration-[120ms] hover:bg-accent-soft hover:text-ink"
              >
                <ConsoleIcon name="close" className="size-4" />
              </button>
            </div>
            <SidebarContent pathname={pathname} onNavigate={() => setDrawerOpen(false)} />
          </aside>
        </div>
      ) : null}

      {/* 右侧主工作区：顶部 48px 状态操作条 + 独立内容区 */}
      <div className="flex min-w-0 flex-1 flex-col md:pl-[60px] lg:pl-[220px]">
        <TopUtilityBar
          onOpenMenu={() => setDrawerOpen(true)}
          onRequestRestart={() => setRestartOpen(true)}
        />
        <main className="mx-auto w-full max-w-[1400px] flex-1 px-4 py-6 sm:px-6 lg:px-8">
          <div key={pathname} className="animate-page-in">
            <Outlet />
          </div>
        </main>
      </div>

      <RestartConfirmDialog open={restartOpen} onOpenChange={setRestartOpen} />
    </div>
  );
}

function SidebarContent({
  pathname,
  collapsedTablet = false,
  onNavigate,
}: {
  pathname: string;
  collapsedTablet?: boolean;
  onNavigate?: () => void;
}) {
  const statusQuery = useConsoleStatusQuery();
  const mode = useAtomValue(themeModeAtom);
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  const logoutMutation = useMutation({
    mutationFn: logout,
    onSuccess: () => {
      // 退出登录后清理运行状态缓存，避免下一会话复用旧数据。
      queryClient.clear();
      void navigate({ to: "/" });
    },
    onError: (cause) => {
      showToast("error", cause instanceof Error ? cause.message : "退出请求失败，本地会话已清理。");
    },
  });

  const version = statusQuery.data?.runtime.version;
  const labelHideClass = collapsedTablet ? "md:hidden lg:inline" : "";

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {/* 品牌区 */}
      <div className={cn("flex h-12 shrink-0 items-center border-b border-line px-4", collapsedTablet && "md:justify-center md:px-0 lg:justify-start lg:px-4")}>
        <div className={cn("min-w-0", collapsedTablet && "md:hidden lg:block")}>
          <p className="m-0 font-display text-[17px] leading-tight font-semibold text-ink">QQ Maid Console</p>
          <p className="m-0 font-mono text-[11px] leading-tight text-muted">
            {version ? `v${version} · Local` : "Local Ops"}
          </p>
        </div>
        {collapsedTablet ? (
          <span className="hidden font-display text-lg font-semibold text-ink md:inline lg:hidden" title="QQ Maid Console">
            M
          </span>
        ) : null}
      </div>

      {/* 导航分组 */}
      <nav aria-label="控制台页面" className="flex flex-1 flex-col gap-3 overflow-y-auto px-2 py-3">
        <NavGroup pages={OPS_PAGES} pathname={pathname} collapsedTablet={collapsedTablet} onNavigate={onNavigate} />
        <div className="mx-2 border-t border-line" role="separator" />
        <NavGroup pages={CAPABILITY_PAGES} pathname={pathname} collapsedTablet={collapsedTablet} onNavigate={onNavigate} />
      </nav>

      {/* 底部常驻：昼夜模式切换 + 退出登录 */}
      <div className="shrink-0 border-t border-line p-2">
        <div className={cn("flex gap-1.5", collapsedTablet ? "md:flex-col lg:flex-row" : "flex-row")}>
          <button
            type="button"
            onClick={toggleThemeMode}
            title={mode === "dark" ? "切换到晨雾浅色态" : "切换到暮色深色态"}
            aria-label={mode === "dark" ? "切换到晨雾浅色态" : "切换到暮色深色态"}
            className={cn(
              "inline-flex h-8 flex-1 cursor-pointer items-center justify-center gap-1.5 rounded-console-sm border border-line bg-card/70 px-2 text-xs font-medium text-ink transition-colors duration-[120ms] ease-out hover:border-accent/40 hover:bg-accent-soft",
              collapsedTablet && "md:px-0 lg:px-2",
            )}
          >
            <ConsoleIcon name={mode === "dark" ? "sun" : "moon"} className="size-3.5 shrink-0" />
            <span className={labelHideClass}>{mode === "dark" ? "暮色" : "晨雾"}</span>
          </button>
          <button
            type="button"
            disabled={logoutMutation.isPending}
            onClick={() => logoutMutation.mutate()}
            title="退出登录"
            aria-label="退出登录"
            className={cn(
              "inline-flex h-8 flex-1 cursor-pointer items-center justify-center gap-1.5 rounded-console-sm border border-line bg-transparent px-2 text-xs font-medium text-muted transition-colors duration-[120ms] ease-out hover:bg-accent-soft hover:text-ink disabled:cursor-not-allowed disabled:opacity-50",
              collapsedTablet && "md:px-0 lg:px-2",
            )}
          >
            <ConsoleIcon name="logout" className="size-3.5 shrink-0" />
            <span className={labelHideClass}>退出</span>
          </button>
        </div>
      </div>
    </div>
  );
}

function NavGroup({
  pages,
  pathname,
  collapsedTablet,
  onNavigate,
}: {
  pages: readonly NavPage[];
  pathname: string;
  collapsedTablet: boolean;
  onNavigate?: (() => void) | undefined;
}) {
  return (
    <ul className="m-0 flex list-none flex-col gap-0.5 p-0">
      {pages.map((page) => {
        const active = page.to === "/" ? pathname === "/" : pathname.startsWith(page.to);
        return (
          <li key={page.id}>
            <Link
              to={page.to}
              aria-current={active ? "page" : undefined}
              aria-label={page.label}
              title={page.label}
              onClick={() => onNavigate?.()}
              className={cn(
                "relative flex items-center gap-2.5 rounded-console-sm px-2.5 py-2 text-[13px] font-semibold no-underline transition-colors duration-[120ms] ease-out",
                collapsedTablet && "md:justify-center md:px-0 lg:justify-start lg:px-2.5",
                active
                  ? "bg-accent-soft text-ink"
                  : "text-muted hover:bg-accent-soft/60 hover:text-ink",
              )}
            >
              {active ? (
                <span aria-hidden="true" className="absolute inset-y-1.5 left-0 w-[2px] rounded-full bg-accent" />
              ) : null}
              <ConsoleIcon name={page.icon} className={cn("size-4 shrink-0", active ? "text-accent" : "text-muted")} />
              <span className={cn("truncate", collapsedTablet && "md:hidden lg:inline")}>{page.label}</span>
            </Link>
          </li>
        );
      })}
    </ul>
  );
}

/** 顶部 48px L1 玻璃态状态操作条：环境状态、自动/手动刷新、管理员标识与受控重启。 */
function TopUtilityBar({
  onOpenMenu,
  onRequestRestart,
}: {
  onOpenMenu: () => void;
  onRequestRestart: () => void;
}) {
  const session = useAtomValue(sessionAtom);
  const [autoRefresh, setAutoRefresh] = useAtom(autoRefreshAtom);
  const statusQuery = useConsoleStatusQuery();

  const runtime = statusQuery.data?.runtime;
  const setupRequired = runtime?.state === "setup_required";
  const statusTone = !runtime ? "neutral" : setupRequired ? "warning" : runtime.ok ? "success" : "error";
  const statusLabel = !runtime ? "同步中" : setupRequired ? "等待配置" : runtime.ok ? "在线" : "异常";

  const updatedLabel = statusQuery.dataUpdatedAt
    ? new Date(statusQuery.dataUpdatedAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" })
    : "—";

  return (
    <header className="console-shell sticky top-0 z-20 flex h-12 shrink-0 items-center justify-between gap-3 border-b border-line px-4 sm:px-6 lg:px-8">
      <div className="flex min-w-0 items-center gap-3">
        <button
          type="button"
          aria-label="打开导航菜单"
          onClick={onOpenMenu}
          className="inline-flex size-8 cursor-pointer items-center justify-center rounded-console-sm border border-line bg-card/70 text-ink transition-colors duration-[120ms] hover:bg-accent-soft md:hidden"
        >
          <ConsoleIcon name="menu" className="size-4" />
        </button>
        <StatusBadge tone={statusTone} label={statusLabel} />
        <span className="hidden truncate font-mono text-xs text-muted sm:inline">
          管理员 <strong className="font-medium text-ink">{session?.username ?? "—"}</strong>
        </span>
      </div>

      <div className="flex items-center gap-2 sm:gap-3">
        <span className="hidden font-mono text-xs text-muted md:inline" title="最近状态刷新时间">
          刷新于 {updatedLabel}
        </span>
        <label className="hidden cursor-pointer items-center gap-1.5 text-xs text-muted select-none sm:inline-flex">
          <input
            type="checkbox"
            checked={autoRefresh}
            onChange={(event) => setAutoRefresh(event.target.checked)}
            className="size-3.5 accent-[var(--console-accent)]"
          />
          自动刷新
        </label>
        <Button
          variant="ghost"
          size="sm"
          disabled={statusQuery.isFetching}
          onClick={() => void statusQuery.refetch()}
          title="立即刷新运行状态"
        >
          <ConsoleIcon name="refresh" className="size-3.5" />
          <span className="hidden sm:inline">刷新</span>
        </Button>
        <Button variant="danger" size="sm" onClick={onRequestRestart}>
          <ConsoleIcon name="restart" className="size-3.5" />
          <span>重启服务</span>
        </Button>
      </div>
    </header>
  );
}

/** 受控重启：确认对话框 + 真实请求结果以 toast 展示。 */
function RestartConfirmDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (open: boolean) => void }) {
  const restartMutation = useMutation({
    mutationFn: requestRestart,
    onSuccess: (message) => {
      onOpenChange(false);
      showToast("info", message);
    },
    onError: (cause) => {
      showToast("error", cause instanceof Error ? cause.message : "重启请求失败");
    },
  });
  return (
    <ConfirmDialog
      open={open}
      onOpenChange={onOpenChange}
      title="重启服务"
      description="受控重启会结束当前进程并由守护脚本重新拉起；重启会使内存中的管理员 session 失效，需要重新登录。"
      confirmLabel="确认重启"
      danger
      busy={restartMutation.isPending}
      onConfirm={() => restartMutation.mutate()}
    />
  );
}
