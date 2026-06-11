/// The one channel into the composer from elsewhere in the UI: a
/// history row click hands its command over here; the composer takes
/// it as the draft, focuses, and clears the store.

import { writable } from "svelte/store";

/// Text waiting to become the composer draft; null when nothing is
/// pending.
export const composerPrefill = writable<string | null>(null);

/// Put a command into the composer (it replaces the current draft and
/// takes focus; nothing is executed).
export function prefillComposer(text: string): void {
  composerPrefill.set(text);
}
