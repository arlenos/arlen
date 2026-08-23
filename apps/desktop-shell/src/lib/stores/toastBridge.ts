/// Bridge for backend-emitted toasts.
///
/// Tauri-side code (e.g. `quick_action_run`) emits
/// `arlen://toast` events with a kind + message payload. This
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
import { get } from "svelte/store";
import { t } from "$lib/i18n/messages";

interface ToastPayload {
  kind: "success" | "info" | "warning" | "error";
  message: string;
  /// Values the named line interpolates.
  params?: Record<string, string>;
  /// A catalog id to render instead of `message`.
  ///
  /// The backend knows WHICH line to say and not the words for it - the catalog
  /// is here. A quick action used to build its own English sentence and emit it
  /// as text, so a German desktop flipped the switch and answered in English.
  key?: string;
}

export function initToastBridge(): () => void {
  let unlisten: UnlistenFn | null = null;

  listen<ToastPayload>("arlen://toast", ({ payload }) => {
    const write = get(t);
    // A key the catalog does not carry renders as the id, which is legible and
    // greppable; falling back to the backend's own `message` would put the
    // source language on screen, which is the thing this exists to stop.
    const message = payload?.key
      ? write(payload.key, payload.params ?? undefined)
      : (payload?.message ?? "");
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
