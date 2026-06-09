/// Planning for "fork a conversation here": work out the message prefix a new
/// branch starts from. Pure, so the decision is unit-tested without the store;
/// the store action in `conversation.ts` creates the new session from it.
import type { Message } from "$lib/stores/conversation";

/// Decide the prefix a fork would copy into a new conversation: every message up
/// to and including the one identified by `id`, or `null` when the fork is not
/// allowed.
///
/// Forking branches the conversation at a chosen turn into an independent new
/// session, so the user can explore an alternative without disturbing the
/// original. The prefix is everything through the selected message; the original
/// is left untouched. It is refused while a turn is in flight (the branch point
/// must be settled) or when the id is not present.
export function planFork(messages: Message[], id: number): Message[] | null {
  if (messages.some((msg) => msg.pending)) return null;

  const i = messages.findIndex((msg) => msg.id === id);
  if (i < 0) return null;

  return messages.slice(0, i + 1);
}
