/// Windows apps / Compatibility (windows-apps-plan.md): Windows apps run in managed
/// Wine bottles. A compat-recipe database auto-configures the bottle for KNOWN apps
/// (the right Wine version, DLL overrides, winetricks) so the user never fiddles -
/// "handled, not your fight". The default view is thin (compat tier + install); the
/// app's own page carries real Bottles-level depth on demand, and the sovereign
/// angle (what a Windows app can reach) leads it.
///
/// The honesty discipline: the compat tier is labelled honestly - curated-verified
/// vs best-effort - never implying "everything just works"; every answer the daemon
/// gives back (where a prefix went, how much a clear freed, why a launch was
/// refused) reaches the screen as a sentence, and every question it cannot answer
/// yet stays an absent field rather than a plausible value.
///
/// Mock-vs-live: under vite the store serves a fixture and flags `mocked` for the
/// honest banner (the Printers pattern); live, every call is one ask of the bottle
/// daemon through the settings bridge.

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
/// drive the thin default row; the rest are the depth on the app's page.
export interface Bottle {
  id: string;
  /// What the bottle knows about itself, from the runtime.
  access: BottleAccess;
  /// The drives the bottle maps, from the daemon's dosdevices table.
  drives: BottleDrive[];
  /// Whether somebody has said which program this bottle starts.
  ///
  /// False between an install finishing and the person picking the app out of
  /// what the installer left. A launch refuses while it is false, so the panel
  /// asks the question instead of letting them meet the refusal.
  hasProgram: boolean;

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
  /// Human-readable disk usage of the bottle, e.g. "1.2 GB". Nothing live
  /// measures it yet; the page says so instead of rendering the gap.
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
    hasProgram: boolean;
  }[];
  unreadable: string[];
}

/// What this machine can run Windows programs with: the runtimes that ARE here.
///
/// Only installed ones, because only those are measured. A row for a runtime
/// that could be installed needs a way to install it, and no host has one; a
/// button that does nothing is worse than no row.
export interface WinDefaults {
  runtimes: { name: string }[];
}

interface WinAppsState {
  bottles: Bottle[];
  /// True until the first listing answered. The list page holds its empty
  /// state back while this is set, so the "add one" invitation is never a
  /// flash before the real answer.
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
    hasProgram: true,
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
    hasProgram: true,
    drives: [
      { letter: "C", path: null },
      { letter: "D", path: "/home/mara/Pictures" },
    ],
  },
  // The one the installer just left: no recipe, no program picked, and a
  // prefix that reaches further than its table says. Every state the live
  // path produces after an install has to be designable from here.
  {
    id: "ledger-setup",
    access: { network: true, homeFolder: true },
    hasProgram: false,
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
  "ledger-setup": { agrees: false, escapes: 2 },
};

/// The Wine/Proton versions the selectors offer.
export const wineVersions = ["Wine 9.0", "Wine 8.21", "Proton 9.0", "Wine (staging)"];

export const winApps = writable<WinAppsState>({
  bottles: [],
  loading: true,
  mocked: false,
  unavailable: false,
  unreadable: [],
});

/// True when the last change to a bottle's config did not reach the daemon, so
/// the switches went back to what the bottle really holds. A Windows app's
/// config decides what that app can reach on this machine, so a switch showing
/// one thing while the prefix holds another is a claim about containment.
export const winActionFailed = writable(false);

