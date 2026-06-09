/// Per-conversation composer drafts: the text typed but not yet sent, kept per
/// session so switching conversations (or reloading) never loses it. The pure
/// `applyDraft` is unit-tested; the store wrappers below are the thin IO.
import { get, writable } from "svelte/store";

/// Compute the drafts map after setting `sessionId`'s draft to `text`, returning
/// a new map (the input is never mutated). A whitespace-only or empty draft is
/// removed rather than stored, so a cleared composer leaves no entry and the map
/// stays minimal. The text is stored verbatim when kept (leading and trailing
/// whitespace the user typed is preserved); only the keep-or-drop decision trims.
export function applyDraft(
  drafts: Record<string, string>,
  sessionId: string,
  text: string,
): Record<string, string> {
  if (text.trim().length === 0) {
    if (!(sessionId in drafts)) return drafts;
    const next = { ...drafts };
    delete next[sessionId];
    return next;
  }
  return { ...drafts, [sessionId]: text };
}

/// The composer drafts, keyed by session id. In-memory for the session; the
/// composer reads and writes it as the user types and switches conversations.
export const drafts = writable<Record<string, string>>({});

/// Save (or clear) the draft for a session.
export function setDraft(sessionId: string, text: string): void {
  drafts.update((d) => applyDraft(d, sessionId, text));
}

/// Read a session's draft, or the empty string when there is none.
export function getDraft(sessionId: string): string {
  return get(drafts)[sessionId] ?? "";
}

/// Clear a session's draft (e.g. after its text is sent).
export function clearDraft(sessionId: string): void {
  setDraft(sessionId, "");
}
