import { listen } from "@tauri-apps/api/event";
import { toast } from "svelte-sonner";
import { invoke } from "@tauri-apps/api/core";
import { notificationCount } from "./applets.js";

export interface NotificationPayload {
    id: number;
    app_name: string;
    summary: string;
    body: string;
    /** "critical" | "high" | "normal" | "low" */
    priority: string;
}

let visibleCount = 0;

function onToastVisible() {
    visibleCount++;
    if (visibleCount === 1) {
        invoke("set_notification_input_region", { expanded: true }).catch(() => {});
    }
}

function logToastRects() {
    const el = document.querySelector("[data-sonner-toast]");
    if (!el) {
        console.log("[notifications] no [data-sonner-toast] found in DOM");
        return;
    }
    const all = document.querySelectorAll("[data-sonner-toast]");
    all.forEach((t, i) => {
        const r = (t as HTMLElement).getBoundingClientRect();
        console.log(`[notifications] toast[${i}] rect: x=${r.x} y=${r.y} w=${r.width} h=${r.height}`);
    });
    const toaster = document.querySelector("[data-sonner-toaster]");
    if (toaster) {
        const r = (toaster as HTMLElement).getBoundingClientRect();
        console.log(`[notifications] toaster rect: x=${r.x} y=${r.y} w=${r.width} h=${r.height}`);
    }
}

function onToastGone() {
    visibleCount = Math.max(0, visibleCount - 1);
    if (visibleCount === 0) {
        invoke("set_notification_input_region", { expanded: false }).catch(() => {});
    }
}

/** Registers the Tauri event listener for `lunaris://notification-show`. */
export function initNotificationListener() {
    listen<NotificationPayload>("lunaris://notification-show", ({ payload }) => {
        notificationCount.update(n => n + 1);

        if (payload.priority === "low") return;

        const description = payload.body || undefined;

        onToastVisible();

        if (payload.priority === "critical") {
            toast.error(payload.summary, {
                description,
                duration: Infinity,
                closeButton: true,
                onDismiss: onToastGone,
                onAutoClose: onToastGone,
            });
        } else {
            toast(payload.summary, {
                description,
                duration: payload.priority === "high" ? 8000 : 4000,
                closeButton: true,
                onDismiss: onToastGone,
                onAutoClose: onToastGone,
            });
        }
        setTimeout(logToastRects, 100);
    });
}
