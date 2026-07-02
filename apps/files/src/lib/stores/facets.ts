/// The faceted KG filter: the four facet groups the filter bar narrows by
/// (project, type, time, touched), serialized into a `facet:` virtual location
/// the adapter lists. Selecting a value navigates the active controller to that
/// location, so the normal listing renders the faceted result the same way it
/// renders `project:` or `recent:`; clearing returns to the folder the filter
/// opened over. A named combo saves as a Smart Folder in the sidebar.
///
/// Type and time are intrinsic file attributes, so their value sets are static
/// and client-side. Project options reuse `files_projects`; touched options and
/// per-value counts need graph reads that the contract does not have yet
/// (flagged) — until they land the bar still drives navigation and the counts
/// simply stay absent.

import { get, writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// The facet groups, in the deterministic order they serialize and render.
export type FacetGroup = "project" | "type" | "time" | "touched";

/// One selectable value within a facet group.
export interface FacetValue {
  /// The stable key encoded into the `facet:` location.
  value: string;
  /// The label shown in the dropdown and the active chip.
  label: string;
  /// The graph count for this value, when the backend supplies it.
  count?: number;
}

/// A saved facet combo, surfaced as a Smart Folder place in the sidebar.
export interface SmartFolder {
  id: string;
  name: string;
  /// The `facet:` location the folder re-applies on click.
  location: string;
}

/// The groups in render/serialize order. Time is single-select (a cutoff, not a
/// union); the rest union their values (OR within a group, AND across groups).
export const FACET_GROUPS: FacetGroup[] = ["project", "type", "time", "touched"];

/// The human label for each group, shown on the dropdown trigger and the chip.
export const GROUP_LABEL: Record<FacetGroup, string> = {
  project: "Project",
  type: "Type",
  time: "Time",
  touched: "Touched",
};

/// Time is one cutoff at a time, so its dropdown replaces rather than unions.
export const SINGLE_SELECT: Record<FacetGroup, boolean> = {
  project: false,
  type: false,
  time: true,
  touched: false,
};

/// The intrinsic type facet: file kinds derived from the extension, no graph.
export const TYPE_VALUES: FacetValue[] = [
  { value: "document", label: "Documents" },
  { value: "image", label: "Images" },
  { value: "audio", label: "Audio" },
  { value: "video", label: "Video" },
  { value: "archive", label: "Archives" },
  { value: "code", label: "Code" },
];

/// The intrinsic time facet: a single recency cutoff.
export const TIME_VALUES: FacetValue[] = [
  { value: "day", label: "Today" },
  { value: "week", label: "Last 7 days" },
  { value: "month", label: "Last 30 days" },
  { value: "older", label: "Older than 30 days" },
];

/// Graph-loaded option sets (empty until `loadFacetOptions` runs, or when the
/// graph has none — the dropdown then reads as having nothing to offer).
export const projectValues = writable<FacetValue[]>([]);
export const touchedValues = writable<FacetValue[]>([]);

/// Whether the filter bar is revealed.
export const facetOpen = writable(false);

/// The real folder the filter opened over, returned to when every facet clears.
export const facetBase = writable("/");

/// The saved Smart Folders. Loaded from disk on start and persisted on every
/// change so they survive a restart.
export const savedFolders = writable<SmartFolder[]>([]);

// Persist on change, but only after the initial load has run, so restoring the
// saved set on start doesn't immediately echo straight back to disk.
let persistReady = false;
savedFolders.subscribe((list) => {
  if (!persistReady) return;
  void invoke("files_smart_folders_save", { folders: list }).catch(() => {});
});

function emptySelection(): Record<FacetGroup, Set<string>> {
  return { project: new Set(), type: new Set(), time: new Set(), touched: new Set() };
}

/// The selected value keys per group.
export const selectedFacets = writable<Record<FacetGroup, Set<string>>>(emptySelection());

/// Build the `facet:` location key for a selection, or "" when nothing is set.
export function serializeFacets(sel: Record<FacetGroup, Set<string>>): string {
  const parts: string[] = [];
  for (const g of FACET_GROUPS) {
    const vals = [...sel[g]];
    if (vals.length) parts.push(`${g}=${vals.join(",")}`);
  }
  return parts.length ? `facet:${parts.join(";")}` : "";
}

/// Parse a `facet:` location back into a selection (for a saved Smart Folder).
export function parseFacets(location: string): Record<FacetGroup, Set<string>> {
  const sel = emptySelection();
  if (!location.startsWith("facet:")) return sel;
  for (const part of location.slice("facet:".length).split(";")) {
    const eq = part.indexOf("=");
    if (eq < 0) continue;
    const group = part.slice(0, eq) as FacetGroup;
    const csv = part.slice(eq + 1);
    if (!FACET_GROUPS.includes(group) || !csv) continue;
    for (const v of csv.split(",")) if (v) sel[group].add(v);
  }
  return sel;
}

/// Total number of selected values across all groups.
export function countFacets(sel: Record<FacetGroup, Set<string>>): number {
  return FACET_GROUPS.reduce((n, g) => n + sel[g].size, 0);
}

/// Toggle a value in a group; a single-select group replaces instead of unions.
export function toggleValue(group: FacetGroup, value: string): void {
  selectedFacets.update((s) => {
    const next = { ...s, [group]: new Set(s[group]) };
    const set = next[group];
    if (SINGLE_SELECT[group]) {
      const had = set.has(value);
      set.clear();
      if (!had) set.add(value);
    } else if (set.has(value)) {
      set.delete(value);
    } else {
      set.add(value);
    }
    return next;
  });
}

/// Drop one group's whole selection (dismissing its active chip).
export function clearGroup(group: FacetGroup): void {
  selectedFacets.update((s) => ({ ...s, [group]: new Set() }));
}

/// Drop every selection.
export function clearFacets(): void {
  selectedFacets.set(emptySelection());
}

/// Adopt a saved Smart Folder's facets into the live selection.
export function applySmartFolder(folder: SmartFolder): void {
  selectedFacets.set(parseFacets(folder.location));
}

/// Save the current selection as a named Smart Folder. Returns null when nothing
/// is selected. The new folder persists through the `savedFolders` subscription.
export function saveSmartFolder(name: string): SmartFolder | null {
  const location = serializeFacets(get(selectedFacets));
  if (!location) return null;
  const folder: SmartFolder = {
    id: `smart-${get(savedFolders).length + 1}-${location}`,
    name: name.trim() || "Filtered",
    location,
  };
  savedFolders.update((list) => [...list, folder]);
  return folder;
}

/// Load the persisted Smart Folders on start, then enable the persist
/// subscription so later changes are written back.
export async function loadSmartFolders(): Promise<void> {
  try {
    savedFolders.set(await invoke<SmartFolder[]>("files_smart_folders"));
  } catch {
    // No saved folders yet, or the read failed; leave the set empty.
  }
  persistReady = true;
}

/// Load the graph-backed facet options. Projects reuse `files_projects`; the
/// touched-app list needs a dedicated read (flagged). Failures leave the set
/// empty rather than fake.
export async function loadFacetOptions(): Promise<void> {
  try {
    const projects = await invoke<{ id: string; name: string }[]>("files_projects");
    projectValues.set(projects.map((p) => ({ value: p.id, label: p.name })));
  } catch {
    projectValues.set([]);
  }
  try {
    const apps = await invoke<{ id: string; label: string; count?: number }[]>(
      "files_touched_apps",
    );
    touchedValues.set(apps.map((a) => ({ value: a.id, label: a.label, count: a.count })));
  } catch {
    touchedValues.set([]);
  }
}
