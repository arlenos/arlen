/// Which full-view overlay (if any) replaces the file listing. Trash and Recent
/// are virtual views, not browsed folders, and are mutually exclusive - one
/// shared source keeps them so (opening one closes the other) without the
/// stores importing each other. `null` = the normal listing/search view.

import { writable } from "svelte/store";

export type OverlayKind = "trash" | "recent" | null;

export const overlay = writable<OverlayKind>(null);

/// Back to the listing.
export function closeOverlay(): void {
  overlay.set(null);
}
