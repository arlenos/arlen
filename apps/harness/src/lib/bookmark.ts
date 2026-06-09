/// Pure logic for bookmarking messages: toggle a bookmark and list the
/// bookmarked turns. Unit-tested without the store; the `togglePin` action in
/// `conversation.ts` applies the toggle and persists.
import type { Message } from "$lib/stores/conversation";

/// Toggle the bookmark on the message identified by `id`, returning a new
/// messages array (the originals are never mutated). Pinning sets `pinned: true`;
/// unpinning removes the flag entirely, so an unmarked message carries no
/// `pinned` key. A message in flight is not bookmarkable (a placeholder is not a
/// settled turn) and an unknown id leaves the array unchanged.
export function togglePinned(messages: Message[], id: number): Message[] {
  return messages.map((msg) => {
    if (msg.id !== id || msg.pending) return msg;
    if (msg.pinned) {
      const next = { ...msg };
      delete next.pinned;
      return next;
    }
    return { ...msg, pinned: true };
  });
}

/// The bookmarked messages, in conversation order (the bookmarks view).
export function pinnedMessages(messages: Message[]): Message[] {
  return messages.filter((msg) => msg.pinned);
}
