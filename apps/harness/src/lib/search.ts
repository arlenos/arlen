/// Matching for the conversation history rail's search box. Pure, so the
/// behaviour is unit-tested without the store. A conversation matches when the
/// query (case-insensitive, trimmed) appears in its title OR in any of its
/// message texts, so the rail finds a chat by what was said in it, not only by
/// its title.
import type { Session } from "$lib/stores/conversation";

export function sessionMatches(session: Session, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (q.length === 0) return true;
  // The sessions file is persisted as schema-agnostic JSON, so an old or
  // partially corrupted record may carry a non-string title or a message with
  // a missing/non-string `text`, or even a non-array `messages`. Search must
  // not throw over that (it would break the rail for every session), so coerce
  // defensively and treat anything unexpected as empty.
  const text = (v: unknown): string => (typeof v === "string" ? v : "");
  if (text(session.title).toLowerCase().includes(q)) return true;
  const messages = Array.isArray(session.messages) ? session.messages : [];
  return messages.some((m) => text(m?.text).toLowerCase().includes(q));
}
