/// Composer prompt history: recall previously-sent prompts shell-style with the
/// up and down keys. Both functions are pure, so the navigation is unit-tested
/// without the composer; the composer derives the history from the active
/// conversation and tracks the index.
import type { Message } from "$lib/stores/conversation";

/// The prompts sent in a conversation, oldest first: the user turns with actual
/// text (an attachment-only turn carries no recallable prompt).
export function promptHistory(messages: Message[]): string[] {
  return messages
    .filter((msg) => msg.role === "user" && msg.text.trim().length > 0)
    .map((msg) => msg.text);
}

/// One step of history navigation, returning the new position and the text to
/// show. `index` is the current position (`null` means the live composer, not in
/// history). "older" steps back in time (the up key), "newer" steps forward (the
/// down key).
///
/// From the live composer, "older" recalls the most recent prompt; stepping
/// "older" walks toward the start and stops at the oldest. Stepping "newer" walks
/// back toward the present and, past the newest prompt, returns to the live
/// composer (`index: null`, empty text), so the caller can restore the draft the
/// user was typing. An empty history is always the live composer.
export interface HistoryNav {
  /// Position in the history, or `null` for the live composer.
  index: number | null;
  /// The recalled prompt, or the empty string at the live composer.
  text: string;
}

export function navigateHistory(
  history: string[],
  index: number | null,
  direction: "older" | "newer",
): HistoryNav {
  if (history.length === 0) return { index: null, text: "" };

  if (direction === "older") {
    const next = index === null ? history.length - 1 : Math.max(0, index - 1);
    return { index: next, text: history[next] };
  }

  // "newer": already-live stays live; past the newest entry returns to live.
  if (index === null) return { index: null, text: "" };
  const next = index + 1;
  if (next >= history.length) return { index: null, text: "" };
  return { index: next, text: history[next] };
}
