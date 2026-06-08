/// Matching for the conversation history rail's search box. Pure, so the
/// behaviour is unit-tested without the store. A conversation matches when the
/// query (case-insensitive, trimmed) appears in its title OR in any of its
/// message texts, so the rail finds a chat by what was said in it, not only by
/// its title.
import type { Session } from "$lib/stores/conversation";

export function sessionMatches(session: Session, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (q.length === 0) return true;
  if (session.title.toLowerCase().includes(q)) return true;
  return session.messages.some((m) => m.text.toLowerCase().includes(q));
}
