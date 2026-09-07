/// The cross-toolkit status + control list for the Toolkits theme page. One Arlen
/// theme drives every toolkit; this states how far it reaches per toolkit
/// (the honest fidelity ceiling), whether it is on, and a per-toolkit override.
/// A flat list, never an N x M matrix; ragged coverage is stated per row.
///
/// Mock-vs-live: the coverage tiers + notes are real; the per-toolkit on/off, the
/// override map, and the prerequisite detection need coder backend. Fixture until.

import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { tauriAvailable } from "$lib/tauri";

/// The coverage a theme achieves on a toolkit.
export type Coverage = "full" | "colours" | "best-effort";

/// One toolkit target.
export interface Toolkit {
  id: string;
  name: string;
  coverage: Coverage;
  /// Catalogue key for the one-line fidelity note. A key rather than the
  /// sentence, because this list is a module constant: prose written here is
  /// resolved once at import and would keep the startup language for the life
  /// of the process. `coverageBadge` below already does it this way.
  noteKey: string;
  /// Catalogue key for the honest prerequisite or status, or null. A key for
  /// the same reason `noteKey` is one: this list is a module constant, so a
  /// sentence written here resolves once at import and keeps the language the
  /// process started in. These four were prose until 7 September and the
  /// born-translatable lint never saw them, because it does not read a string
  /// assigned to a field in a constant - which is how the one thing on the page
  /// that tells a person what to DO stayed English in every locale.
  prereqKey: string | null;
  /// The native surface (always on; no off toggle, no override).
  native?: boolean;
}

/// The toolkits and their honest fidelity ceiling (theming-system-plan.md §1).
export const TOOLKITS: Toolkit[] = [
  { id: "arlen", name: "Arlen apps + compositor", coverage: "full", noteKey: "s.toolkit.note.arlen", prereqKey: null, native: true },
  { id: "gtk3", name: "GTK3", coverage: "full", noteKey: "s.toolkit.note.gtk3", prereqKey: "s.toolkit.prereq.gtk3" },
  { id: "gtk4", name: "GTK4 / libadwaita", coverage: "colours", noteKey: "s.toolkit.note.gtk4", prereqKey: null },
  { id: "qt", name: "Qt5 / Qt6", coverage: "colours", noteKey: "s.toolkit.note.qt", prereqKey: "s.toolkit.prereq.qt" },
  { id: "terminal", name: "Terminal", coverage: "full", noteKey: "s.toolkit.note.terminal", prereqKey: null },
  { id: "wine", name: "Wine", coverage: "best-effort", noteKey: "s.toolkit.note.wine", prereqKey: "s.toolkit.prereq.wine" },
];

/// Whether the theme is actually in place for a toolkit, as opposed to what the
/// toolkit could take. The badge says the ceiling; this says the floor.
export interface ToolkitReach {
  state: "ours" | "blocked" | "absent";
  /// The file in the way, when one is.
  blockedBy: string | null;
}

/// Per toolkit id. Empty until read, which reads as "not checked" rather than
/// "fine" - the row shows nothing until there is an answer.
export const reach = writable<Record<string, ToolkitReach>>({});

/// Ask the backend whether the theme is in place. Read-only, so it is safe on
/// mount and safe to repeat.
export async function loadReach(): Promise<void> {
  try {
    reach.set(await invoke<Record<string, ToolkitReach>>("theme_toolkit_reach"));
  } catch {
    // Under vite there is no backend. The fixture shows the state that is worth
    // designing for - one toolkit the theme cannot reach - rather than the happy
    // one, because the happy one renders as nothing at all.
    if (!tauriAvailable) {
      reach.set({
        gtk3: { state: "ours", blockedBy: null },
        gtk4: { state: "ours", blockedBy: null },
        qt: { state: "blocked", blockedBy: "qt6ct.conf" },
        terminal: { state: "ours", blockedBy: null },
      });
    }
  }
}

/// Coverage tier → the badge label + tone.
export function coverageBadge(c: Coverage): { labelKey: string; tone: "success" | "outline" | "warn" } {
  if (c === "full") return { labelKey: "s.toolkit.full", tone: "success" };
  if (c === "colours") return { labelKey: "s.toolkit.coloursOnly", tone: "outline" };
  return { labelKey: "s.toolkit.bestEffort", tone: "warn" };
}

/// Toolkits the user has switched off (theme not emitted there). Default: none.
export const disabled = writable<Record<string, boolean>>({});
/// Per-toolkit accent overrides (a toolkit uses a different accent than the hub).
export const accentOverrides = writable<Record<string, string>>({});

/// Whether the theme is applied to a toolkit.
export function isEnabled(d: Record<string, boolean>, id: string): boolean {
  return !d[id];
}

/// Switch a toolkit's theming on/off.
export function setEnabled(id: string, on: boolean): void {
  disabled.update((d) => {
    const next = { ...d };
    if (on) delete next[id];
    else next[id] = true;
    return next;
  });
}

/// Whether a toolkit has a per-toolkit accent override.
export function hasAccentOverride(a: Record<string, string>, id: string): boolean {
  return id in a;
}

/// Set a toolkit's accent override.
export function setAccentOverride(id: string, hex: string): void {
  accentOverrides.update((a) => ({ ...a, [id]: hex }));
}

/// Clear a toolkit's accent override (back to the hub accent).
export function resetAccentOverride(id: string): void {
  accentOverrides.update((a) => {
    const next = { ...a };
    delete next[id];
    return next;
  });
}
