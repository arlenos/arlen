/// Cross-component chat navigation: a one-shot request to scroll the transcript to
/// a message. The bookmarks affordance (in the composer foot) sets it; ChatThread,
/// which owns the scroll region, consumes it and resets it to null.
import { writable } from "svelte/store";

/// The id of a message to scroll into view, or null when there is no pending jump.
export const jumpToMessage = writable<number | null>(null);

/// Whether the bookmarks drawer is open. Like the transparency drawer, it is summoned
/// from the composer foot and mounted once in the layout, so its open state is shared.
export const bookmarksOpen = writable(false);

/// Open the bookmarks drawer (from the composer foot).
export function openBookmarks(): void {
  bookmarksOpen.set(true);
}

/// Whether the find-in-chat bar is open. Summoned from the composer foot or Ctrl+F,
/// the bar itself lives in ChatThread (which owns the scroll region).
export const findOpen = writable(false);

/// Open the find-in-chat bar.
export function openFind(): void {
  findOpen.set(true);
}
