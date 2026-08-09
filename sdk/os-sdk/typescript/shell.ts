/**
 * `shell.*` — first-party app TypeScript surface for the Arlen OS SDK.
 *
 * Mirrors the foundation §6 Shell API spec for the subset that has
 * landed: `shell.menu` is wired end-to-end against desktop-shell's
 * existing Tauri commands. `shell.presence`, `shell.timeline`, and
 * `shell.spatial` are sketched here and require apps to register the
 * matching Tauri commands in their own `src-tauri` until the
 * `tauri-plugin-shell` follow-up lands. Each method documents the
 * exact command signature an app must implement.
 *
 * Usage in a Tauri app:
 *
 *     import { shell } from "@arlen/os-sdk/typescript/shell";
 *
 *     await shell.menu.register({ items: [...] });
 *     await shell.presence.set({ activity: "editing", subject: "x.md" });
 *     await shell.timeline.record({ type: "export", label: "Exported PDF", ... });
 */

import { invoke } from "@tauri-apps/api/core";

// ── shell.menu ─────────────────────────────────────────────────────────
//
// Those command names live in the SHELL's own binary
// (desktop-shell/src-tauri/src/menu_store.rs), and a Tauri command is reachable
// only inside the binary that registers it - so an app invoking them by those
// names is rejected at runtime, which is what `tauri-plugin-shell` says beside
// its own `menu_register`. The plugin is the app-facing half, and a plugin
// command is addressed `plugin:arlen-shell|<name>`, not bare.
//
// This file said "wired end-to-end" and named the shell's spelling, so an app
// following it got a throw per call. The harness did exactly that, which is why
// `register_menu` sits in the missing-command inventory under the harness's
// name rather than under this file's. Foundation §712-783.

/** Single menu item or separator. Items can nest via `children`. */
export interface MenuItem {
  /** Display label. Omitted when `separator` is true. */
  label?: string;
  /** Opaque action identifier dispatched back to the app on activation. */
  action?: string;
  /** Optional keyboard shortcut display string, e.g. "Ctrl+S". */
  shortcut?: string;
  /** When false, the item renders disabled and cannot be activated. */
  enabled?: boolean;
  /** For toggle/radio items: current checked state. */
  checked?: boolean;
  /** Item type. `recent` is a system-filled slot from the Knowledge Graph. */
  type?: "command" | "toggle" | "radio" | "recent";
  /** Group identifier for radio items. */
  group?: string;
  /** For `type: "recent"`: which graph node type to surface. */
  node_type?: string;
  /** For `type: "recent"`: maximum entries. */
  limit?: number;
  /** Foundation §794 context tags — surface marker when matching focus. */
  context?: string[];
  /** Submenu items. Mutually exclusive with `action`. */
  children?: MenuItem[];
  /** When true, render a horizontal divider. All other fields ignored. */
  separator?: boolean;
}

export interface MenuRegisterOptions {
  /** App identifier. Defaults to `ARLEN_APP_ID` env var on the Rust side. */
  appId?: string;
  /** Top-level menu structure. */
  items: MenuItem[];
}

export interface MenuStatePatch {
  enabled?: boolean;
  label?: string;
  checked?: boolean;
}

export const menu = {
  /** Register or replace this app's global menu. */
  async register(options: MenuRegisterOptions): Promise<void> {
    // The plugin takes the groups and resolves the caller itself - passing an
    // app id was the shell-internal shape, where the caller is another process.
    return invoke("plugin:arlen-shell|menu_register", { groups: options.items });
  },

  /** Remove this app's menu from the global menu bar. */
  async unregister(appId?: string): Promise<void> {
    void appId;
    return invoke("plugin:arlen-shell|menu_unregister");
  },

  /**
   * Update a single item's runtime state by action identifier.
   *
   * NOT AVAILABLE TO AN APP. `tauri-plugin-shell` exposes no counterpart, so
   * there is no name this could be called under; the shell's `set_menu_state`
   * is internal to its own binary. Kept declared rather than silently removed
   * because the shape is the contract the plugin would implement, and left
   * rejecting rather than pretending: re-registering the whole menu is what an
   * app can do today.
   */
  async setState(action: string, state: MenuStatePatch, appId?: string): Promise<void> {
    void action;
    void state;
    void appId;
    return Promise.reject(
      new Error("shell.menu.setState: no app-facing command; re-register the menu instead"),
    );
  },

  /** Get the current menu tree for an app. Same as `setState`: shell-internal,
   * with no plugin counterpart an app could reach. */
  async get(appId: string): Promise<MenuItem[] | null> {
    void appId;
    return Promise.reject(new Error("shell.menu.get: no app-facing command"));
  },
};

// ── shell.presence ─────────────────────────────────────────────────────
//
// Rust-side shipped (sdk/os-sdk/src/presence.rs). The TS wrapper here
// requires apps to register the matching Tauri commands until
// `tauri-plugin-shell` provides them automatically:
//
//     #[tauri::command]
//     async fn shell_presence_set(state: State<'_, Arc<Presence<UnixEventEmitter>>>,
//                                 params: PresenceParams) -> Result<(), String> {
//         state.set(params).await.map_err(|e| e.to_string())
//     }
//     // and similarly shell_presence_clear

export type AutoClear = "on-blur" | "on-idle" | "manual";

export interface PresenceParams {
  /** "editing" | "reading" | "reviewing" | "building" | custom verb */
  activity: string;
  /** Free-form subject — typically a file path, document name, or URL. */
  subject: string;
  /** Optional project context. Empty inherits Focus Mode. */
  project?: string;
  /** Free-form structured context. Stays in the SQLite event log. */
  metadata?: Record<string, string>;
  /** Default `manual` (caller calls `clear` explicitly). */
  auto_clear?: AutoClear;
}

export const presence = {
  async set(params: PresenceParams): Promise<void> {
    return invoke("plugin:arlen-shell|presence_set", { params });
  },
  async clear(): Promise<void> {
    return invoke("plugin:arlen-shell|presence_clear");
  },
};

// ── shell.timeline ─────────────────────────────────────────────────────
//
// Rust-side shipped (sdk/os-sdk/src/timeline.rs). Apps register the
// `shell_timeline_record` Tauri command that calls Timeline::record.

export interface TimelineParams {
  /** User-facing summary, e.g. "Exported PDF". */
  label: string;
  /** File path, project name, or URL. */
  subject: string;
  /** App-defined category like "export" | "build" | "deploy" | "save". */
  type: string;
  /** Microseconds since Unix epoch. Omit for point-in-time events. */
  started_at?: number;
  /** Microseconds since Unix epoch. Omit for point-in-time events. */
  ended_at?: number;
  /** Free-form structured context. Stays in the SQLite event log. */
  metadata?: Record<string, string>;
}

export const timeline = {
  async record(params: TimelineParams): Promise<void> {
    return invoke("plugin:arlen-shell|timeline_record", { params });
  },
};

// ── shell.spatial ──────────────────────────────────────────────────────
//
// Rust-side stub shipped (sdk/os-sdk/src/spatial.rs). Per foundation
// §634 the call is "accepted and silently ignored" until the
// compositor-side extension lands. Apps can call this today and
// receive real behaviour without code changes later.

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

export const spatial = {
  async hint(_h: SpatialHint): Promise<void> {
    // No-op until compositor extension lands. The Tauri command is
    // optional today; we no-op locally so apps don't need any
    // backend wiring for spatial yet.
    return Promise.resolve();
  },
};

/** Convenience aggregate matching foundation §316: `shell.{menu,presence,timeline,spatial}`. */
export const shell = { menu, presence, timeline, spatial };
