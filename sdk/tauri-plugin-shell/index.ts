/**
 * @arlen/tauri-plugin-shell
 *
 * TypeScript surface for the Arlen OS shell.* APIs. The Rust plugin
 * registers `plugin:arlen-shell|*` commands; this module provides a
 * typed wrapper that mirrors foundation §6 (presence, timeline,
 * spatial). `shell.menu` is **not** here — it lives in desktop-shell
 * because menus are global state owned by the shell.
 *
 * # Usage
 *
 * ```typescript
 * import { shell } from "@arlen/tauri-plugin-shell";
 *
 * await shell.presence.set({ activity: "editing", subject: "report.md" });
 * await shell.timeline.record({
 *   label: "Exported PDF",
 *   subject: "/home/tim/report.pdf",
 *   type: "export",
 * });
 * ```
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// ── Types (mirror os-sdk Rust API) ────────────────────────────────────

export type AutoClear = "on-blur" | "on-idle" | "manual";

export interface PresenceParams {
  activity: string;
  subject: string;
  project?: string;
  metadata?: Record<string, string>;
  auto_clear?: AutoClear;
}

export interface TimelineParams {
  label: string;
  subject: string;
  type: string;
  started_at?: number;
  ended_at?: number;
  metadata?: Record<string, string>;
}

export interface OutputHint {
  connector?: string;
}

export interface GeometryHint {
  x?: number;
  y?: number;
  width?: number;
  height?: number;
}

export interface SpatialHint {
  window_id: string;
  output?: OutputHint;
  geometry?: GeometryHint;
}

// ── Plugin commands ───────────────────────────────────────────────────
//
// The plugin registers commands under the `arlen-shell` namespace.
// Tauri exposes them as `plugin:arlen-shell|<command>`.

const PLUGIN = "plugin:arlen-shell";

export const presence = {
  async set(params: PresenceParams): Promise<void> {
    return invoke(`${PLUGIN}|presence_set`, { params });
  },
  async clear(): Promise<void> {
    return invoke(`${PLUGIN}|presence_clear`);
  },
};

export const timeline = {
  async record(params: TimelineParams): Promise<void> {
    return invoke(`${PLUGIN}|timeline_record`, { params });
  },
};

export const spatial = {
  async hint(hint: SpatialHint): Promise<void> {
    return invoke(`${PLUGIN}|spatial_hint`, { hint });
  },
};

// ── shell.annotations ─────────────────────────────────────────────────

export type AnnotationTarget =
  | { type: "File"; path: string }
  | { type: "App"; id: string }
  | { type: "Project"; id: string }
  | { type: "Session"; id: string };

export interface AnnotationSetParams {
  target: AnnotationTarget;
  namespace: string;
  data: unknown;
}

export interface AnnotationLookup {
  target: AnnotationTarget;
  namespace: string;
}

export interface AnnotationRecord {
  data: unknown;
  /** Microseconds since Unix epoch. */
  created_at: number;
  /** Microseconds since Unix epoch. */
  last_modified: number;
}

/**
 * Tagged-union payload delivered to `onChanged` handlers.
 *
 * Wire form matches `serde(tag = "kind", rename_all = "lowercase")`
 * on the Rust side: `{ kind: "set", target, namespace, app_id, data }`
 * or `{ kind: "cleared", target, namespace, app_id }`.
 */
export type AnnotationChange =
  | {
      kind: "set";
      target: AnnotationTarget;
      namespace: string;
      app_id: string;
      data: unknown;
    }
  | {
      kind: "cleared";
      target: AnnotationTarget;
      namespace: string;
      app_id: string;
    };

export interface AnnotationSubscribeParams {
  target: AnnotationTarget;
  namespace: string;
}

