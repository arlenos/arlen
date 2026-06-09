/// Pure ordering for the conversation rail: pinned conversations float to the
/// top. Unit-tested without the store; the `togglePinSession` action and the
/// `orderedSessions` derived store in `conversation.ts` apply it.
import type { Session } from "$lib/stores/conversation";

/// Return the sessions with pinned ones first, preserving each group's existing
/// relative order (a stable partition, not a comparator sort): the store keeps
/// sessions newest-first, and that order is left intact within the pinned and
/// unpinned groups, so pinning only lifts a conversation above the unpinned
/// ones, never reshuffles them.
export function sortSessions(sessions: Session[]): Session[] {
  const pinned = sessions.filter((s) => s.pinned);
  const rest = sessions.filter((s) => !s.pinned);
  return [...pinned, ...rest];
}
