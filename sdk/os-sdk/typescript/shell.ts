/**
 * `shell.*` — first-party app TypeScript surface for the Lunaris OS SDK.
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
 *     import { shell } from "@lunaris/os-sdk/typescript/shell";
 *
 *     await shell.menu.register({ items: [...] });
 *     await shell.presence.set({ activity: "editing", subject: "x.md" });
 *     await shell.timeline.record({ type: "export", label: "Exported PDF", ... });
 */

import { invoke } from "@tauri-apps/api/core";

// ── shell.menu ─────────────────────────────────────────────────────────
//
// Wired end-to-end. Tauri commands `register_menu`, `unregister_menu`,
// `set_menu_state`, `get_menu`, `dispatch_menu_action` live in
// desktop-shell/src-tauri/src/menu_store.rs. Foundation §712-783.

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
  /** App identifier. Defaults to `LUNARIS_APP_ID` env var on the Rust side. */
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
    return invoke("register_menu", {
      appId: options.appId ?? "unknown",
      items: options.items,
    });
  },

  /** Remove this app's menu from the global menu bar. */
  async unregister(appId?: string): Promise<void> {
    return invoke("unregister_menu", { appId: appId ?? "unknown" });
  },

  /**
   * Update a single item's runtime state by action identifier.
   * Item not found is silently ignored — see foundation §776.
   */
  async setState(action: string, state: MenuStatePatch, appId?: string): Promise<void> {
    return invoke("set_menu_state", {
      appId: appId ?? "unknown",
      action,
      state,
    });
  },

  /** Get the current menu tree for an app (used for shell rehydration). */
  async get(appId: string): Promise<MenuItem[] | null> {
    return invoke("get_menu", { appId });
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
    return invoke("shell_presence_set", { params });
  },
  async clear(): Promise<void> {
    return invoke("shell_presence_clear");
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
    return invoke("shell_timeline_record", { params });
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