export const annotations = {
  async set(params: AnnotationSetParams): Promise<void> {
    return invoke(`${PLUGIN}|annotation_set`, { params });
  },
  async clear(lookup: AnnotationLookup): Promise<void> {
    return invoke(`${PLUGIN}|annotation_clear`, { lookup });
  },
  async get(lookup: AnnotationLookup): Promise<AnnotationRecord | null> {
    return invoke(`${PLUGIN}|annotation_get`, { lookup });
  },
  /**
   * Subscribe to annotation changes for a specific target+namespace.
   *
   * Returns an unsubscribe function. Call it (or let the window
   * close — subscriptions are automatically torn down on
   * `WindowEvent::Destroyed`) to release the subscription.
   *
   * Subscribers see future events only. To bootstrap with the
   * current state, call `annotations.get()` first; there is a
   * small race window between the two calls (FA8 in
   * `docs/architecture/annotations-api.md`).
   *
   * Implementation note — two-step subscribe:
   *
   *   1. `annotation_subscribe_prepare` opens the bus stream
   *      and parks events in a backend buffer.
   *   2. `listen()` registers the JS handler.
   *   3. `annotation_subscribe_start` flushes the buffer and
   *      begins emitting per-webview events going forward.
   *
   * The order is what closes the listener-registration race —
   * any event between prepare and start sits in the backend
   * buffer until the JS listener exists. The single-shot
   * `subscribe()` shape was a footgun precisely here.
   */
  async onChanged(
    params: AnnotationSubscribeParams,
    handler: (change: AnnotationChange) => void,
  ): Promise<() => Promise<void>> {
    const subscriptionId: string = await invoke(
      `${PLUGIN}|annotation_subscribe_prepare`,
      { params },
    );
    const eventName = `arlen://annotation-changed/${subscriptionId}`;
    const unlisten: UnlistenFn = await listen<AnnotationChange>(
      eventName,
      (e) => handler(e.payload),
    );
    // Listener is now registered; safe to start the pump.
    await invoke(`${PLUGIN}|annotation_subscribe_start`, { subscriptionId });
    return async () => {
      unlisten();
      await invoke(`${PLUGIN}|annotation_unsubscribe`, { subscriptionId });
    };
  },
};

// ── shell.toolbar ─────────────────────────────────────────────────

export interface QuickAction {
  /** ui-kit / Lucide icon id. */
  icon: string;
  /** Opaque action string dispatched back to the app on click. */
  action: string;
  tooltip: string;
  toggle?: boolean;
  active?: boolean;
}

export interface BreadcrumbItem {
  label: string;
  action: string;
}

export interface ProgressState {
  /** Clamped to [0, 1] backend-side. */
  value: number;
  label?: string;
}

/**
 * Payload of the per-window `arlen://app-action/{action}`
 * Tauri event the shell dispatches when the user clicks a
 * Quick Action or Breadcrumb segment in the top bar.
 */
export interface AppActionEvent {
  action: string;
}

/**
 * Hard cap from foundation §6.4 Listing 22. The SDK rejects
 * `setQuickActions` calls with more than this many entries.
 */
export const MAX_QUICK_ACTIONS = 3;

export const toolbar = {
  /**
   * Push Quick Action buttons into the top bar slot. Replaces
   * any previously-set Quick Actions, Breadcrumb, or Progress
   * (mutually exclusive).
   *
   * Throws if `actions.length > MAX_QUICK_ACTIONS`.
   */
  async setQuickActions(actions: QuickAction[]): Promise<void> {
    return invoke(`${PLUGIN}|toolbar_set_quick_actions`, { actions });
  },
  /** Push a Breadcrumb path. Replaces any previously-set toolbar variant. */
  async setBreadcrumb(items: BreadcrumbItem[]): Promise<void> {
    return invoke(`${PLUGIN}|toolbar_set_breadcrumb`, { items });
  },
  /** Set the Progress indicator. `value` is clamped to [0, 1] backend-side. */
  async setProgress(progress: ProgressState): Promise<void> {
    return invoke(`${PLUGIN}|toolbar_set_progress`, { progress });
  },
  /** Clear only the Progress slot. Quick Actions / Breadcrumb stay. */
  async clearProgress(): Promise<void> {
    return invoke(`${PLUGIN}|toolbar_clear_progress`);
  },
  /**
   * Drop every toolbar variant for this app. The shell also
   * clears on focus loss automatically; call this for eager
   * pre-blur clear (e.g. mid-task abort).
   */
  async clear(): Promise<void> {
    return invoke(`${PLUGIN}|toolbar_clear`);
  },
  /**
   * Subscribe to action dispatches from the shell. The shell
   * fires `arlen://app-action/{action}` Tauri events scoped
   * to this webview when the user clicks a Quick Action or
   * Breadcrumb segment.
   *
   * Returns an unsubscribe function. The handler receives the
   * action id; the app routes from there.
   *
   * Listener is window-scoped: actions only reach the focused
   * webview that registered. For multi-webview apps, each
   * webview must register its own handler if it handles
   * different actions.
   */
  async onAction(
    handler: (event: AppActionEvent) => void,
  ): Promise<() => void> {
    // Listen to the per-action event family. Tauri does not
    // support wildcards in `listen`, so we use a single shared
    // event name and let the handler discriminate.
    const unlisten: UnlistenFn = await listen<AppActionEvent>(
      "arlen://app-action",
      (e) => handler(e.payload),
    );
    return () => unlisten();
  },
};