/// The Windows-compatibility runtimes on this machine.
///
/// Opens EMPTY, and that is the fix rather than an oversight: it used to open
/// with "Wine 9.0 installed, Proton 9.0 installed, DXVK 2.4 installed" as a
/// hardcoded initial value, so every session - including a real one on a machine
/// with no Wine at all - stated which runtimes were on the disk. A runtime list
/// is an OBSERVATION, and there is no honest default for one of those, so this
/// opens empty and `runtimesKnown` says whether anybody has looked. Under vite
/// the fixture stands in for the look.
export const defaults = writable<WinDefaults>({
  runtimes: !tauriAvailable ? [{ name: "Wine 9.0" }, { name: "Proton 9.0" }, { name: "DXVK 2.4" }] : [],
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
        hasProgram: b.hasProgram,
      })),
      unreadable: listing.unreadable,
      loading: false,
      mocked: false,
      unavailable: false,
    });
  } catch {
    if (!tauriAvailable) {
      // The fixture keeps what the page did to it (a picked program, a
      // forgotten bottle) so the whole flow drives without a daemon.
      const kept = get(winApps);
      winApps.set({
        bottles: kept.mocked ? kept.bottles : FIXTURE,
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

/// Why an install could not start, or null.
///
/// A REASON, not a boolean, for the same argument `launchFailed` makes: the
/// daemon distinguishes "this machine has no Wine" from "something went wrong",
/// and a banner that flattens the two sends somebody looking for the wrong
/// problem.
export const installFailed = writable<string | null>(null);

/// The bottle whose installer was started from this window and has not had its
/// program picked yet. The install runs in the installer's own window and
/// nothing signals when it is done (a named seam), so the app's page says the
/// installer is running until the person picks what it left behind.
export const installStarted = writable<string | null>(null);

/// The message key for an install refusal token.
///
/// The bridge makes a bottle, then runs the installer in it, and the two refuse
/// differently. Making one can answer no-wine, bottle-exists, bad-id and
/// could-not-create; running the installer can answer no-installer, no-wine,
/// prefix-missing and could-not-start; and the bridge itself answers
/// unnamed-installer when the file's name gives no name for an app. Anything
/// else still gets a sentence - a token must never reach the screen.
export function installFailureKey(reason: string): string {
  switch (reason) {
    case "no-wine":
      return "s.wa.installNoWine";
    case "bottle-exists":
      return "s.wa.installExists";
    case "could-not-create":
      return "s.wa.installNotCreated";
    case "no-installer":
      return "s.wa.installNoFile";
    case "unnamed-installer":
      return "s.wa.installUnnamed";
    default:
      return "s.wa.installFailed";
  }
}

/// Install a new Windows app: pick an installer, get a bottle, run it there.
///
/// Answers the new bottle's id, or null when the picker was cancelled - which is
/// not a failure and must not raise the error banner. The list is reloaded after
/// EVERY attempt, refused ones included: the bottle is made before the installer
/// runs, so a refusal on the second step leaves a real bottle on disk, and a list
/// that does not show it is a machine the person thinks is emptier than it is.
export async function installExe(): Promise<string | null> {
  installFailed.set(null);
  try {
    const id = await invoke<string | null>("install_windows_app");
    if (id) {
      installStarted.set(id);
      await load();
    }
    return id;
  } catch (e) {
    // No bottle daemon under vite: the escape hatch is inert in the mock.
    if (!tauriAvailable) return null;
    installFailed.set(String(e));
    await load();
    return null;
  }
}

/// Which file action did not happen, and to which app. Its own store because
/// "your caches were not cleared" is not `winActionFailed`'s "your setting did
/// not save": nothing about the bottle's config was in question.
export const fileActionFailed = writable<{ name: string; action: "browse" | "clear" } | null>(null);

/// What the last cache clear freed, for the page to say. `null` until one ran.
export const cleared = writable<{ id: string; bytes: number; files: number } | null>(null);

/// Open the app's C: drive (its Wine prefix) in the file manager. Live:
/// `browse_bottle_files`.
export async function browseFiles(id: string): Promise<void> {
  fileActionFailed.set(null);
  try {
    await invoke("browse_bottle_files", { id });
  } catch {
    if (!tauriAvailable) return;
    fileActionFailed.set({ name: nameOf(id), action: "browse" });
  }
}

/// Clear the bottle's regenerable caches to reclaim space, and keep what the
/// daemon says it freed. Live: `clear_bottle_caches`; the fixture frees a
/// plausible amount so the answer is designable.
export async function clearCaches(id: string): Promise<void> {
  fileActionFailed.set(null);
  try {
    const r = await invoke<{ bytes: number; files: number }>("clear_bottle_caches", { id });
    cleared.set({ id, ...r });
  } catch {
    if (!tauriAvailable) {
      cleared.set({ id, bytes: 356_515_840, files: 212 });
      return;
    }
    fileActionFailed.set({ name: nameOf(id), action: "clear" });
  }
}

/// A byte count as the short size people read, "340 MB" not "356515840".
export function formatSize(bytes: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let v = bytes;
  let u = 0;
  while (v >= 1000 && u < units.length - 1) {
    v /= 1000;
    u += 1;
  }
  const digits = u === 0 ? 0 : v < 10 ? 1 : 0;
  return `${v.toFixed(digits)} ${units[u]}`;
}

/// The last bottle this window forgot, and whether its files went to the trash
/// or there were none on disk to move. `null` until one was forgotten; the list
/// says it once and clears it on the next action.
export const forgotten = writable<{ name: string; trashed: boolean } | null>(null);

/// Remove the app + its bottle. Live: `delete_bottle`, which answers where the
/// prefix went.
export async function deleteBottle(id: string): Promise<void> {
  const before = get(winApps).bottles;
  const name = nameOf(id);
  winApps.update((s) => ({ ...s, bottles: s.bottles.filter((b) => b.id !== id) }));
  forgetFailed.set(null);
  forgotten.set(null);
  try {
    const trashedTo = await invoke<string | null>("delete_bottle", { id });
    forgotten.set({ name, trashed: trashedTo !== null });
  } catch (e) {
    if (!tauriAvailable) {
      forgotten.set({ name, trashed: true });
      return;
    }
    // The app and its prefix are still on disk; a list that hides them is a
    // machine the user thinks is cleaner than it is.
    winApps.update((s) => ({ ...s, bottles: before }));
    forgetFailed.set({ name, reason: String(e) });
  }
  installStarted.update((s) => (s === id ? null : s));
}

/// Why a bottle was not forgotten, and which one.
///
/// Its own store rather than `winActionFailed`, which says a CONFIG change did not
/// stick. Two of these reasons are not that at all: the runtime may refuse the
/// caller outright, and it refuses when the ledger cannot record the removal -
/// "nothing happened because nobody could write it down" is a different thing to
/// tell somebody than "your setting did not save".
export const forgetFailed = writable<{ name: string; reason: string } | null>(null);

/// The message key for a refused forget.
///
/// The three the runtime can answer here, plus everything else. `bad-id` and
/// `unreadable` land on the generic sentence deliberately: to the person pressing
/// Remove they mean the same thing, that it is still on the machine.
export function forgetFailureKey(reason: string): string {
  switch (reason) {
    case "not-allowed":
      return "s.wa.forgetNotAllowed";
    case "could-not-forget":
      return "s.wa.forgetNotRecorded";
    case "no-such-bottle":
      return "s.wa.forgetGone";
    default:
      return "s.wa.forgetFailed";
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

/// The message key for a refusal token.
///
/// THE RUNTIME'S WHOLE VOCABULARY IS LISTED, including the ones that end at the
/// same sentence, so a reader can see the set rather than infer it from what is
/// missing. The runtime can answer: nothing-to-run, no-wine, prefix-missing,
/// drives-unmet, could-not-start, no-such-bottle, bad-id, unreadable, not-allowed,
/// could-not-forget, bottle-exists, could-not-create, not-in-this-bottle,
/// no-installer. A launch reaches the first five plus the three lookup ones; the
/// rest belong to forgetting, making and installing. A token this does not know
/// still gets a sentence, because a person reading the screen should never be
/// shown a token.
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
    // could-not-start, no-such-bottle, bad-id, unreadable: the app is gone or the
    // confinement would not come up, and "it did not start" is the honest summary
    // of both. Named here so their absence below is a decision, not an oversight.
    default:
      return "s.wa.launchFailed";
  }
}

/// Start the Windows app. Live: `launch_windows_app` (the daemon owns the
/// process, so it outlives this window). The pid it answers with is dropped on
/// purpose: without a way to ask whether that pid is still alive it would only
/// ever say "started once", and the app's own window already says that. Under
/// vite the fixture app has nothing to start, so the button is inert rather
/// than pretending.
export async function launchApp(id: string): Promise<void> {
  launchFailed.set(null);
  try {
    await invoke("launch_windows_app", { id });
  } catch (e) {
    if (!tauriAvailable) return;
    launchFailed.set({ name: nameOf(id), reason: String(e) });
  }
}

/// Read the daemon's prefix-vs-description check for one bottle. Live:
/// `bottle_health`. `null` means the check could not be read - which is NOT the
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

/// What the last reach change actually did.
///
/// `changed: false` is an ANSWER, not a failure: revoking a folder a bottle no
/// longer had is a thing somebody asked for and nothing happened, and saying so
/// beats a silent success that reads as "it was taken away" when there was
/// nothing to take. Cleared on the next reach action.
export type ReachChange =
  | { kind: "network"; changed: boolean }
  | { kind: "drive"; changed: boolean; host: string }
  | { kind: "sever"; cut: number; stillEscaping: number };
export const reachChanged = writable<ReachChange | null>(null);

/// Why a reach change did not happen, and to which bottle.
///
/// Its own store rather than `winActionFailed`, which says a CONFIG change did not
/// stick. This is not that: the reach a Windows app has is the sovereign question
/// the panel exists to answer, and "the folder is still granted" is what somebody
/// who pressed Take away needs told, not "your setting did not save".
export const reachFailed = writable<{ name: string; action: "network" | "drive" | "sever" } | null>(
  null,
);

/// Cut a bottle off the network. Live: `revoke_bottle_network`.
///
/// One direction only - there is no ask that gives it back, by design - so the
/// caller confirms first. The local row follows the daemon's answer rather than
/// being set optimistically: an app the panel shows as cut off while it still
/// reaches the network is the one mistake this section must not make.
export async function revokeNetwork(id: string): Promise<void> {
  reachFailed.set(null);
  reachChanged.set(null);
  try {
    const changed = await invoke<boolean>("revoke_bottle_network", { id });
    if (changed) {
      winApps.update((s) => ({
        ...s,
        bottles: s.bottles.map((b) =>
          b.id === id ? { ...b, access: { ...b.access, network: false } } : b,
        ),
      }));
    }
    reachChanged.set({ kind: "network", changed });
  } catch {
    if (!tauriAvailable) {
      winApps.update((s) => ({
        ...s,
        bottles: s.bottles.map((b) =>
          b.id === id ? { ...b, access: { ...b.access, network: false } } : b,
        ),
      }));
      reachChanged.set({ kind: "network", changed: true });
      return;
    }
    reachFailed.set({ name: nameOf(id), action: "network" });
  }
}

/// Take one granted folder away from a bottle. Live: `revoke_bottle_drive`.
///
/// Keyed by the HOST path, never the drive letter: letters are handed out by
/// sorting the grants, so they shift the moment one goes and revoking `D:` twice
/// would take two different folders away.
export async function revokeDrive(id: string, host: string): Promise<void> {
  reachFailed.set(null);
  reachChanged.set(null);
  try {
    const changed = await invoke<boolean>("revoke_bottle_drive", { id, host });
    if (changed) {
      dropDrive(id, host);
      // AND THEN ASK AGAIN, because removing a grant SHIFTS the letters of the
      // ones that remain - the daemon assigns them by sorting the grants
      // (`revoke_grant` in bottled). Dropping the row locally is right about
      // which folder went and wrong about what the survivors are called, and a
      // panel showing `E:` for a drive the app now sees as `D:` is the kind of
      // claim this page exists not to make.
      await load();
    }
    reachChanged.set({ kind: "drive", changed, host });
  } catch {
    if (!tauriAvailable) {
      dropDrive(id, host);
      reachChanged.set({ kind: "drive", changed: true, host });
      return;
    }
    reachFailed.set({ name: nameOf(id), action: "drive" });
  }
}

function dropDrive(id: string, host: string): void {
  winApps.update((s) => ({
    ...s,
    bottles: s.bottles.map((b) =>
      b.id === id ? { ...b, drives: b.drives.filter((d) => d.path !== host) } : b,
    ),
  }));
}

/// Cut the links that lead out of a bottle's prefix. Live: `sever_bottle`.
///
/// The remedy for the health warning, and the reason `stillEscaping` comes back
/// rather than being assumed zero: Wine writes those links on every boot, so a
/// pass that could not finish must not read as one that did.
export async function severBottle(id: string): Promise<void> {
  reachFailed.set(null);
  reachChanged.set(null);
  try {
    const r = await invoke<{ cut: number; stillEscaping: number }>("sever_bottle", { id });
    reachChanged.set({ kind: "sever", cut: r.cut, stillEscaping: r.stillEscaping });
  } catch {
    if (!tauriAvailable) {
      reachChanged.set({ kind: "sever", cut: 3, stillEscaping: 0 });
      return;
    }
    reachFailed.set({ name: nameOf(id), action: "sever" });
  }
}

/// How much disk one bottle holds, in bytes. Live: `bottle_disk_usage`.
///
/// `null` means nothing measured it - a bottle made and never booted, or a read
/// that failed - and the caller renders that as an absent line, never as zero.
/// Its own ask rather than a field on the listing, because measuring walks a
/// whole Wine prefix and the list shows every bottle at once.
export async function bottleDisk(id: string): Promise<number | null> {
  try {
    return await invoke<number | null>("bottle_disk_usage", { id });
  } catch {
    if (!tauriAvailable) return 1_200_000_000;
    return null;
  }
}

/// Read what this machine can actually run Windows programs with.
///
/// Replaces the opening list of runtimes that said "installed" about a disk nobody
/// had read. An empty list after this ran means there is none, which the panel says
/// rather than leaving the section looking merely unfilled.
export async function loadRuntimes(): Promise<void> {
  try {
    const wine = await invoke<string | null>("windows_runtimes");
    defaults.update((d) => ({
      ...d,
      runtimes: wine ? [{ name: wine }] : [],
    }));
    runtimesKnown.set(true);
  } catch {
    // Not measured is not the same as none, so the panel is told nothing rather
    // than told there is nothing.
    runtimesKnown.set(!tauriAvailable);
  }
}

/// Whether the runtime list above was actually read.
export const runtimesKnown = writable(false);

/// One program an installer left inside a bottle.
export interface BottleProgram {
  path: string;
  name: string;
}

/// What a bottle holds, and whether that is all of it.
export interface BottleProgramList {
  programs: BottleProgram[];
  /// True when the daemon cut the list to fit its wire frame. A panel that showed
  /// these as the whole set would be stating something it was told is incomplete.
  truncated: boolean;
}

/// What an installer left in a bottle, for the person to pick the app from.
///
/// A list rather than a guess: an installer writes the app, usually an
/// uninstaller, sometimes a crash reporter, and a bottle that launches the wrong
/// one is worse than a bottle that asked.
export async function bottlePrograms(id: string): Promise<BottleProgramList> {
  try {
    return await invoke<BottleProgramList>("bottle_programs", { id });
  } catch {
    // Under vite there is no daemon to walk a prefix, and a list nobody can see
    // is a row nobody can design. Live, an unreachable daemon answers nothing,
    // which the panel renders as "no program found yet" - the honest reading,
    // since it did not find one.
    if (!tauriAvailable) {
      return {
        programs: [
          { path: "/pfx/drive_c/Program Files/Ledger/ledger.exe", name: "ledger.exe" },
          { path: "/pfx/drive_c/Program Files/Ledger/report-tool.exe", name: "report-tool.exe" },
          { path: "/pfx/drive_c/Program Files/Ledger/unins000.exe", name: "unins000.exe" },
        ],
        truncated: true,
      };
    }
    return { programs: [], truncated: false };
  }
}

/// Why a program was not recorded for a bottle, as the daemon's token; null
/// when the last pick took. The one refusal specific to picking is
/// not-in-this-bottle; everything else says the pick did not reach the daemon.
export const programFailed = writable<string | null>(null);

/// The message key for a refused program pick.
export function programFailureKey(reason: string): string {
  return reason === "not-in-this-bottle" ? "s.wa.programNotInBottle" : "s.wa.programFailed";
}

/// Record which program the bottle starts, then reload so the page stops asking.
export async function setBottleProgram(id: string, program: string): Promise<void> {
  programFailed.set(null);
  try {
    await invoke("set_bottle_program", { id, program });
    await load();
  } catch (e) {
    if (!tauriAvailable) {
      winApps.update((s) => ({
        ...s,
        bottles: s.bottles.map((b) => (b.id === id ? { ...b, hasProgram: true } : b)),
      }));
    } else {
      programFailed.set(String(e));
      return;
    }
  }
  installStarted.update((s) => (s === id ? null : s));
}

/// The name the page shows for a bottle: the app's, or the bottle's id, which
/// is the installer's name and so is the honest fallback.
function nameOf(id: string): string {
  return get(winApps).bottles.find((b) => b.id === id)?.appName ?? id;
}
