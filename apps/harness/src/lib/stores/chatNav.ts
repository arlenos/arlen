/// Cross-component chat navigation: a one-shot request to scroll the transcript to
/// a message. The bookmarks affordance (in the composer foot) sets it; ChatThread,
/// which owns the scroll region, consumes it and resets it to null.
import { writable } from "svelte/store";

/// The id of a message to scroll into view, or null when there is no pending jump.
export const jumpToMessage = writable<number | null>(null);
