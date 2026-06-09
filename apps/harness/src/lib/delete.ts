/// Planning for "delete a turn": work out the conversation that results from
/// removing a message (and, for a question, the answer that belongs with it).
/// Pure, so the decision is unit-tested without the store or the daemon; the
/// store action in `conversation.ts` performs the IO (and the persist).
import type { Message } from "$lib/stores/conversation";

/// Decide the messages that remain after deleting the turn identified by `id`,
/// or `null` when the delete is not allowed.
///
/// Deleting a user message removes its answer too (the response immediately
/// after it), so a question and its reply leave together rather than stranding a
/// reply with no question. Deleting an assistant or error message removes only
/// that reply; the question stays and is regeneratable. It is refused while a
/// turn is in flight (never delete mid-stream) or when the id is not present. An
/// empty result is a valid outcome (the last turn was deleted), distinct from
/// `null` (the delete was not allowed).
export function planDelete(messages: Message[], id: number): Message[] | null {
  if (messages.some((msg) => msg.pending)) return null;

  const i = messages.findIndex((msg) => msg.id === id);
  if (i < 0) return null;

  // A user turn takes its following reply (an assistant or error message) with
  // it; anything else removes just itself.
  const takesReply =
    messages[i].role === "user" && i + 1 < messages.length && messages[i + 1].role !== "user";
  const removeCount = takesReply ? 2 : 1;

  return [...messages.slice(0, i), ...messages.slice(i + removeCount)];
}
