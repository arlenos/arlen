/// Windows apps / Compatibility (windows-apps-plan.md): Windows apps run in managed
/// Wine bottles. A compat-recipe database auto-configures the bottle for KNOWN apps
/// (the right Wine version, DLL overrides, winetricks) so the user never fiddles -
/// "handled, not your fight". The default view is thin (compat tier + install); the
/// Advanced expand carries real Bottles-level depth on demand, and the sovereign
/// angle (what a Windows app can reach) leads it.
///
/// The honesty discipline: the compat tier is labelled honestly - curated-verified
/// vs best-effort - never implying "everything just works".
///
/// Mock-vs-live: the whole backend (the bottle daemon, wine-proton-plan.md) is
/// build-deferred, so everything is a coder seam; under vite the store serves a
/// fixture and flags `mocked` for the honest banner (the Printers pattern).

import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// How well-supported the app is - stated honestly.
export type CompatTier = "curated" | "best-effort";

/// What the confined Windows app can reach - the sovereign angle, surfaced honestly.
export interface BottleAccess {
  network: boolean;
  homeFolder: boolean;
}

/// One Windows app in its bottle, as the panel renders it. The first few fields
/// drive the thin default row; the rest are the Advanced depth.
export interface Bottle {
  id: string;
  appName: string;
  appId: string;
  /// The compat-recipe that configured this bottle (or the default bottle).
  recipe: string;
  tier: CompatTier;
  wineVersion: string;
  /// The Windows version the app is told it is running on.
  windowsVersion: "7" | "10" | "11";
  /// The DLL overrides + winetricks verbs the recipe applied (editable).
  dllOverrides: string[];
  winetricks: string[];
  launchArgs: string;
  workingDir: string;
  /// Environment variables as "KEY=value" tokens.
  envVars: string[];
  /// Translate Direct3D to Vulkan (DXVK) for better graphics performance.
  dxvk: boolean;
  /// Display scaling as a percentage.
  scaling: number;
  windowMode: "windowed" | "fullscreen";
  /// Human-readable disk usage of the bottle, e.g. "1.2 GB".
  diskUsage: string;
  /// Whether the app follows the Arlen theme (wine-theming-plan.md).
  followsTheme: boolean;
  access: BottleAccess;
}

/// Global cross-bottle defaults + installed runtimes.
export interface WinDefaults {
  version: string;
  bottleMode: "per-app" | "shared";
  runtimes: { name: string; installed: boolean }[];
}

interface WinAppsState {
  bottles: Bottle[];
  loading: boolean;
  mocked: boolean;
}

const FIXTURE: Bottle[] = [
  {
    id: "b1",
    appName: "Notepad++",
    appId: "notepad-plus-plus",
    recipe: "Notepad++ recipe",
    tier: "curated",
    wineVersion: "Wine 9.0",
    windowsVersion: "10",
    dllOverrides: ["msftedit = native"],
    winetricks: ["corefonts"],
    launchArgs: "",
    workingDir: "",
    envVars: [],
    dxvk: false,
    scaling: 100,
    windowMode: "windowed",
    diskUsage: "480 MB",
    followsTheme: true,
    access: { network: false, homeFolder: false },
  },
  {
    id: "b2",
    appName: "Paint.NET",
    appId: "paint-net",
    recipe: "Paint.NET recipe",
    tier: "curated",
    wineVersion: "Wine 9.0",
    windowsVersion: "10",
    dllOverrides: ["d3dcompiler_47 = native"],
    winetricks: ["dotnet48", "corefonts"],
    launchArgs: "",
    workingDir: "",
    envVars: [],
    dxvk: true,
    scaling: 100,
    windowMode: "windowed",
    diskUsage: "1.2 GB",
    followsTheme: true,
    access: { network: true, homeFolder: false },
  },
  {
    id: "b3",
    appName: "LegacyTool.exe",
    appId: "legacytool",
    recipe: "Default bottle",
    tier: "best-effort",
    wineVersion: "Wine 9.0",
    windowsVersion: "7",
    dllOverrides: [],
    winetricks: [],
    launchArgs: "",
    workingDir: "",
    envVars: [],
    dxvk: false,
    scaling: 100,
    windowMode: "windowed",
    diskUsage: "320 MB",
    followsTheme: false,
    access: { network: true, homeFolder: true },
  },
];

/// The Wine/Proton versions the selectors offer.
export const wineVersions = ["Wine 9.0", "Wine 8.21", "Proton 9.0", "Wine (staging)"];

export const winApps = writable<WinAppsState>({ bottles: [], loading: false, mocked: false });

export const defaults = writable<WinDefaults>({
  version: "Wine 9.0",
  bottleMode: "per-app",
  runtimes: [
    { name: "Wine 9.0", installed: true },
    { name: "Proton 9.0", installed: true },
    { name: "DXVK 2.4", installed: true },
    { name: "Wine 8.21", installed: false },
  ],
});

/// Load the bottles. Live: `list_bottles`; fixture under vite.
export async function load(): Promise<void> {
  winApps.update((s) => ({ ...s, loading: true }));
  try {
    const bottles = await invoke<Bottle[]>("list_bottles");
    winApps.set({ bottles, loading: false, mocked: false });
  } catch {
    winApps.set({ bottles: FIXTURE, loading: false, mocked: true });
  }
}

/// Change any of a bottle's config, optimistically. Live: `set_bottle_config`.
export async function patchBottle(id: string, patch: Partial<Bottle>): Promise<void> {
  winApps.update((s) => ({
    ...s,
    bottles: s.bottles.map((b) => (b.id === id ? { ...b, ...patch } : b)),
  }));
  try {
    await invoke("set_bottle_config", { id, patch });
  } catch {
    // optimistic in the mock
  }
}

/// Install a new Windows app. Live: a file-pick (a .exe or .msi installer) ->
/// the install command sets up a bottle.
export async function installExe(): Promise<void> {
  try {
    await invoke("install_windows_app");
  } catch {
    // No bottle daemon under vite: the escape hatch is inert in the mock.
  }
}

/// Open the app's C: drive (its Wine prefix) in the file manager. Live seam.
export async function browseFiles(id: string): Promise<void> {
  try {
    await invoke("browse_bottle_files", { id });
  } catch {
    // seam
  }
}

/// Clear the bottle's shader/font caches to reclaim space. Live seam.
export async function clearCaches(id: string): Promise<void> {
  try {
    await invoke("clear_bottle_caches", { id });
  } catch {
    // seam
  }
}

/// Remove the app + its bottle. Live: `delete_bottle`.
export async function deleteBottle(id: string): Promise<void> {
  winApps.update((s) => ({ ...s, bottles: s.bottles.filter((b) => b.id !== id) }));
  try {
    await invoke("delete_bottle", { id });
  } catch {
    // optimistic in the mock
  }
}

/// Change a global default, optimistically. Live: `set_windows_defaults`.
export async function patchDefaults(patch: Partial<WinDefaults>): Promise<void> {
  defaults.update((d) => ({ ...d, ...patch }));
  try {
    await invoke("set_windows_defaults", { patch });
  } catch {
    // optimistic in the mock
  }
}
