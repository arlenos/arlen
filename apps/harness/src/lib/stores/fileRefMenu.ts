/// The single right-click menu shared by every file-reference pill in the
/// transcript. A pill's contextmenu opens it at the cursor with that pill's
/// target; one instance (mounted once near the transcript root) reads this,
/// rather than a menu per pill.

import { writable } from "svelte/store";

/// What the open file-ref menu is acting on, and where it sits.
export interface FileRefMenuState {
  open: boolean;
  x: number;
  y: number;
  path: string;
  name: string;
  /// A path the opener could not resolve offers only Copy path (no Open).
  resolvable: boolean;
}

const closed: FileRefMenuState = {
  open: false,
  x: 0,
  y: 0,
  path: "",
  name: "",
  resolvable: true,
};

/// The live menu state.
export const fileRefMenu = writable<FileRefMenuState>(closed);

/// Open the menu at the cursor for one pill's target.
export function openFileRefMenu(at: Omit<FileRefMenuState, "open">): void {
  fileRefMenu.set({ ...at, open: true });
}

/// Dismiss the menu.
export function closeFileRefMenu(): void {
  fileRefMenu.update((s) => ({ ...s, open: false }));
}
