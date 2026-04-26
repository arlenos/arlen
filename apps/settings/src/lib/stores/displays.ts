/// Display panel state.
///
/// Subscribes to `displays:changed` Tauri events from the wayland
/// thread and republishes them as a Svelte store. The frontend
/// interacts with the compositor exclusively through these
/// commands; raw Tauri-event listening should not happen in
/// component code.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { writable, type Writable } from "svelte/store";

// ---------------------------------------------------------------------------
// Types — mirror `app-settings/src-tauri/src/displays/types.rs`. Kept
// in sync by hand for now; if the surface grows, ts-rs would be the
// right fix.
// ---------------------------------------------------------------------------

export interface MonitorMode {
  width: number;
  height: number;
  /** Refresh rate in milli-Hertz. 60_000 == 60 Hz. */
  refreshMhz: number;
  preferred: boolean;
}

export interface Position {
  x: number;
  y: number;
}

export type Transform =
  | "normal"
  | "rotate-90"
  | "rotate-180"
  | "rotate-270"
  | "flipped"
  | "flipped-90"
  | "flipped-180"
  | "flipped-270";

export type VrrState = "enabled" | "disabled" | "force";

export type EnabledKind =
  | { type: "active" }
  | { type: "disabled" }
  | { type: "mirror"; target: string };

export interface Monitor {
  connector: string;
  make: string;
  model: string;
  serial: string;
  physicalSizeMm: [number, number];
  modes: MonitorMode[];
  currentMode: number | null;
  preferredMode: number | null;
  position: Position;
  scale: number;
  transform: Transform;
  enabled: boolean;
  mirroring: string | null;
  vrr: VrrState;
  primary: boolean;
  maxBpc: number;
}

export interface MonitorConfig {
  connector: string;
  modeIndex: number | null;
  position: Position;
  scale: number;
  transform: Transform;
  enabled: EnabledKind;
  vrr: VrrState;
  primary: boolean;
  maxBpc: number;
}

export interface ApplyHandle {
  requestId: string;
  snapshot: MonitorConfig[];
}

export interface ApplyResult {
  requestId: string;
  outcome: "succeeded" | "failed" | "cancelled";
}

// ---------------------------------------------------------------------------
// Stores
// ---------------------------------------------------------------------------

/// Live monitor list as published by the wayland thread.
export const monitors: Writable<Monitor[]> = writable([]);

/// Connector of the user's currently-selected monitor in the UI.
/// `null` means "no selection yet" (first mount before the canvas
/// auto-picks the first output).
export const selectedConnector: Writable<string | null> = writable(null);

/// Apply-result stream for whichever apply request is currently
/// in flight. The revert modal subscribes to this and decides
/// whether to keep showing the countdown or close itself.
export const lastApplyResult: Writable<ApplyResult | null> = writable(null);

// ---------------------------------------------------------------------------
// Initialisation
// ---------------------------------------------------------------------------

let unlistenChanged: UnlistenFn | null = null;
let unlistenResult: UnlistenFn | null = null;

/// Mount the listeners and pull the initial monitor list. Call once
/// from the display page's `onMount`. Returns a teardown function
/// the caller MUST invoke from `onDestroy`.
export async function initDisplayStore(): Promise<() => void> {
  // First load: synchronously snapshot whatever the wayland thread
  // already published. This avoids a flash of "no monitors" before
  // the first event fires.
  try {
    const initial = await invoke<Monitor[]>("display_get_monitors");
    monitors.set(initial);
    if (initial.length > 0) {
      selectedConnector.update((cur) => cur ?? initial[0]!.connector);
    }
  } catch (err) {
    console.warn("[displays] initial fetch failed:", err);
  }

  unlistenChanged = await listen<Monitor[]>("displays:changed", (ev) => {
    monitors.set(ev.payload);
    // If the previously-selected monitor was removed (hot-unplug),
    // fall back to the first remaining one. Without this, the side
    // panel would render against a stale connector and every apply
    // would silently target nothing.
    selectedConnector.update((cur) => {
      if (!cur) return ev.payload[0]?.connector ?? null;
      return ev.payload.some((m) => m.connector === cur)
        ? cur
        : (ev.payload[0]?.connector ?? null);
    });
  });

  unlistenResult = await listen<ApplyResult>("displays:apply-result", (ev) => {
    lastApplyResult.set(ev.payload);
  });

  return () => {
    unlistenChanged?.();
    unlistenResult?.();
    unlistenChanged = null;
    unlistenResult = null;
  };
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

export async function applyConfig(config: MonitorConfig[]): Promise<ApplyHandle> {
  return await invoke<ApplyHandle>("display_apply_config", { config });
}

export async function revertConfig(snapshot: MonitorConfig[]): Promise<string> {
  return await invoke<string>("display_revert", { snapshot });
}

export async function saveCurrent(): Promise<void> {
  await invoke("display_save_current");
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert a `Monitor` (server snapshot) into a `MonitorConfig`
/// (client-side draft). The side panel calls this when the user
/// starts editing a monitor: edits flow into a draft copy, the
/// original snapshot stays available for revert.
export function monitorToConfig(m: Monitor): MonitorConfig {
  let enabled: EnabledKind;
  if (!m.enabled) {
    enabled = { type: "disabled" };
  } else if (m.mirroring) {
    enabled = { type: "mirror", target: m.mirroring };
  } else {
    enabled = { type: "active" };
  }
  return {
    connector: m.connector,
    modeIndex: m.currentMode,
    position: m.position,
    scale: m.scale,
    transform: m.transform,
    enabled,
    vrr: m.vrr,
    primary: m.primary,
    maxBpc: m.maxBpc,
  };
}

/// Format `MonitorMode` as `2560 × 1440 @ 60.00 Hz`.
export function formatMode(mode: MonitorMode): string {
  const hz = (mode.refreshMhz / 1000).toFixed(2);
  return `${mode.width} × ${mode.height} @ ${hz} Hz`;
}

/// Group modes by resolution so the resolution dropdown shows each
/// (width, height) exactly once and the refresh-rate dropdown
/// derives its options from the selected resolution.
export function groupedResolutions(
  modes: MonitorMode[],
): { width: number; height: number; refreshOptions: MonitorMode[] }[] {
  const groups = new Map<string, { width: number; height: number; refreshOptions: MonitorMode[] }>();
  for (const m of modes) {
    const key = `${m.width}x${m.height}`;
    let g = groups.get(key);
    if (!g) {
      g = { width: m.width, height: m.height, refreshOptions: [] };
      groups.set(key, g);
    }
    g.refreshOptions.push(m);
  }
  // Sort: highest resolution first; refreshes within a group highest first.
  const out = Array.from(groups.values());
  out.sort((a, b) => b.width * b.height - a.width * a.height);
  for (const g of out) {
    g.refreshOptions.sort((a, b) => b.refreshMhz - a.refreshMhz);
  }
  return out;
}
