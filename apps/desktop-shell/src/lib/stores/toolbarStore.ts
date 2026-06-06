/**
 * Per-(app, window) toolbar state derived from `app.toolbar.*`
 * Event Bus events forwarded by the Rust backend. The shell
 * maintains state for every (app, window) pair that has
 * emitted; only the focused window's slot renders in the top
 * bar. Multi-window apps see distinct toolbars per window — no
 * last-emit-wins between sibling windows of the same app.
 *
 * Mutually-exclusive variants (Quick Actions / Breadcrumb /
 * Progress) — setting one drops the others for that (app,
 * window) pair.
 *
 * See `docs/architecture/topbar-toolbar.md`.
 */

import { derived, writable, type Readable } from "svelte/store";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { activeWindow } from "./windows";

export interface ToolbarQuickAction {
  icon: string;
  action: string;
  tooltip: string;
  toggle: boolean;
  active: boolean;
}

export interface ToolbarBreadcrumbItem {
  label: string;
  action: string;
}

export interface ToolbarProgress {
  value: number;
  label?: string | null;
}

export type ToolbarSlot =
  | { kind: "none" }
  | { kind: "quick-actions"; actions: ToolbarQuickAction[] }
  | { kind: "breadcrumb"; items: ToolbarBreadcrumbItem[] }
  | { kind: "progress"; progress: ToolbarProgress };

/**
 * Store key. The `windowId` is the source app's Tauri webview
 * label (per-app namespace, opaque to the shell). Empty string
 * is a legacy fallback for single-window producers.
 */
export interface ToolbarKey {
  appId: string;
  windowId: string;
}

interface ToolbarStore {
  /**
   * Map serialised key -> current slot. Keys serialised as
   * `${appId}\x1f${windowId}` (US separator) — `app_id` and
   * window labels do not legitimately contain control chars.
   */
  byKey: Map<string, ToolbarSlot>;
}

const initial: ToolbarStore = { byKey: new Map() };

const internal = writable<ToolbarStore>(initial);

function keyOf(appId: string, windowId: string): string {
  return `${appId}\x1f${windowId}`;
}

function updateKey(appId: string, windowId: string, slot: ToolbarSlot) {
  internal.update((s) => {
    const next = new Map(s.byKey);
    const key = keyOf(appId, windowId);
    if (slot.kind === "none") {
      next.delete(key);
    } else {
      next.set(key, slot);
    }
    return { byKey: next };
  });
}

function clearProgress(appId: string, windowId: string) {
  internal.update((s) => {
    const key = keyOf(appId, windowId);
    const cur = s.byKey.get(key);
    if (!cur || cur.kind !== "progress") return s;
    const next = new Map(s.byKey);
    next.delete(key);
    return { byKey: next };
  });
}

interface QuickActionsEvent {
  appId: string;
  windowId: string;
  actions: ToolbarQuickAction[];
}
interface BreadcrumbEvent {
  appId: string;
  windowId: string;
  items: ToolbarBreadcrumbItem[];
}
interface ProgressEvent {
  appId: string;
  windowId: string;
  value: number;
  label?: string | null;
}
interface KeyOnlyEvent {
  appId: string;
  windowId: string;
}

/**
 * Wire Tauri-event listeners. Returns a disposer that removes
 * every registered listener (matches the +layout init pattern).
 */
export function initToolbarStore(): () => void {
  const unlistens: UnlistenFn[] = [];
  const tasks = [
    listen<QuickActionsEvent>("arlen://toolbar-quick-actions", (e) => {
      updateKey(e.payload.appId, e.payload.windowId, {
        kind: "quick-actions",
        actions: e.payload.actions,
      });
    }),
    listen<BreadcrumbEvent>("arlen://toolbar-breadcrumb", (e) => {
      updateKey(e.payload.appId, e.payload.windowId, {
        kind: "breadcrumb",
        items: e.payload.items,
      });
    }),
    listen<ProgressEvent>("arlen://toolbar-progress", (e) => {
      updateKey(e.payload.appId, e.payload.windowId, {
        kind: "progress",
        progress: { value: e.payload.value, label: e.payload.label ?? null },
      });
    }),
    listen<KeyOnlyEvent>("arlen://toolbar-progress-cleared", (e) => {
      clearProgress(e.payload.appId, e.payload.windowId);
    }),
    listen<KeyOnlyEvent>("arlen://toolbar-cleared", (e) => {
      updateKey(e.payload.appId, e.payload.windowId, { kind: "none" });
    }),
  ];

  Promise.all(tasks)
    .then((u) => unlistens.push(...u))
    .catch((e) => console.warn("initToolbarStore listen failed:", e));

  return () => {
    for (const u of unlistens) {
      try {
        u();
      } catch {
        // swallow — disposer must not throw
      }
    }
  };
}

/**
 * The (appId, windowId) the focused-toolbar derived store is
 * currently rendering for. Useful for the action-dispatch
 * command which needs to send the window_id back to the
 * source app.
 */
export const focusedToolbarKey: Readable<ToolbarKey | null> = derived(
  activeWindow,
  ($active) => {
    const appId = $active?.app_id;
    if (!appId) return null;
    // Tauri exposes the *cosmic-toplevel* id rather than a webview
    // label here — for cross-process toolbar matching we use the
    // `id` field which is stable per top-level. This must match
    // what the source app passes as `window_id` (its own
    // `WebviewWindow::label()`). Mapping fidelity is a Phase 6
    // concern: the SDK currently sends webview-label and the
    // shell receives compositor-toplevel-id, so multi-window
    // apps are reliable only when the app uses one webview per
    // top-level (the common case).
    return { appId, windowId: $active.id };
  },
);

/**
 * Slot to render in the top bar. Reflects the toolbar state of
 * the (focused app, focused window) pair, or `{ kind: "none" }`
 * when no key is focused or no state has been emitted for it.
 *
 * Falls back to ANY state for the focused app if the exact
 * (appId, windowId) miss — covers the legacy producer case
 * where `windowId` is empty, and the case where the SDK's
 * webview-label differs from the shell's toplevel-id (most
 * single-webview-per-window apps coincide, but third-party
 * Tauri apps may not).
 */
export const focusedToolbar: Readable<ToolbarSlot> = derived(
  [internal, focusedToolbarKey],
  ([$internal, $key]) => {
    if (!$key) return { kind: "none" } as ToolbarSlot;
    const exact = $internal.byKey.get(keyOf($key.appId, $key.windowId));
    if (exact) return exact;
    // Fallback: any state under this app (legacy or label-mismatch).
    const prefix = `${$key.appId}\x1f`;
    for (const [k, slot] of $internal.byKey) {
      if (k.startsWith(prefix)) return slot;
    }
    return { kind: "none" } as ToolbarSlot;
  },
);
