/// Bridge for backend-emitted toasts.
///
/// Tauri-side code (e.g. `quick_action_run`) emits
/// `lunaris://toast` events with a kind + message payload. This
/// listener routes them through svelte-sonner so the user sees the
/// confirmation regardless of which window invoked the underlying
/// action — the Toaster mounted in `+layout.svelte` exists in both
/// the main and waypointer webviews, but the action originator
/// (waypointer) typically hides immediately after Enter, so the
/// reliable place to render the toast is the main top-bar.
///
/// Returns a disposer that unregisters the listener.

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { toast } from "svelte-sonner";

interface ToastPayload {
  kind: "success" | "info" | "warning" | "error";
  message: string;
}

export function initToastBridge(): () => void {
  let unlisten: UnlistenFn | null = null;

  listen<ToastPayload>("lunaris://toast", ({ payload }) => {
    const message = payload?.message ?? "";
    if (!message) return;
    switch (payload.kind) {
      case "success":
        toast.success(message);
        break;
      case "warning":
        toast.warning(message);
        break;
      case "error":
        toast.error(message);
        break;
      case "info":
      default:
        toast.info(message);
        break;
    }
  })
    .then((un) => {
      unlisten = un;
    })
    .catch((e) => {
      console.warn("[toast-bridge] listen failed:", e);
    });

  return () => {
    unlisten?.();
    unlisten = null;
  };
}
