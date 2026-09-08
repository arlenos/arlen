/// The cross-toolkit status + control list for the Toolkits theme page. One Arlen
/// theme drives every toolkit; this states how far it reaches per toolkit
/// (the honest fidelity ceiling), whether it is on, and a per-toolkit override.
/// A flat list, never an N x M matrix; ragged coverage is stated per row.
///
/// Mock-vs-live: the coverage tiers, the notes, the prerequisite detection and
/// the reach (whether the theme is actually in place, or whose file is in the
/// way) are all real. The per-toolkit on/off and the override map still need
/// coder backend and are fixture until they have one.

import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { tauriAvailable } from "$lib/tauri";

/// The coverage a theme achieves on a toolkit.
export type Coverage = "full" | "colours" | "best-effort";

/// One toolkit target.
export interface Toolkit {
  id: string;
  /// The row's name, as a catalogue key rather than a literal.
  ///
  /// Five of the six are product names that read the same in every language, so
  /// this looks like ceremony until you read the sixth: "Arlen apps +
  /// compositor" is a PHRASE, and it sat in a German page in English, inside
  /// German sentences ("Das Thema auf Arlen apps + compositor anwenden"). A key
  /// for every row means the next one that is a phrase cannot arrive as a
  /// literal by being typed next to a product name.
  nameKey: string;
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
  { id: "arlen", nameKey: "s.toolkit.name.arlen", coverage: "full", noteKey: "s.toolkit.note.arlen", prereqKey: null, native: true },
  { id: "gtk3", nameKey: "s.toolkit.name.gtk3", coverage: "full", noteKey: "s.toolkit.note.gtk3", prereqKey: "s.toolkit.prereq.gtk3" },
  { id: "gtk4", nameKey: "s.toolkit.name.gtk4", coverage: "colours", noteKey: "s.toolkit.note.gtk4", prereqKey: null },
  { id: "qt", nameKey: "s.toolkit.name.qt", coverage: "colours", noteKey: "s.toolkit.note.qt", prereqKey: "s.toolkit.prereq.qt" },
  { id: "terminal", nameKey: "s.toolkit.name.terminal", coverage: "full", noteKey: "s.toolkit.note.terminal", prereqKey: null },
  { id: "wine", nameKey: "s.toolkit.name.wine", coverage: "best-effort", noteKey: "s.toolkit.note.wine", prereqKey: "s.toolkit.prereq.wine" },
];

/// Whether the theme is actually in place for a toolkit, as opposed to what the
/// toolkit could take. The badge says the ceiling; this says the floor.
export interface ToolkitReach {
  state: "ours" | "blocked" | "unselected" | "absent";
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
        // Two different unhappy states, because each renders a different
        // sentence and the fixture is the only place either is looked at
        // without a machine in that condition.
        gtk3: { state: "unselected", blockedBy: "Adwaita" },
        gtk4: { state: "ours", blockedBy: null },
        qt: { state: "blocked", blockedBy: "qt6ct.conf" },
        terminal: { state: "ours", blockedBy: null },
      });
    }
  }
}

/// Whether each detectable prerequisite is met, per toolkit id. Empty until
/// read. A toolkit with no entry has nothing to detect, and its line is a status
/// rather than a condition - Wine's "Experimental" is true whatever is
/// installed.
export const prereqs = writable<Record<string, boolean>>({});

/// Ask the backend which prerequisites this machine meets. Read-only.
export async function loadPrereqs(): Promise<void> {
  try {
    prereqs.set(await invoke<Record<string, boolean>>("theme_toolkit_prereqs"));
  } catch {
    // The fixture says MET for both, so the vite render shows the page a person
    // with a working desktop sees. The unmet case is one line and is designed
    // by reading the row with the condition shown.
    if (!tauriAvailable) prereqs.set({ gtk3: true, qt: true });
  }
}

/// Whether a toolkit's prerequisite line should be shown at all.
///
/// A line that says what to install is worth reading when the thing is missing
/// and is noise every other time. So: a toolkit the backend can detect shows it
/// only when the answer is NO, and one it cannot detect shows it always, because
/// there the line is a standing status rather than a thing to go and do.
export function showPrereq(p: Record<string, boolean>, id: string): boolean {
  return !(id in p) || p[id] === false;
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
