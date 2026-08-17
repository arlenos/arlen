/// The cross-toolkit status + control list for the Toolkits theme page. One Arlen
/// theme drives every toolkit; this states how far it reaches per toolkit
/// (the honest fidelity ceiling), whether it is on, and a per-toolkit override.
/// A flat list, never an N x M matrix; ragged coverage is stated per row.
///
/// Mock-vs-live: the coverage tiers + notes are real; the per-toolkit on/off, the
/// override map, and the prerequisite detection need coder backend. Fixture until.

import { writable } from "svelte/store";

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
  /// The honest prerequisite/status, or null.
  prereq: string | null;
  /// The native surface (always on; no off toggle, no override).
  native?: boolean;
}

/// The toolkits and their honest fidelity ceiling (theming-system-plan.md §1).
export const TOOLKITS: Toolkit[] = [
  { id: "arlen", name: "Arlen apps + compositor", coverage: "full", noteKey: "s.toolkit.note.arlen", prereq: null, native: true },
  { id: "gtk3", name: "GTK3", coverage: "full", noteKey: "s.toolkit.note.gtk3", prereq: "adw-gtk3 detected" },
  { id: "gtk4", name: "GTK4 / libadwaita", coverage: "colours", noteKey: "s.toolkit.note.gtk4", prereq: null },
  { id: "qt", name: "Qt5 / Qt6", coverage: "colours", noteKey: "s.toolkit.note.qt", prereq: "Set Qt apps to use qt6ct" },
  { id: "terminal", name: "Terminal", coverage: "full", noteKey: "s.toolkit.note.terminal", prereq: "Include the colour file in your terminal config" },
  { id: "wine", name: "Wine", coverage: "best-effort", noteKey: "s.toolkit.note.wine", prereq: "Experimental" },
];

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
