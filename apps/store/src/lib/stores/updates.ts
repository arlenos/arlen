/// Pending updates (update-flow-plan.md U-5): an update is a consent event, not
/// a chore. Quiet by default so the loud case - a capability widening - is
/// believed. The wire type mirrors store-backend's `PendingUpdate` exactly
/// (`store_outdated` / `store_skip_update` return it verbatim, unflattened);
/// everything the page renders on top (label, icon, delta grouping, prose
/// lines, the state of each row) is derived HERE, so there is one shape on the
/// wire and one place that interprets it. Under vite a fixture in the wire
/// shape stands in and the actions apply locally so the whole flow drives.
///
/// State is PER ROW, never one global sentence: a refusal that cannot say which
/// app it was about is a refusal the reader has to guess at. And a row never
/// vanishes on an unanswered question - the gate's answer to `store_update`
/// arrives later, as installd's `ConsentRequired` signal, which nothing in this
/// window can hear yet (the job-outcome channel is a named seam). Until it
/// exists, a started update whose row is still listed after a reload is said
/// to be exactly that: started, outcome not known here.
import { derived, get, writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { apps, type SourceLayer } from "./catalog";

export type { SourceLayer } from "./catalog";

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

/// What one row is doing. `refused` carries the backend's own sentence (free
/// prose today - a tagged refusal vocabulary is a named seam); `unconfirmed`
/// is a started update whose row survived the reload; `notStarted` is a
/// routine-batch member the daemon never reached.
export type RowStatus =
  | { kind: "applying" }
  | { kind: "unconfirmed" }
  | { kind: "refused"; reason: string }
  | { kind: "notStarted" };

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

/// A refusal the fixture shows, so the state is designable without a daemon
/// that refuses on demand. The sentence is the shape installd really returns
/// for a Debian-layer app.
const FIXTURE_STATUS: Record<string, RowStatus> = {
  "org.example.timer": {
    kind: "refused",
    reason: "org.example.timer is recorded as installed from apt, which this build has no way to update",
  },
};

/// The pending updates in the wire shape; the page derives its view per row.
export const pendingUpdates = writable<PendingUpdate[]>([]);
/// True while the list is the FIXTURE.
export const updatesMocked = writable(false);
/// Each row's state, keyed by app id; absent means idle.
export const rowStatus = writable<Record<string, RowStatus>>({});
/// Updates skipped this session. A skipped widening leaves the decision group
/// because the user decided, and stays visible because a silently vanishing
/// capability widening is the one thing U-4 forbids. The backend filters
/// skipped versions out of `store_outdated`; reading them back is a seam.
export const skippedUpdates = writable<PendingUpdate[]>([]);
/// The rail count: only rows that need a decision (U-5 - no red dot for
/// routine updates).
export const updateCount = derived(pendingUpdates, ($u) => $u.filter((p) => deltaOf(p) !== "none").length);

/// Display name + icon for an update row, resolved from the loaded catalog
/// when it knows the app; the id itself is the honest fallback.
export function updateApp(id: string): { name: string; icon: string | null } {
  const known = get(apps).find((a) => a.id === id);
  return known ? { name: known.name, icon: known.icon } : { name: id, icon: null };
}

function setStatus(id: string, status: RowStatus | null): void {
  rowStatus.update((m) => {
    const next = { ...m };
    if (status) next[id] = status;
    else delete next[id];
    return next;
  });
}

/// Load the pending updates. Live: `store_outdated` (a local computation over
/// the lock record + the cached catalog, never a per-open network call).
export async function loadUpdates(): Promise<void> {
  try {
    pendingUpdates.set(await invoke<PendingUpdate[]>("store_outdated"));
    updatesMocked.set(false);
  } catch {
    pendingUpdates.set(structuredClone(FIXTURE));
    if (!get(updatesMocked)) rowStatus.set({ ...FIXTURE_STATUS });
    updatesMocked.set(true);
  }
}

function drop(id: string): void {
  pendingUpdates.update((u) => u.filter((p) => p.id !== id));
}

/// After a started update: reload, and any started row still listed is said
/// to be unconfirmed rather than done. Under vite the local apply IS the
/// behaviour, so the row simply leaves.
async function settle(started: string[]): Promise<void> {
  if (get(updatesMocked)) {
    for (const id of started) {
      drop(id);
      setStatus(id, null);
    }
    return;
  }
  await new Promise((r) => setTimeout(r, 1500));
  await loadUpdates();
  const still = new Set(get(pendingUpdates).map((p) => p.id));
  for (const id of started) setStatus(id, still.has(id) ? { kind: "unconfirmed" } : null);
}

/// Apply one update. The gate at installd refuses a version asking for more
/// than the installed one; the page never offers this button on a widened row
/// (it would be refused), only on routine and unknown ones.
export async function applyUpdate(id: string): Promise<void> {
  setStatus(id, { kind: "applying" });
  try {
    await invoke("store_update", { id });
  } catch (e) {
    if (get(updatesMocked)) {
      drop(id);
      setStatus(id, null);
      return;
    }
    setStatus(id, { kind: "refused", reason: String(e) });
    return;
  }
  await settle([id]);
}

/// Update everything routine (safe precisely because widened AND unknown ones
/// are excluded - unknown is not routine). The daemon enqueues in order and
/// stops at the first refusal, returning the job ids it DID enqueue - so the
/// first `jobs.length` ids started and the rest did not, and the rest are
/// told so rather than folded into a success.
export async function applyAllRoutine(): Promise<void> {
  const ids = get(pendingUpdates)
    .filter((p) => deltaOf(p) === "none")
    .map((p) => p.id);
  for (const id of ids) setStatus(id, { kind: "applying" });
  let jobs: string[];
  try {
    jobs = await invoke<string[]>("store_update_all_routine", { ids });
  } catch (e) {
    if (get(updatesMocked)) {
      await settle(ids);
      return;
    }
    // The first one refused: nothing started.
    for (const id of ids) setStatus(id, { kind: "refused", reason: String(e) });
    return;
  }
  const started = ids.slice(0, jobs.length);
  for (const id of ids.slice(jobs.length)) setStatus(id, { kind: "notStarted" });
  await settle(started);
}

/// Skip this version (the app keeps asking at the next one). The row moves to
/// the skipped group instead of vanishing.
export async function skipUpdate(id: string): Promise<void> {
  const row = get(pendingUpdates).find((p) => p.id === id);
  if (!row) return;
  drop(id);
  setStatus(id, null);
  skippedUpdates.update((s) => [...s.filter((p) => p.id !== id), row]);
  try {
    await invoke("store_skip_update", { id });
  } catch (e) {
    if (get(updatesMocked)) return;
    // The skip did not record: the row comes back, with the reason.
    skippedUpdates.update((s) => s.filter((p) => p.id !== id));
    pendingUpdates.update((u) => [...u, row]);
    setStatus(id, { kind: "refused", reason: String(e) });
  }
}

/// A row was uninstalled from this page; it leaves the list.
export function forgetUpdate(id: string): void {
  drop(id);
  setStatus(id, null);
}
