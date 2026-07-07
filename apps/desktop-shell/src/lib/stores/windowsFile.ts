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
  /// If a Wine/Proton version must be fetched the first time, its name.
  needsRuntime?: string;
}

// One representative installer + one portable so both action layouts + tiers render.
const MOCK: PendingWindowsFile[] = [
  {
    id: 1,
    appName: "Paint.NET Setup",
    fileName: "paint.net.5.1.install.msi",
    fileKind: "installer",
    tier: "verified",
    recipe: "Paint.NET recipe",
    access: ["Its own files"],
    needsRuntime: "Proton 9.0",
  },
  {
    id: 2,
    appName: "LegacyTool",
    fileName: "LegacyTool.exe",
    fileKind: "portable",
    tier: "untested",
    access: ["Its own files", "Network"],
  },
];

/// The Windows file waiting on a decision now, or null.
export const current = writable<PendingWindowsFile | null>(null);

let mockIndex = 0;

/// Fetch the pending open request. Live: `windows_file_request`; fixture under vite.
export async function openWindowsFile(): Promise<void> {
  try {
    current.set(await invoke<PendingWindowsFile | null>("windows_file_request"));
  } catch {
    current.set(MOCK[mockIndex % MOCK.length]);
  }
}

/// Run the app as a one-off in an auto-bottle, then clear. Live: `windows_file_run`.
export async function run(id: number): Promise<void> {
  current.set(null);
  try {
    await invoke("windows_file_run", { id });
  } catch {
    // No bottle daemon under vite: the optimistic clear stands.
  }
}

/// Install the app as a first-class app, then clear. Live: `windows_file_install`.
export async function install(id: number): Promise<void> {
  current.set(null);
  try {
    await invoke("windows_file_install", { id });
  } catch {
    // seam
  }
}

/// Decline the open and clear.
export function cancel(): void {
  current.set(null);
}

/// Dev-only: step to the next fixture (the screenshot loop).
export function cycleMock(): void {
  mockIndex = (mockIndex + 1) % MOCK.length;
  current.set(MOCK[mockIndex]);
}
