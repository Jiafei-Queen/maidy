import * as DialogPrimitive from "@radix-ui/react-dialog";
import type { ReactNode } from "react";
import { Button } from "./button.js";

type ConfirmDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description?: ReactNode;
  confirmLabel: string;
  cancelLabel?: string;
  danger?: boolean;
  onConfirm: () => void;
  /** 确认动作进行中时禁用按钮，防止重复提交。 */
  busy?: boolean;
};

/**
 * 受控确认对话框（Radix Dialog）：6px 微圆角、1px 边框与轻阴影，120ms 静默淡入。
 */
export function ConfirmDialog({
  open,
  onOpenChange,
  title,
  description,
  confirmLabel,
  cancelLabel = "取消",
  danger = false,
  onConfirm,
  busy = false,
}: ConfirmDialogProps) {
  return (
    <DialogPrimitive.Root open={open} onOpenChange={onOpenChange}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="fixed inset-0 z-40 bg-scrim backdrop-blur-[2px] animate-fade-in" />
        <DialogPrimitive.Content className="console-dialog fixed top-1/2 left-1/2 z-50 w-[min(26rem,calc(100vw-1.5rem))] -translate-x-1/2 -translate-y-1/2 p-6 animate-page-in">
          <DialogPrimitive.Title className="m-0 font-display text-xl font-semibold text-ink">{title}</DialogPrimitive.Title>
          {description ? (
            <DialogPrimitive.Description className="mt-2 text-sm leading-relaxed text-muted">
              {description}
            </DialogPrimitive.Description>
          ) : null}
          <div className="mt-6 flex justify-end gap-2.5">
            <DialogPrimitive.Close asChild>
              <Button variant="secondary" disabled={busy}>
                {cancelLabel}
              </Button>
            </DialogPrimitive.Close>
            <Button
              variant={danger ? "danger" : "primary"}
              disabled={busy}
              onClick={() => {
                onConfirm();
              }}
            >
              {confirmLabel}
            </Button>
          </div>
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  );
}
