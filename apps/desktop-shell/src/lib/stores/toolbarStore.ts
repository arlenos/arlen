/**
 * Per-(app, window) toolbar state derived from `app.toolbar.*`
 * Event Bus events forwarded by the Rust backend. The shell
 * maintains state for every (app, window) pair that has
 * emitted; only the focused window's slot renders in the top
 * bar.
 *
 * THE PER-WINDOW HALF DOES NOT WORK TODAY, and this header said
 * the opposite until 9 September ("Multi-window apps see
 * distinct toolbars per window — no last-emit-wins between
 * sibling windows"). The two halves of the key come from
 * different namespaces: the app publishes `window.label()`, its
 * own Tauri webview label, and `focusedToolbarKey` computes the
 * compositor's toplevel id. Those are not the same string and
 * do not become one, so the exact lookup below essentially
 * never hits and the FALLBACK — any state under the focused app
 * — is what renders. Which is last-emit-wins between sibling
 * windows, the thing this comment promised it was not.
 *
 * Nothing here can close that on its own: the shell would need
 * the app's webview label alongside the toplevel it belongs to,
 * which is a protocol question rather than a store one. What is
 * fixed is the consequence that WAS in reach — see
 * `forgetToolbarApp`.
 *
 * Mutually-exclusive variants (Quick Actions / Breadcrumb /
 * Progress) — setting one drops the others for that (app,
 * window) pair.
 *
 * See `docs/architecture/topbar-toolbar.md`.
 */

import { derived, get, writable, type Readable } from "svelte/store";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { activeWindow } from "./windows";
import { activeAppId } from "./activeApp";

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

/// Drop everything one app published, across every window it had.
///
/// The teardown an app that died never got to send. `toolbar_clear` exists on
/// the plugin and nothing calls it, and an app that crashes or is killed could
/// not call it anyway - so the shell reclaims on OBSERVED ABSENCE instead, the
/// same rule the knowledge daemon uses for an unclosed presence and the same one
/// `forgetApp` applies to the badge, shortcut and ambient stores.
///
/// This store was the one those four missed, and it leaks differently because it
/// is keyed per (app, WINDOW): its entries survive the window that made them, so
/// a webview label an app reuses - `main`, across a restart - would hand a fresh
/// window the previous instance's toolbar, drawn before the new one has
/// published anything. Dropping by app takes those with it, which is why this is
/// keyed on the app half rather than trying to match the window half: the
/// `windowId` is the SOURCE APP's own webview label, opaque to the shell and not
/// comparable with anything in its window list.
export function forgetToolbarApp(appId: string): void {
  internal.update((s) => {
    const prefix = `${appId}\x1f`;
    const doomed = [...s.byKey.keys()].filter((k) => k.startsWith(prefix));
    if (doomed.length === 0) return s;
    const next = new Map(s.byKey);
    for (const k of doomed) next.delete(k);
    return { byKey: next };
  });
}

/// Which apps this store currently holds a toolbar for. The lifetime watcher
/// reads it to decide what has outlived its windows; nothing else should.
export function appsWithToolbar(): string[] {
  const ids = new Set<string>();
  for (const k of get(internal).byKey.keys()) ids.add(k.split("\x1f")[0]);
  return [...ids];
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
 *
 * `appId` is the permission id, since that is what a toolbar state is published
 * under; the window's own app_id is a different name for the same app and keying
 * on it matched nothing. See `activeApp.ts`.
 */
export const focusedToolbarKey: Readable<ToolbarKey | null> = derived(
  [activeAppId, activeWindow],
  ([$appId, $active]) => {
    const appId = $appId;
    if (!appId || !$active) return null;
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
 * (appId, windowId) miss. That was written as a safety net for
 * the legacy empty-`windowId` producer and for a label that
 * differs from the toplevel id; measured, it is the normal
 * path, because the SDK sends a webview label and this store is
 * asked with a compositor toplevel id. "Most single-webview-
 * per-window apps coincide" is not true of two identifiers
 * minted by different systems.
 *
 * So it is doing real work and must stay until the two names
 * can be matched — without it every toolbar in the system goes
 * dark. It is also why a stale entry is not merely stored but
 * RENDERED, which is what makes `forgetToolbarApp` a visible
 * fix rather than housekeeping.
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
