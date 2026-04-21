/**
 * Notification store: connects to the lunaris-notifyd daemon via Tauri
 * events and provides reactive state for the UI.
 */

import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { writable, derived, get } from "svelte/store";
import { toast } from "svelte-sonner";
import type { Component } from "svelte";
import NotificationToastIcon from "$lib/components/NotificationToastIcon.svelte";
import { activePopover } from "./activePopover.js";
import { windows } from "./windows.js";

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
  console.log("[notifications] dismiss", id);
  // Optimistic: remove from store immediately.
  notifications.update(($n) => $n.filter((n) => n.id !== id));
  invoke("notification_dismiss", { id }).catch((e) =>
    console.error("[notifications] dismiss failed:", e)
  );
}

export async function invokeAction(
  id: number,
  actionKey: string
): Promise<void> {
  console.log("[notifications] invokeAction", id, actionKey);
  invoke("notification_invoke_action", { id, actionKey }).catch((e) =>
    console.error("[notifications] invokeAction failed:", e)
  );
}

export async function markRead(id: number): Promise<void> {
  await invoke("notification_mark_read", { id });
}

export async function clearAll(): Promise<void> {
  console.log("[notifications] clearAll");
  // Optimistic: clear store immediately.
  notifications.set([]);
  invoke("notification_clear_all").catch((e) =>
    console.error("[notifications] clearAll failed:", e)
  );
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

// ── Toast Queue (suppress while panel open) ─────────────────────────────

const MAX_QUEUED = 5;
let toastQueue: Notification[] = [];
let panelOpen = false;

// Track panel state.
activePopover.subscribe((v) => {
  const wasOpen = panelOpen;
  panelOpen = v !== null;
  // Panel just closed: flush queued toasts.
  if (wasOpen && !panelOpen) {
    const toFlush = toastQueue.splice(0, MAX_QUEUED);
    toastQueue = [];
    for (const n of toFlush) {
      fireToast(n);
    }
  }
});

// ── Toast Logic ──────────────────────────────────────────────────────────

function showToast(n: Notification) {
  if (n.priority === "low") return;

  if (panelOpen) {
    // Queue instead of showing while a panel is open.
    toastQueue.push(n);
    if (toastQueue.length > MAX_QUEUED) {
      toastQueue.shift();
    }
    return;
  }

  fireToast(n);
}

/// Build a per-toast icon component with `app_icon` and `app_name`
/// pre-bound via closure. svelte-sonner instantiates the icon slot as
/// `<toast.icon />` without passing props, so we wrap the real Svelte 5
/// component (which is a function of `(anchor, props)`) in a thin
/// forwarder that injects the baked-in values.
function makeToastIcon(iconUrl: string, appName: string): Component {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const Inner = NotificationToastIcon as unknown as (
    anchor: any,
    props: { iconUrl: string; appName: string },
  ) => unknown;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  return ((anchor: any) => Inner(anchor, { iconUrl, appName })) as Component;
}

/// Find a matching open window for a notification and activate it.
/// Case-insensitive; tries exact, then substring both directions, so
/// "Thunderbird" matches `thunderbird`, "Discord" matches
/// `org.discordapp.Discord`, and so on. Does nothing if no window
/// matches — we deliberately don't launch the app here; a click that
/// silently fails is safer than spawning a fresh process when the
/// user just wanted to surface an existing window.
function focusAppForNotification(n: Notification): void {
  const target = n.app_name.toLowerCase().trim();
  if (!target) return;
  const wins = get(windows);
  const match =
    wins.find((w) => w.app_id.toLowerCase() === target) ??
    wins.find((w) => w.app_id.toLowerCase().includes(target)) ??
    wins.find((w) => target.includes(w.app_id.toLowerCase())) ??
    null;
  if (match) {
    invoke("activate_window", { id: match.id }).catch((e) =>
      console.warn("[toast-click] activate_window failed:", e),
    );
  } else {
    console.log(
      `[toast-click] no window match for app_name="${n.app_name}" (tried ${wins.length} windows)`,
    );
  }
}

/// Install a single document-level click listener that turns clicks on
/// a toast body (anywhere outside action/cancel/close buttons) into a
/// focus-the-app action. Runs once; subsequent calls are no-ops.
///
/// We deliberately do not use svelte-sonner's built-in action handler
/// here: that would conflate "click to peek" with "confirm primary
/// action", which is dangerous for destructive notifications (e.g.
/// "Delete all messages" with an Undo action would execute the delete
/// instead of surfacing the app).
let toastClickHandlerInstalled = false;
function installToastClickHandler(): void {
  if (toastClickHandlerInstalled) return;
  toastClickHandlerInstalled = true;
  document.addEventListener("click", (e) => {
    const target = e.target as Element | null;
    if (!target) return;
    const toastEl = target.closest("[data-sonner-toast]");
    if (!toastEl) return;
    // Skip clicks that land on one of the toast's own buttons — those
    // own their handlers (action/cancel callbacks, close dismiss).
    if (target.closest("button")) return;
    // Per-toast marker class carries the notification id.
    const idClass = Array.from(toastEl.classList).find((c) =>
      c.startsWith("lunaris-notif-"),
    );
    if (!idClass) return;
    const id = parseInt(idClass.slice("lunaris-notif-".length), 10);
    if (Number.isNaN(id)) return;
    const notif = get(notifications).find((nn) => nn.id === id);
    if (!notif) return;
    focusAppForNotification(notif);
    toast.dismiss(id);
  });
}

function fireToast(n: Notification) {
  const description = n.body || undefined;

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const opts: Record<string, any> = {
    // Use the notification's own id so we can `toast.dismiss(id)`
    // after surfacing the app on body-click.
    id: n.id,
    description,
    closeButton: true,
    icon: makeToastIcon(n.app_icon, n.app_name),
    // Per-toast marker class for click routing. Merged with the
    // Toaster-level default class by svelte-sonner.
    class: `lunaris-notif-${n.id}`,
    onDismiss: onToastGone,
    onAutoClose: onToastGone,
  };

  // Map first two notification actions to toast action/cancel buttons.
  if (n.actions.length > 0) {
    const actionKey = n.actions[0].key;
    const nId = n.id;
    opts.action = {
      label: n.actions[0].label,
      onClick: () => {
        console.log("[toast] action clicked:", nId, actionKey);
        invokeAction(nId, actionKey);
      },
    };
  }
  if (n.actions.length > 1) {
    const cancelKey = n.actions[1].key;
    const nId = n.id;
    opts.cancel = {
      label: n.actions[1].label,
      onClick: () => {
        console.log("[toast] cancel clicked:", nId, cancelKey);
        invokeAction(nId, cancelKey);
      },
    };
  }

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
  installToastClickHandler();

  // New notification from daemon.
  listen<Notification>("notification:new", ({ payload }) => {
    console.log("[notifications] new:", payload.id, payload.app_name, payload.summary, "icon:", payload.app_icon ? "yes" : "no");
    notifications.update(($n) => {
      const updated = [payload, ...$n];
      // Cap at 200 to prevent unbounded memory growth in long sessions.
      return updated.length > 200 ? updated.slice(0, 200) : updated;
    });
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
    console.log("[notifications] sync:", payload.pending.length, "pending, dnd:", payload.dnd_mode);
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
