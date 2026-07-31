/// Pending updates (update-flow-plan.md U-5): an update is a consent event, not
/// a chore. Quiet by default so the loud case - a capability widening - is
/// believed. The reads and actuators are coder seams (`store_outdated` over the
/// U-1 lock record + the cached catalog, `store_update`,
/// `store_update_all_routine`, `store_skip_update`); under vite a fixture stands
/// in and the actions apply locally so the whole flow drives.
import { derived, get, writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import type { Source } from "./catalog";

/// How an update changes the app's declared capabilities.
export type Delta = "none" | "narrowed" | "widened";

/// One outdated app.
export interface PendingUpdate {
  id: string;
  name: string;
  /// CSS background for the icon tile (live: the AppStream icon).
  icon: string;
  from: string;
  to: string;
  source: Source;
  delta: Delta;
  /// The plain-language capability delta ("Now also wants: ..."); empty when
  /// nothing changed.
  deltaLines: string[];
  /// Upstream release notes (AppStream release description), or null when the
  /// developer shipped none - the surface says so, never invents.
  notes: string | null;
}

const g = (a: string, b: string) => `linear-gradient(135deg, ${a}, ${b})`;

const FIXTURE: PendingUpdate[] = [
  {
    id: "org.example.stream",
    name: "Wavecast",
    icon: g("#5f3a1e", "#a06a3b"),
    from: "2.3.1",
    to: "2.4.0",
    source: "flathub",
    delta: "widened",
    deltaLines: ["Now also wants: Reads your files"],
    notes: "Adds playlist import from local files.",
  },
  {
    id: "org.arlen.notes",
    name: "Quiet Notes",
    icon: g("#1e3a5f", "#3b82a0"),
    from: "1.4.2",
    to: "1.5.0",
    source: "forage",
    delta: "none",
    deltaLines: [],
    notes: "Faster search across large folders and a fix for duplicated headings on paste.",
  },
  {
    id: "org.example.reader",
    name: "Leselampe",
    icon: g("#1e4a5f", "#3b7ea0"),
    from: "0.9.3",
    to: "0.9.4",
    source: "flathub",
    delta: "none",
    deltaLines: [],
    notes: null,
  },
  {
    id: "org.example.timer",
    name: "Sandglass",
    icon: g("#5f1e2e", "#a03b52"),
    from: "3.0.0",
    to: "3.1.0",
    source: "forage",
    delta: "narrowed",
    deltaLines: ["Asks for less than before: no longer shows notifications"],
    notes: "The chime now plays through the session sound theme.",
  },
];

/// The pending updates, widened first (the surface groups them anyway).
export const pendingUpdates = writable<PendingUpdate[]>([]);
/// True while the list is the FIXTURE.
export const updatesMocked = writable(false);
/// The quiet rail count.
export const updateCount = derived(pendingUpdates, ($u) => $u.length);

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

/// Update everything routine (safe precisely because widened ones are excluded).
export async function applyAllRoutine(): Promise<void> {
  const routine = get(pendingUpdates).filter((p) => p.delta !== "widened");
  pendingUpdates.update((u) => u.filter((p) => p.delta === "widened"));
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
