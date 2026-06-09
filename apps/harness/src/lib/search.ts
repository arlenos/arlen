/// Matching for the conversation history rail's search box. Pure, so the
/// behaviour is unit-tested without the store. A conversation matches when the
/// query (case-insensitive, trimmed) appears in its title OR in any of its
/// message texts, so the rail finds a chat by what was said in it, not only by
/// its title.
import type { Message, Session } from "$lib/stores/conversation";

/// Coerce a possibly-non-string field to a string, so a corrupt persisted record
/// cannot make search throw (the sessions file is schema-agnostic JSON).
function asText(v: unknown): string {
  return typeof v === "string" ? v : "";
}

export function sessionMatches(session: Session, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (q.length === 0) return true;
  if (asText(session.title).toLowerCase().includes(q)) return true;
  const messages = Array.isArray(session.messages) ? session.messages : [];
  return messages.some((m) => asText(m?.text).toLowerCase().includes(q));
}

/// The messages in the open conversation whose text matches `query`
/// (case-insensitive, trimmed), for an in-conversation find-and-jump. An empty
/// query matches nothing here, unlike `sessionMatches` (which returns every
/// session for an empty rail query): an in-conversation find with no term has
/// nothing to highlight. Coerces defensively, like `sessionMatches`.
export function matchingMessages(messages: Message[], query: string): Message[] {
  const q = query.trim().toLowerCase();
  if (q.length === 0) return [];
  return messages.filter((m) => asText(m?.text).toLowerCase().includes(q));
}
