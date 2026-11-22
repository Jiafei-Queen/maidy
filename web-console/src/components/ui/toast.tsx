import * as ToastPrimitive from "@radix-ui/react-toast";
import { useAtomValue, useSetAtom } from "jotai";
import { toastAtom } from "../../stores/toast.js";
import { cn } from "../../lib/utils.js";

const KIND_CLASSES = {
  info: "border-line text-ink",
  success: "border-success/50 text-success",
  error: "border-error/50 text-error",
} as const;

/** 全局单条 Toast：L1 玻璃态浮层，点击立即关闭，role=status 播报。 */
export function ConsoleToastHost() {
  const toast = useAtomValue(toastAtom);
  const setToast = useSetAtom(toastAtom);
  return (
    <ToastPrimitive.Provider swipeDirection="right" duration={5000}>
      <ToastPrimitive.Root
        key={toast?.id}
        open={toast !== null}
        onOpenChange={(open) => {
          if (!open) setToast(null);
        }}
        onClick={() => setToast(null)}
        role="status"
        className={cn(
          "console-shell fixed right-4 bottom-5 z-50 w-[min(26rem,calc(100vw-2rem))] cursor-pointer rounded-console-lg border px-4 py-3 text-[13px] font-medium shadow-console animate-fade-in",
          KIND_CLASSES[toast?.kind ?? "info"],
        )}
      >
        <ToastPrimitive.Description>{toast?.message}</ToastPrimitive.Description>
      </ToastPrimitive.Root>
      <ToastPrimitive.Viewport />
    </ToastPrimitive.Provider>
  );
}
