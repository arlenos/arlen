/// The on-demand duplicate finder: a BLAKE3 content scan of the current
/// location (never always-on) groups byte-identical files, and this holds the
/// review state. The destructive part is gated by a safety floor: at least one
/// copy per group is always kept (the guard refuses to mark the last one), the
/// action is trash (not delete), and a sensible default keeps the newest.
///
/// The scan command does not exist yet (coder seam, `files_find_duplicates`);
/// until it lands the surface drives the review against mocked groups.

import { get, writable, derived } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// One copy within a duplicate group.
export interface DupFile {
  path: string;
  name: string;
  size: number;
  modified_unix: number | null;
}

/// A set of byte-identical files (same BLAKE3 hash).
export interface DupGroup {
  hash: string;
  files: DupFile[];
}

/// Whether the duplicates view is shown in place of the listing.
export const duplicatesOpen = writable(false);
/// True while the scan runs (the surface shows the scanning state).
export const duplicatesScanning = writable(false);
/// null = no scan yet; [] = scanned and clean; else the groups.
export const duplicateGroups = writable<DupGroup[] | null>(null);
/// The location the scan ran under, for the header and the empty copy.
export const duplicatesScope = writable("");
/// The paths the user has marked to trash (everything else is kept).
export const trashMarks = writable<Set<string>>(new Set());

/// The newest file's path in a group (the default keeper).
function newestPath(g: DupGroup): string {
  return [...g.files].sort(
    (a, b) => (b.modified_unix ?? 0) - (a.modified_unix ?? 0),
  )[0].path;
}

/// How many copies in a group are NOT marked for trash (i.e. kept).
export function keptCount(g: DupGroup, marks: Set<string>): number {
  return g.files.filter((f) => !marks.has(f.path)).length;
}

/// Mark every copy except the newest in each group (the keep-newest preset).
export function keepNewest(): void {
  const groups = get(duplicateGroups) ?? [];
  const next = new Set<string>();
  for (const g of groups) {
    const keep = newestPath(g);
    for (const f of g.files) if (f.path !== keep) next.add(f.path);
  }
  trashMarks.set(next);
}

/// Toggle a copy's trash mark, refusing to mark the last kept copy in its group
/// (the keep-one safety guard). The keeper can always be un-marked.
export function toggleTrash(g: DupGroup, path: string): void {
  trashMarks.update((set) => {
    const next = new Set(set);
    if (next.has(path)) {
      next.delete(path);
    } else if (keptCount(g, next) > 1) {
      next.add(path);
    }
    return next;
  });
}

/// The full paths currently marked for trash.
export const markedPaths = derived(trashMarks, ($marks) => [...$marks]);

/// The bytes reclaimed by trashing the marked copies.
export const reclaimable = derived(
  [duplicateGroups, trashMarks],
  ([$groups, $marks]) => {
    let bytes = 0;
    for (const g of $groups ?? [])
      for (const f of g.files) if ($marks.has(f.path)) bytes += f.size;
    return bytes;
  },
);

/// Reclaimable bytes within one group (its marked copies), for the group header.
export function groupReclaimable(g: DupGroup, marks: Set<string>): number {
  return g.files.reduce((n, f) => (marks.has(f.path) ? n + f.size : n), 0);
}

/// Run the on-demand scan over `path` (recursively). Failures clear to a clean
/// result rather than a stuck spinner.
export async function scanDuplicates(path: string): Promise<void> {
  duplicatesScope.set(path);
  duplicatesScanning.set(true);
  duplicateGroups.set(null);
  try {
    const groups = await invoke<DupGroup[]>("files_find_duplicates", { path });
    duplicateGroups.set(groups);
    keepNewest();
  } catch {
    duplicateGroups.set([]);
    trashMarks.set(new Set());
  } finally {
    duplicatesScanning.set(false);
  }
}

/// Close the duplicates view and drop its state.
export function closeDuplicates(): void {
  duplicatesOpen.set(false);
  duplicatesScanning.set(false);
  duplicateGroups.set(null);
  trashMarks.set(new Set());
}
