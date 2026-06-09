/// Planning for "edit a user message and resend": work out whether a prior user
/// turn can be edited to new text and re-answered, and which messages to keep.
/// Pure, so the decision is unit-tested without the store or the daemon; the
/// store action in `conversation.ts` performs the IO.
import type { Message } from "$lib/stores/conversation";

/// What an edit-and-resend would do: keep the transcript BEFORE the edited user
/// turn, and send `prompt` (the new text) as a fresh question.
export interface EditPlan {
  keep: Message[];
  prompt: string;
}

/// Decide whether the user message with `id` can be edited to `newText` and
/// re-sent, returning the plan or `null` when it cannot.
///
/// Editing replaces a prior question, and everything after it (its answer and
/// any later turns), with a new question. It is refused when: the id does not
/// name a user message, the new text is empty, a turn is still in flight, or the
/// original question carried file attachments. Attachments are not persisted
/// (only their names are), so an edited turn cannot faithfully carry them;
/// refusing is honest, where silently dropping the files would mislead. The new
/// text is trimmed, so an edit that is empty or only whitespace is not a resend.
export function planEdit(messages: Message[], id: number, newText: string): EditPlan | null {
  const prompt = newText.trim();
  if (prompt.length === 0) return null;

  // Never interrupt an in-flight turn anywhere in the conversation.
  if (messages.some((msg) => msg.pending)) return null;

  const i = messages.findIndex((msg) => msg.id === id);
  if (i < 0) return null;

  const target = messages[i];
  if (target.role !== "user") return null;
  if (target.mentions && target.mentions.length > 0) return null;

  return { keep: messages.slice(0, i), prompt };
}
