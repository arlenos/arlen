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

import { get, writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { tauriAvailable } from "$lib/tauri";

/// How well-supported the app is - stated honestly.
export type CompatTier = "curated" | "best-effort";

/// What the confined Windows app can reach - the sovereign angle, surfaced honestly.
export interface BottleAccess {
  network: boolean;
  homeFolder: boolean;
}

/// One drive letter inside the bottle and the folder behind it. The daemon's
/// drive table IS the file boundary: a Windows app sees exactly these letters
/// and nothing else. `path: null` is the system drive - the app's own world,
/// not a folder of the user's.
export interface BottleDrive {
  letter: string;
  path: string | null;
}

/// The daemon's prefix-vs-description check, shown ONLY when it disagrees:
/// a bottle whose links lead outside itself has wider reach than its drive
/// table claims, and that is the one thing this panel must never overstate.
export interface BottleHealth {
  agrees: boolean;
  escapes: number;
}

/// One Windows app in its bottle, as the panel renders it. The first few fields
/// drive the thin default row; the rest are the Advanced depth.
export interface Bottle {
  id: string;
  /// What the bottle knows about itself, from the runtime.
  access: BottleAccess;
  /// The drives the bottle maps, from the daemon's dosdevices table.
  drives: BottleDrive[];

  // EVERYTHING BELOW IS OPTIONAL, and the question mark is the honest part
  // rather than a convenience. A bottle knows its id, what it may reach and
  // which folders it was granted; it does not know its Wine version, its DLL
  // overrides, its winetricks verbs, DXVK, scaling or a window mode. Those come
  // from the compat recipe (`windows-apps-plan.md` lists it as its own piece,
  // forage-distributed and signed), which does not exist yet - so nothing on
  // this machine has measured them and the panel must render an absent field as
  // not-measured rather than as a value. They were required fields filled by a
  // fixture until 24 August, which is the same shape as a task manager
  // reporting memory it never read.
  appName?: string;
  appId?: string;
  /// The compat-recipe that configured this bottle (or the default bottle).
  recipe?: string;
  tier?: CompatTier;
  wineVersion?: string;
  /// The Windows version the app is told it is running on.
  windowsVersion?: "7" | "10" | "11";
  /// The DLL overrides + winetricks verbs the recipe applied (editable).
  dllOverrides?: string[];
  winetricks?: string[];
  launchArgs?: string;
  workingDir?: string;
  /// Environment variables as "KEY=value" tokens.
  envVars?: string[];
  /// Translate Direct3D to Vulkan (DXVK) for better graphics performance.
  dxvk?: boolean;
  /// Display scaling as a percentage.
  scaling?: number;
  windowMode?: "windowed" | "fullscreen";
  /// Human-readable disk usage of the bottle, e.g. "1.2 GB".
  diskUsage?: string;
  /// Whether the app follows the Arlen theme (wine-theming-plan.md).
  followsTheme?: boolean;
}

/// What `list_bottles` answers: the bottles that read, and the ones that did not.
interface BottleListing {
  bottles: {
    id: string;
    network: boolean;
    homeFolder: boolean;
    drives: BottleDrive[];
  }[];
  unreadable: string[];
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
  /// True when a real session could not read the bottles at all.
  unavailable: boolean;
  /// Bottles that are on disk and did not read, by id.
  ///
  /// Kept apart from `bottles` because "you have none" and "one of yours is
  /// broken" are different sentences, and only the second is one somebody can do
  /// something about. Silently, a bottle whose description will not parse simply
  /// vanishes from the list.
  unreadable: string[];
}

const FIXTURE: Bottle[] = [
  {
    id: "b1",
    appName: "Notepad++",
    appId: "notepad-plus-plus",
    recipe: "Notepad++",
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
    drives: [{ letter: "C", path: null }],
  },
  {
    id: "b2",
    appName: "Paint.NET",
    appId: "paint-net",
    recipe: "Paint.NET",
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
    drives: [
      { letter: "C", path: null },
      { letter: "D", path: "/home/mara/Pictures" },
    ],
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
    drives: [
      { letter: "C", path: null },
      { letter: "D", path: "/home/mara" },
    ],
  },
];

/// The fixture's one unhealthy bottle: the deviating state has to be designable
/// without a daemon that can produce it on demand.
const FIXTURE_HEALTH: Record<string, BottleHealth> = {
  b1: { agrees: true, escapes: 0 },
  b2: { agrees: true, escapes: 0 },
  b3: { agrees: false, escapes: 2 },
};

/// The Wine/Proton versions the selectors offer.
export const wineVersions = ["Wine 9.0", "Wine 8.21", "Proton 9.0", "Wine (staging)"];

export const winApps = writable<WinAppsState>({
  bottles: [],
  loading: false,
  mocked: false,
  unavailable: false,
  unreadable: [],
});

/// True when the last change to a bottle or a default did not reach the daemon,
/// so the switches went back to what the bottle really holds. A Windows app's
/// config decides what that app can reach on this machine, so a switch showing
/// one thing while the prefix holds another is a claim about containment.
export const winActionFailed = writable(false);

/// The Windows-compatibility defaults.
///
/// The runtime list starts EMPTY, and that is the fix rather than an oversight:
/// it used to open with "Wine 9.0 installed, Proton 9.0 installed, DXVK 2.4
/// installed" as a hardcoded initial value, so every session - including a real
/// one on a machine with no Wine at all - stated which runtimes were on the
/// disk. Nothing ever corrected it, because the command that would read them has
/// no backend.
///
/// `version` and `bottleMode` are preferences rather than observations, so a
/// default for them is a real default; a runtime list is an observation, and
/// there is no honest default for one of those.
export const defaults = writable<WinDefaults>({
  version: "Wine 9.0",
  bottleMode: "per-app",
  runtimes: !tauriAvailable
    ? [
        { name: "Wine 9.0", installed: true },
        { name: "Proton 9.0", installed: true },
        { name: "DXVK 2.4", installed: true },
        { name: "Wine 8.21", installed: false },
      ]
    : [],
});

/// Load the bottles. Live: `list_bottles`; fixture under vite.
export async function load(): Promise<void> {
  winApps.update((s) => ({ ...s, loading: true }));
  try {
    const listing = await invoke<BottleListing>("list_bottles");
    // Mapped rather than passed through: the runtime answers what a bottle
    // knows, and the fields it does not know stay ABSENT here. Filling them with
    // plausible values would put a Wine version and a scaling percentage on
    // screen that nothing on this machine measured.
    winApps.set({
      bottles: listing.bottles.map((b) => ({
        id: b.id,
        appId: b.id,
        access: { network: b.network, homeFolder: b.homeFolder },
        drives: b.drives,
      })),
      unreadable: listing.unreadable,
      loading: false,
      mocked: false,
      unavailable: false,
    });
  } catch {
    if (!tauriAvailable) {
      winApps.set({
        bottles: FIXTURE,
        loading: false,
        mocked: true,
        unavailable: false,
        unreadable: [],
      });
      return;
    }
    // Each bottle carries live config controls through `patchBottle`, so an
    // invented one is a row of switches writing to a bottle that does not exist.
    winApps.set({
      bottles: [],
      loading: false,
      mocked: false,
      unavailable: true,
      unreadable: [],
    });
  }
}

/// Change any of a bottle's config, optimistically. Live: `set_bottle_config`.
export async function patchBottle(id: string, patch: Partial<Bottle>): Promise<void> {
  const before = get(winApps).bottles.find((b) => b.id === id);
  winApps.update((s) => ({
    ...s,
    bottles: s.bottles.map((b) => (b.id === id ? { ...b, ...patch } : b)),
  }));
  winActionFailed.set(false);
  try {
    await invoke("set_bottle_config", { id, patch });
  } catch {
    if (!tauriAvailable) return; // no host, so no daemon to ask
    if (before) {
      winApps.update((s) => ({
        ...s,
        bottles: s.bottles.map((b) => (b.id === id ? before : b)),
      }));
    }
    winActionFailed.set(true);
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
  const before = get(winApps).bottles;
  winApps.update((s) => ({ ...s, bottles: s.bottles.filter((b) => b.id !== id) }));
  winActionFailed.set(false);
  try {
    await invoke("delete_bottle", { id });
  } catch {
    if (!tauriAvailable) return;
    // The app and its prefix are still on disk; a list that hides them is a
    // machine the user thinks is cleaner than it is.
    winApps.update((s) => ({ ...s, bottles: before }));
    winActionFailed.set(true);
  }
}

/// Why a launch did not happen, and to which app.
///
/// THE REASON TRAVELS AS A TOKEN, and the window writes the sentence. The runtime
/// keeps five refusals apart - nothing is installed in this bottle yet, this
/// machine has no Wine, the bottle's prefix was never made, the granted folders
/// are not reachable, the confinement would not start - and they used to arrive
/// as one message saying the service "did not take the request", which is true of
/// exactly one of them.
export const launchFailed = writable<{ name: string; reason: string } | null>(null);

/// The message key for a refusal token, and the catch-all for one nobody knows.
export function launchFailureKey(reason: string): string {
  switch (reason) {
    case "nothing-to-run":
      return "s.wa.launchNothing";
    case "no-wine":
      return "s.wa.launchNoWine";
    case "prefix-missing":
      return "s.wa.launchNoPrefix";
    case "drives-unmet":
      return "s.wa.launchDrives";
    default:
      return "s.wa.launchFailed";
  }
}

/// Start the Windows app. Live: `launch_windows_app` (the daemon owns the
/// process, so it outlives this window). Under vite the fixture app has nothing
/// to start, so the button is inert rather than pretending.
export async function launchApp(id: string): Promise<void> {
  launchFailed.set(null);
  try {
    await invoke("launch_windows_app", { id });
  } catch (e) {
    if (!tauriAvailable) return;
    const name = get(winApps).bottles.find((b) => b.id === id)?.appName ?? id;
    launchFailed.set({ name, reason: String(e) });
  }
}

/// Read the daemon's prefix-vs-description check for one bottle. Live:
/// `bottle_health` (the daemon already answers this on its socket; the bridge
/// is the seam). `null` means the check could not be read - which is NOT the
/// same as healthy, so the caller shows nothing rather than a green light.
export async function bottleHealth(id: string): Promise<BottleHealth | null> {
  try {
    const h = await invoke<BottleHealth>("bottle_health", { id });
    return h;
  } catch {
    if (!tauriAvailable) return FIXTURE_HEALTH[id] ?? null;
    return null;
  }
}

/// Change a global default, optimistically. Live: `set_windows_defaults`.
export async function patchDefaults(patch: Partial<WinDefaults>): Promise<void> {
  const before = get(defaults);
  defaults.update((d) => ({ ...d, ...patch }));
  winActionFailed.set(false);
  try {
    await invoke("set_windows_defaults", { patch });
  } catch {
    if (!tauriAvailable) return;
    defaults.set(before);
    winActionFailed.set(true);
  }
}
