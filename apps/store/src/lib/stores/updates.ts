/// Pending updates (update-flow-plan.md U-5): an update is a consent event, not
/// a chore. Quiet by default so the loud case - a capability widening - is
/// believed. The wire type mirrors store-backend's `PendingUpdate` exactly
/// (`store_outdated` / `store_skip_update` return it verbatim, unflattened);
/// everything the page renders on top (label, icon, delta grouping, prose
/// lines) is derived HERE in a view model, so there is one shape on the wire
/// and one place that interprets it. Under vite a fixture in the wire shape
/// stands in and the actions apply locally so the whole flow drives.
import { derived, get, writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { apps } from "./catalog";

/// The backend's source layer, verbatim (six variants, PascalCase serde).
export type SourceLayer = "Personal" | "Community" | "Official" | "Flatpak" | "Apt" | "Native";

/// One outdated app, exactly as `store_outdated` serialises it. The tri-state
/// of `new_capabilities` is the point: `null` means the widening is UNKNOWN
/// (no declaration to compare), which must never render as reassurance; `[]`
/// means the update asks nothing new; entries are capability identifiers.
export interface PendingUpdate {
  id: string;
  layer: SourceLayer;
  installed_version: string;
  available_version: string;
  new_capabilities: string[] | null;
}

/// How an update changes what the app may ask for, derived from
/// `new_capabilities`. The backend deliberately reports additions only (a
/// dropped capability needs no warning), so there is no "narrowed".
export type Delta = "unknown" | "none" | "widened";

/// Classify one update for grouping: unknown and widened both need the user's
/// eyes; only a known-empty delta is routine.
export function deltaOf(u: PendingUpdate): Delta {
  if (u.new_capabilities === null) return "unknown";
  return u.new_capabilities.length > 0 ? "widened" : "none";
}

/// A capability identifier as a readable line, until the identifier -> copy
/// map exists (flagged as a seam): "graph.read_files" -> "graph read files".
export function capText(id: string): string {
  return id.replace(/[._-]/g, " ");
}

const FIXTURE: PendingUpdate[] = [
  {
    id: "org.example.stream",
    layer: "Flatpak",
    installed_version: "2.3.1",
    available_version: "2.4.0",
    new_capabilities: ["files.read_home"],
  },
  {
    id: "org.arlen.notes",
    layer: "Official",
    installed_version: "1.4.2",
    available_version: "1.5.0",
    new_capabilities: [],
  },
  {
    id: "org.example.reader",
    layer: "Flatpak",
    installed_version: "0.9.3",
    available_version: "0.9.4",
    new_capabilities: null,
  },
  {
    id: "org.example.timer",
    layer: "Apt",
    installed_version: "3.0.0",
    available_version: "3.1.0",
    new_capabilities: [],
  },
];

/// The pending updates in the wire shape; the page derives its view per row.
export const pendingUpdates = writable<PendingUpdate[]>([]);
/// True while the list is the FIXTURE.
export const updatesMocked = writable(false);
/// The quiet rail count.
export const updateCount = derived(pendingUpdates, ($u) => $u.length);

/// Display name + icon for an update row, resolved from the loaded catalog
/// when it knows the app; the id itself is the honest fallback.
export function updateApp(id: string): { name: string; icon: string | null } {
  const known = get(apps).find((a) => a.id === id);
  return known ? { name: known.name, icon: known.icon } : { name: id, icon: null };
}

/// Load the pending updates. Live: `store_outdated` (a local computation over
/// the lock record + the cached catalog, never a per-open network call).
export async function loadUpdates(): Promise<void> {
  try {
    pendingUpdates.set(await invoke<PendingUpdate[]>("store_outdated"));
    updatesMocked.set(false);
  } catch {
    pendingUpdates.set(structuredClone(FIXTURE));
    updatesMocked.set(true);
  }
}

function drop(id: string): void {
  pendingUpdates.update((u) => u.filter((p) => p.id !== id));
}

/// Apply one update. A widened one rides the consent friction-ladder at the
/// gate (U-4, the actuator's job); the row leaves the list either way.
export async function applyUpdate(id: string): Promise<void> {
  drop(id);
  try {
    await invoke("store_update", { id });
  } catch {
    // Seam unwired: the local removal stands.
  }
}

/// Update everything routine (safe precisely because widened AND unknown ones
/// are excluded - unknown is not routine).
export async function applyAllRoutine(): Promise<void> {
  const routine = get(pendingUpdates).filter((p) => deltaOf(p) === "none");
  pendingUpdates.update((u) => u.filter((p) => deltaOf(p) !== "none"));
  try {
    await invoke("store_update_all_routine", { ids: routine.map((p) => p.id) });
  } catch {
    // Seam unwired.
  }
}

/// Skip this update (it stops asking until the next version).
export async function skipUpdate(id: string): Promise<void> {
  drop(id);
  try {
    await invoke("store_skip_update", { id });
  } catch {
    // Seam unwired.
  }
}

/// Uninstall instead of accepting a widening - the third honest answer to
/// "now also wants". The row leaves the list; the removal itself is the
/// actuator's job.
export async function uninstallApp(id: string): Promise<void> {
  drop(id);
  try {
    await invoke("store_uninstall", { id });
  } catch {
    // Seam unwired.
  }
}