// ── shell.shortcuts ───────────────────────────────────────────────

export interface Shortcut {
  label: string;
  /** ui-kit / Lucide icon id. */
  icon: string;
  /** Opaque dispatch identifier (received in `onAction`). */
  action: string;
  /**
   * Tag filter for Focus-Mode-aware rendering. Phase 1 ignores
   * this field; Phase 6 brings tag-aware filtering once the
   * project tag system lands.
   */
  context?: string[];
  /**
   * Optional confirm-dialog text. When set, the shell shows a
   * yes/no dialog before dispatching the action.
   */
  confirm?: string;
}

export interface ShortcutState {
  enabled?: boolean;
  /** "" clears the badge. */
  badge?: string;
}

export const shortcuts = {
  /**
   * Register the app's full shortcut list. Replaces any
   * previously-registered set. Empty list = clear.
   */
  async register(list: Shortcut[]): Promise<void> {
    return invoke(`${PLUGIN}|shortcuts_register`, { shortcuts: list });
  },
  /**
   * Update one shortcut's state without re-emitting the full
   * list. Action must reference a previously-registered
   * shortcut; silently no-op on miss.
   */
  async setState(action: string, state: ShortcutState): Promise<void> {
    return invoke(`${PLUGIN}|shortcuts_set_state`, { action, newState: state });
  },
  async clear(): Promise<void> {
    return invoke(`${PLUGIN}|shortcuts_clear`);
  },
  // onAction is provided by `toolbar.onAction` — the same
  // listener handles both toolbar Quick-Action / Breadcrumb
  // clicks and shortcut clicks (uniform `arlen://app-action`
  // event on the receive side).
};

// ── shell.badges ──────────────────────────────────────────────────

export type BadgeStatus = "success" | "warning" | "error" | "progress";

export type BadgeKind =
  | { kind: "count"; count: number }
  | { kind: "dot" }
  | { kind: "status"; status: BadgeStatus; value?: number }
  | { kind: "countWithStatus"; count: number; status: BadgeStatus };

export const badges = {
  /** Replaces any previous variant. Mutually exclusive per app. */
  async set(badge: BadgeKind): Promise<void> {
    return invoke(`${PLUGIN}|badges_set`, { badge });
  },
  async clear(): Promise<void> {
    return invoke(`${PLUGIN}|badges_clear`);
  },
};

// ── shell.ambient ─────────────────────────────────────────────────

export type AmbientEffect = "pulse" | "tint";
export type AmbientColor = "accent" | "warning" | "error" | "success";
export type AmbientSpeed = "slow" | "medium" | "fast";

export interface AmbientParams {
  effect: AmbientEffect;
  /**
   * Token-system color name. Hex / arbitrary CSS values are
   * not permitted; the renderer maps these to
   * `var(--color-{accent|warning|error|success})`.
   */
  color: AmbientColor;
  /** Hard-capped at 0.5 SDK-side. Negative values clamp to 0. */
  intensity: number;
  speed: AmbientSpeed;
  /** Free-form, for debug. Not rendered. */
  reason?: string;
  /** Shell-side autoClear timer in milliseconds. */
  autoClearMs?: number;
}

export const ambient = {
  async set(params: AmbientParams): Promise<void> {
    return invoke(`${PLUGIN}|ambient_set`, { params });
  },
  async clear(): Promise<void> {
    return invoke(`${PLUGIN}|ambient_clear`);
  },
};

/** Aggregate matching foundation §6.4 (`shell.{presence,timeline,spatial,annotations,toolbar,shortcuts,badges,ambient}`). */
export const shell = {
  presence,
  timeline,
  spatial,
  annotations,
  toolbar,
  shortcuts,
  badges,
  ambient,
};
