import { listen } from "@tauri-apps/api/event";
import { toast } from "svelte-sonner";
import { notificationCount } from "./applets.js";

export interface NotificationPayload {
    id: number;
    app_name: string;
    summary: string;
    body: string;
    /** "critical" | "high" | "normal" | "low" */
    priority: string;
}

/// Registers the Tauri event listener for `lunaris://notification-show`.
/// Must be called once from +layout.svelte onMount.
export function initNotificationListener() {
    listen<NotificationPayload>("lunaris://notification-show", ({ payload }) => {
        notificationCount.update(n => n + 1);

        if (payload.priority === "low") return; // silent: count only

        const description = payload.body || undefined;

        if (payload.priority === "critical") {
            toast.error(payload.summary, {
                description,
                duration: Infinity,
                closeButton: true,
            });
        } else {
            toast(payload.summary, {
                description,
                duration: payload.priority === "high" ? 8000 : 4000,
                closeButton: true,
            });
        }
    });
}
