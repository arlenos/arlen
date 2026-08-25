/// History state for the Ctrl+R palette: the query, the filter set
/// and the results, all in writable stores (IPC-callback rule). The
/// search runs against `terminal_history_search`; typing is debounced
/// so every keystroke does not become a backend call.

import { writable, get } from "svelte/store";
import {
  terminalHistorySearch,
  emptyFilters,
  type Block,
  type Origin,
} from "$lib/contract";

/// Whether the history palette is on screen (Ctrl+R toggles it).
export const historyPaletteOpen = writable(false);

/// The free-text query over past commands.
export const historyQuery = writable("");

/// Filter: only blocks whose command failed.
export const historyOnlyFailures = writable(false);

/// Filter: only blocks the agent issued.
export const historyAgentOnly = writable(false);

/// Filter: scope to one project (the palette's project chips).
export const historyProjectId = writable<string | null>(null);

/// The current result set, in the backend's order.
export const historyResults = writable<Block[]>([]);

/// True once the first search answered; before that the list shows
/// nothing instead of claiming "no matches".
export const historyLoaded = writable(false);

/// True when the last search did not run.
///
/// Distinct from an empty result, and the palette needs both: a failed search
/// used to set `historyResults` to `[]` and `historyLoaded` to true, so the
/// palette said "Keine passenden Befehle" or "Befehle, die du ausführst, landen
/// hier" - statements about the person's own history, made after failing to read
/// it. The file manager's search results already carry this three-way
/// distinction; this is the same, one app over.
///
/// WHAT IT CAN ACTUALLY DETECT, since a flag that overstates its reach is the
/// thing it exists to prevent: the invoke rejecting. That is no runtime at all,
/// and - since the command was changed to return a `Result` alongside this - a
/// session registry whose lock is poisoned. A project-scoped search still comes
/// back as an ordinary empty list, deliberately: the palette says project scopes
/// are unavailable on this system before the chip can be set, so the person has
/// already been told and a second sentence about it would be noise.
export const historyUnavailable = writable(false);

let debounce: ReturnType<typeof setTimeout> | null = null;

export function openHistoryPalette(): void {
  historyPaletteOpen.set(true);
}

export function closeHistoryPalette(): void {
  historyPaletteOpen.set(false);
}

/// Run the search with the current query and filters.
export async function runHistorySearch(): Promise<void> {
  const filters = {
    ...emptyFilters(),
    only_failures: get(historyOnlyFailures),
    origin: get(historyAgentOnly) ? ("agent" as Origin) : null,
    project_id: get(historyProjectId),
  };
  try {
    historyResults.set(await terminalHistorySearch(get(historyQuery), filters));
    historyUnavailable.set(false);
  } catch (e) {
    console.warn("terminal: the history search did not run", e);
    historyResults.set([]);
    historyUnavailable.set(true);
  }
  historyLoaded.set(true);
}

/// Debounced variant for keystrokes and chip toggles.
export function queueHistorySearch(): void {
  if (debounce) clearTimeout(debounce);
  debounce = setTimeout(() => {
    runHistorySearch();
  }, 150);
}
