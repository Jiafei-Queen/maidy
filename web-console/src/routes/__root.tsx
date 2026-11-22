import { createRootRoute } from "@tanstack/react-router";
import { useAtomValue } from "jotai";
import { AppShell } from "../components/layout/app-shell.js";
import { ConsoleBackground } from "../components/layout/console-background.js";
import { AuthGate } from "../features/auth/auth-gate.js";
import { authPhaseAtom } from "../stores/auth.js";
import { ConsoleToastHost } from "../components/ui/toast.js";

export const Route = createRootRoute({
  component: RootLayout,
});

function RootLayout() {
  const phase = useAtomValue(authPhaseAtom);
  return (
    <>
      <ConsoleBackground />
      <ConsoleToastHost />
      {phase === "checking" ? <AuthChecking /> : phase === "unauthenticated" ? <AuthGate /> : <AppShell />}
    </>
  );
}

function AuthChecking() {
  return (
    <div className="relative z-1 grid min-h-dvh place-items-center p-4">
      <div className="console-panel px-5 py-3.5 shadow-console">
        <p className="console-meta m-0" role="status">
          正在恢复管理员会话…
        </p>
      </div>
      {/* 会话恢复完成前不挂载业务路由，避免提前触发受保护 API。 */}
    </div>
  );
}
