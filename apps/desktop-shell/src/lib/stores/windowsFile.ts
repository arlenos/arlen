/// The open-a-Windows-file dialog (windows-apps-plan.md §41-60): opening a
/// .exe/.msi is a sovereign TRUST moment, not a setup wall. Arlen pre-installs the
/// bottle daemon, so the dialog says "you're opening a foreign Windows app, here's
/// what happens": it identifies the app, states the compat tier honestly, makes the
/// sandbox + the minted permission profile legible, and offers Run vs Install. The
/// sibling of the unified consent dialog; it reuses that chrome.
///
/// Mock-vs-live: fixture-backed. The trigger (the FM/portal opening a Windows file
/// -> `windows_file_request`), the compat lookup, `.exe` icon extraction, and the
/// run/install commands are all coder seams on the deferred bottle daemon; under
/// vite the store serves a fixture so the surface renders.

import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// How well-supported the app is, stated honestly (never "just works").
export type WinCompatTier = "verified" | "should-work" | "untested";

/// A double-click installer versus a portable executable.
export type WinFileKind = "installer" | "portable";

/// The pending Windows-file open the dialog renders.
export interface PendingWindowsFile {
  id: number;
  /// Best-effort app name (from the .exe metadata, else the file name).
  appName: string;
  fileName: string;
  fileKind: WinFileKind;
  tier: WinCompatTier;
  /// The curated compat-recipe, when one applies.
  recipe?: string;
  /// The scopes the freshly minted permission profile grants (the sovereign preview).
  access: string[];
  /// First-run runtime fetch in progress ("Getting Wine 9.0 for this app") - a
  /// progress step inside the same dialog, never a setup wall. Live: the bottle
  /// daemon's fetch-progress event (seam).
  fetch?: { runtime: string; progress: number } | null;
}

// One case per state the dialog must carry: the verified installer, the
// untested portable, and a first-run fetch in flight.
const MOCK: PendingWindowsFile[] = [
  {
    id: 1,
    appName: "Paint.NET Setup",
    fileName: "paint.net.5.1.install.msi",
    fileKind: "installer",
    tier: "verified",
    recipe: "Paint.NET recipe",
    access: ["Its own files"],
  },
  {
    id: 2,
    appName: "LegacyTool",
    fileName: "LegacyTool.exe",
    fileKind: "portable",
    tier: "untested",
    access: ["Its own files", "Network"],
  },
  {
    id: 3,
    appName: "Affinity Photo Setup",
    fileName: "affinity-photo-2.6.exe",
    fileKind: "installer",
    tier: "should-work",
    access: ["Its own files", "Your Pictures folder"],
    fetch: { runtime: "Wine 9.0", progress: 0.42 },
  },
];


/// The Windows file waiting on a decision now, or null.
export const current = writable<PendingWindowsFile | null>(null);

/// True when the last Run or Install did not reach the bottle daemon. The dialog
/// stays open: closing it is how it says "started", and nothing did.
export const launchFailed = writable(false);

// `?wfmock=<n>` (DEV only) pins which fixture renders, so the screenshot loop
// can address every state by URL - the `?menumock` pattern.
//
// ASKED FOR, not merely allowed. The fixture used to show on any failed request
// under vite, and since vite never has a bottle daemon that meant every dev route
// in the shell wore a modal "Open Paint.NET Setup?" over it, with a full-screen
// overlay at z-490 underneath. The sound panel's own refusal strip was behind it
// in a photograph I took of the sound panel. A fixture that appears where nobody
// invited it is indistinguishable from the app doing something wrong, and it
// hides whatever was actually being looked at.
const wanted = (() => {
  if (!import.meta.env.DEV || typeof location === "undefined") return null;
  const raw = new URLSearchParams(location.search).get("wfmock");
  if (raw === null) return null;
  const pinned = Number(raw);
  return Number.isInteger(pinned) && pinned >= 0 ? pinned : 0;
})();
let mockIndex = wanted ?? 0;

/// Fetch the pending open request. Live: `windows_file_request`. The fixture is
/// served ONLY under vite (dev) and only when the URL asks for it; on a real boot
/// a failed request shows nothing rather than covering the desktop with a demo
/// modal every session.
export async function openWindowsFile(): Promise<void> {
  try {
    current.set(await invoke<PendingWindowsFile | null>("windows_file_request"));
  } catch {
    // `import.meta.env.DEV` is spelled out here rather than left inside
    // `wanted` so `check-fixture-on-failure` can see the guard it is looking
    // for; both halves are real, and the second one is what keeps the fixture
    // off every dev route that did not ask for it.
    current.set(
      import.meta.env.DEV && wanted !== null ? MOCK[mockIndex % MOCK.length] : null,
    );
  }
}

/// Run the app as a one-off in an auto-bottle, then clear. Live: `windows_file_run`.
export async function run(id: number): Promise<void> {
  launchFailed.set(false);
  try {
    await invoke("windows_file_run", { id });
  } catch {
    if (import.meta.env.DEV) {
      current.set(null); // no bottle daemon under vite
      return;
    }
    launchFailed.set(true);
    return;
  }
  current.set(null);
}

/// Install the app as a first-class app, then clear. Live: `windows_file_install`.
export async function install(id: number): Promise<void> {
  launchFailed.set(false);
  try {
    await invoke("windows_file_install", { id });
  } catch {
    if (import.meta.env.DEV) {
      current.set(null);
      return;
    }
    // An app the user believes is installed, and is not, is one they will look
    // for in the launcher.
    launchFailed.set(true);
    return;
  }
  current.set(null);
}

/// Decline the open and clear.
export function cancel(): void {
  // Declining needs no backend, so it always succeeds.
  current.set(null);
  launchFailed.set(false);
}

/// Dev-only: step to the next fixture (the screenshot loop).
export function cycleMock(): void {
  mockIndex = (mockIndex + 1) % MOCK.length;
  current.set(MOCK[mockIndex]);
}
