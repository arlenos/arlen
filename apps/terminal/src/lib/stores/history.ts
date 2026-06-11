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
  } catch {
    historyResults.set([]);
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
