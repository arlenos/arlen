/// Windows apps / Compatibility (windows-apps-plan.md): Windows apps run in managed
/// Wine bottles. A compat-recipe database auto-configures the bottle for KNOWN apps
/// (the right Wine version, DLL overrides, winetricks) so the user never fiddles -
/// "handled, not your fight". This panel is mostly status + an escape hatch.
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

/// One Windows app in its bottle, as the panel renders it.
export interface Bottle {
  id: string;
  appName: string;
  appId: string;
  /// The compat-recipe that configured this bottle (or the default bottle).
  recipe: string;
  tier: CompatTier;
  wineVersion: string;
  /// The DLL overrides + winetricks the recipe applied (shown under Advanced).
  dllOverrides: string[];
  winetricks: string[];
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
    dllOverrides: ["msftedit = native"],
    winetricks: ["corefonts"],
  },
  {
    id: "b2",
    appName: "Paint.NET",
    appId: "paint-net",
    recipe: "Paint.NET recipe",
    tier: "curated",
    wineVersion: "Wine 9.0",
    dllOverrides: ["d3dcompiler_47 = native"],
    winetricks: ["dotnet48", "corefonts"],
  },
  {
    id: "b3",
    appName: "LegacyTool.exe",
    appId: "legacytool",
    recipe: "Default bottle",
    tier: "best-effort",
    wineVersion: "Wine 9.0",
    dllOverrides: [],
    winetricks: [],
  },
];

/// The Wine/Proton versions the Advanced selector offers.
export const wineVersions = ["Wine 9.0", "Wine 8.21", "Proton 9.0", "Wine (staging)"];

export const winApps = writable<WinAppsState>({ bottles: [], loading: false, mocked: false });

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

/// Install a new Windows app. Live: a file-pick -> `install_exe` sets up a bottle.
export async function installExe(): Promise<void> {
  try {
    await invoke("install_exe");
  } catch {
    // No bottle daemon under vite: the escape hatch is inert in the mock.
  }
}

/// Change the bottle's Wine/Proton version (the escape hatch). Live: `set_wine_version`.
export async function setWineVersion(id: string, version: string): Promise<void> {
  winApps.update((s) => ({
    ...s,
    bottles: s.bottles.map((b) => (b.id === id ? { ...b, wineVersion: version } : b)),
  }));
  try {
    await invoke("set_wine_version", { id, version });
  } catch {
    // optimistic in the mock
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
