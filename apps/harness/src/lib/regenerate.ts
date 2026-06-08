/// Planning for "regenerate the last response": work out whether the active
/// conversation can be re-answered and, if so, which messages to keep and what
/// prompt to re-send. Pure, so the decision is unit-tested without the store or
/// the daemon; the store action in `conversation.ts` performs the IO.
import type { Message } from "$lib/stores/conversation";

/// What a regenerate would do: keep the transcript up to and including the user
/// turn being re-answered, and re-send `prompt`.
export interface RegenPlan {
  keep: Message[];
  prompt: string;
}

/// Decide whether the conversation can be regenerated, returning the plan or
/// `null` when it cannot.
///
/// Regeneration replaces the most recent settled response (an assistant answer
/// or an error) with a fresh one for the same question. It is refused when:
/// there is nothing to regenerate, a turn is still in flight, or the question
/// carried file attachments. Attachments are not persisted (only their names
/// are), so the original prompt cannot be faithfully rebuilt; refusing is
/// honest, where silently re-asking without the files would not be.
export function planRegenerate(messages: Message[]): RegenPlan | null {
  if (messages.length === 0) return null;

  const last = messages[messages.length - 1];
  // Only a settled response is regenerable: never interrupt an in-flight turn,
  // and there must actually be a response to replace.
  if (last.pending) return null;
  if (last.role !== "assistant" && last.role !== "error") return null;

  // Find the user turn this response answers (the most recent user message).
  let i = messages.length - 1;
  while (i >= 0 && messages[i].role !== "user") i--;
  if (i < 0) return null;

  const userMsg = messages[i];
  if (userMsg.mentions && userMsg.mentions.length > 0) return null;

  return { keep: messages.slice(0, i + 1), prompt: userMsg.text };
}
