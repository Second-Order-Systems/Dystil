"use client"

import {
  Toast,
  ToastClose,
  ToastDescription,
  ToastProvider,
  ToastTitle,
  ToastViewport,
} from "@/components/ui/toast"
import { useToast } from "@/components/ui/use-toast"
import { cn } from "@/lib/utils"

export function Toaster() {
  const { toasts } = useToast()

  return (
    <ToastProvider>
      {toasts.map(function ({ id, title, description, action, persistent, ...props }) {
        return (
          <Toast key={id} {...props} open={persistent ? true : props.open} className={cn(props.className, persistent && "items-start gap-4 rounded-xl border-border bg-card p-4 shadow-[0_12px_32px_rgba(20,32,27,.16)]", persistent && props.variant === "destructive" && "border-destructive bg-destructive text-destructive-foreground")}>
            <div
              className="grid min-w-0 flex-1 gap-2"
              data-testid={props.variant === "destructive" ? "toast-error" : "toast-success"}
            >
              {title && <ToastTitle>{title}</ToastTitle>}
              {description && (
                <ToastDescription>{description}</ToastDescription>
              )}
            </div>
            {action && <div className="shrink-0 pt-0.5">{action}</div>}
            {!persistent && <ToastClose />}
          </Toast>
        )
      })}
      <ToastViewport />
    </ToastProvider>
  )
}
