/**
 * @lunaris/tauri-plugin-shell
 *
 * TypeScript surface for the Lunaris OS shell.* APIs. The Rust plugin
 * registers `plugin:lunaris-shell|*` commands; this module provides a
 * typed wrapper that mirrors foundation §6 (presence, timeline,
 * spatial). `shell.menu` is **not** here — it lives in desktop-shell
 * because menus are global state owned by the shell.
 *
 * # Usage
 *
 * ```typescript
 * import { shell } from "@lunaris/tauri-plugin-shell";
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
// The plugin registers commands under the `lunaris-shell` namespace.
// Tauri exposes them as `plugin:lunaris-shell|<command>`.

const PLUGIN = "plugin:lunaris-shell";

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
    const eventName = `lunaris://annotation-changed/${subscriptionId}`;
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

/** Aggregate matching foundation §316 (`shell.{presence,timeline,spatial,annotations}`). */
export const shell = { presence, timeline, spatial, annotations };
