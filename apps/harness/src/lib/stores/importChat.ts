/// Whether the import-chat dialog is open. Like the mint (share context) flow, the
/// dialog is summoned from the sidebar and mounted once in the layout, so its open
/// state is a shared store.
import { writable } from "svelte/store";

/// True while the import-chat dialog is shown.
export const importOpen = writable(false);

/// Open the import-chat dialog (from the sidebar footer).
export function openImportChat(): void {
  importOpen.set(true);
}

/// Close the import-chat dialog.
export function closeImportChat(): void {
  importOpen.set(false);
}
