/**
 * Notification store: connects to the lunaris-notifyd daemon via Tauri
 * events and provides reactive state for the UI.
 */

import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { writable, derived } from "svelte/store";
import { toast } from "svelte-sonner";

// ── Types ────────────────────────────────────────────────────────────────

export interface NotificationAction {
  key: string;
  label: string;
}

export interface Notification {
  id: number;
  app_name: string;
  summary: string;
  body: string;
  app_icon: string;
  actions: NotificationAction[];
  priority: string;
  category: string;
  timestamp: string;
  read: boolean;
}

export interface DndState {
  mode: string; // "off" | "on" | "scheduled"
}

interface SyncPayload {
  pending: Notification[];
  unread_count: number;
  dnd_mode: string;
}

// ── Stores ───────────────────────────────────────────────────────────────

/** All pending (not dismissed) notifications, newest first. */
export const notifications = writable<Notification[]>([]);

/** Current DND state. */
export const dndState = writable<DndState>({ mode: "off" });

/** Number of unread notifications. */
export const unreadCount = derived(notifications, ($n) =>
  $n.filter((n) => !n.read).length
);

/** Notifications grouped by app_name. */
export const groupedNotifications = derived(notifications, ($n) => {
  const groups = new Map<string, Notification[]>();
  for (const n of $n) {
    const key = n.app_name;
    const list = groups.get(key) ?? [];
    list.push(n);
    groups.set(key, list);
  }
  return groups;
});

// ── Actions ──────────────────────────────────────────────────────────────

export async function dismissNotification(id: number): Promise<void> {
  await invoke("notification_dismiss", { id });
}

export async function invokeAction(
  id: number,
  actionKey: string
): Promise<void> {
  await invoke("notification_invoke_action", { id, actionKey });
}

export async function markRead(id: number): Promise<void> {
  await invoke("notification_mark_read", { id });
}

export async function clearAll(): Promise<void> {
  await invoke("notification_clear_all");
}

export async function setDnd(mode: string): Promise<void> {
  await invoke("notification_set_dnd", { mode });
}

export async function getHistory(
  limit: number,
  beforeTimestamp: string = "",
  appName: string = ""
): Promise<void> {
  await invoke("notification_get_history", {
    limit,
    beforeTimestamp,
    appName,
  });
}

// ── Input Region Tracking ────────────────────────────────────────────────

let visibleCount = 0;

function onToastVisible() {
  visibleCount++;
  if (visibleCount === 1) {
    invoke("set_notification_input_region", { expanded: true }).catch(
      () => {}
    );
  }
}

function onToastGone() {
  visibleCount = Math.max(0, visibleCount - 1);
  if (visibleCount === 0) {
    invoke("set_notification_input_region", { expanded: false }).catch(
      () => {}
    );
  }
}

// ── Toast Logic ──────────────────────────────────────────────────────────

function showToast(n: Notification) {
  if (n.priority === "low") return;

  const description = n.body || undefined;

  const opts = {
    description,
    closeButton: true,
    onDismiss: onToastGone,
    onAutoClose: onToastGone,
  };

  onToastVisible();

  if (n.priority === "critical") {
    toast.error(n.summary, { ...opts, duration: Infinity });
  } else {
    const duration = n.priority === "high" ? 8000 : 4000;
    toast(n.summary, { ...opts, duration });
  }
}

// ── Initialization ───────────────────────────────────────────────────────

/** Initialize notification event listeners. Call once from +layout.svelte. */
export function initNotifications() {
  // New notification from daemon.
  listen<Notification>("notification:new", ({ payload }) => {
    notifications.update(($n) => [payload, ...$n]);
    showToast(payload);
  });

  // Notification closed/dismissed.
  listen<{ id: number }>("notification:closed", ({ payload }) => {
    notifications.update(($n) => $n.filter((n) => n.id !== payload.id));
  });

  // Notification marked as read.
  listen<{ id: number }>("notification:read", ({ payload }) => {
    notifications.update(($n) =>
      $n.map((n) => (n.id === payload.id ? { ...n, read: true } : n))
    );
  });

  // All cleared.
  listen("notification:cleared", () => {
    notifications.set([]);
  });

  // DND state changed.
  listen<DndState>("notification:dnd_changed", ({ payload }) => {
    dndState.set(payload);
  });

  // Initial sync from daemon.
  listen<SyncPayload>("notification:sync", ({ payload }) => {
    notifications.set(payload.pending);
    dndState.set({ mode: payload.dnd_mode });
  });

  // History response (appended to store for infinite scroll).
  listen<{ notifications: Notification[]; has_more: boolean }>(
    "notification:history",
    ({ payload }) => {
      notifications.update(($n) => {
        const existingIds = new Set($n.map((n) => n.id));
        const newOnes = payload.notifications.filter(
          (n) => !existingIds.has(n.id)
        );
        return [...$n, ...newOnes];
      });
    }
  );
}
