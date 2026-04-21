/// Dictionary-definition results from the `core.dict` plugin.
///
/// Offline-only English definitions via Princeton WordNet. The
/// backend lazy-loads the corpus on the first query and returns empty
/// until the data is either ready or confirmed missing — so the
/// "Definitions" section simply won't render on systems without
/// WordNet installed. No error state, no install prompt; graceful
/// degradation is the contract.

import { writable, type Readable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

export interface DictResult {
    id: string;
    title: string;
    description: string | null;
    icon: string | null;
    relevance: number;
    action: unknown;
    plugin_id: string;
}

const _results = writable<DictResult[]>([]);
export const dictResults: Readable<DictResult[]> = {
    subscribe: _results.subscribe,
};

/// Fetch fresh results. Empty query -> empty store. Short queries
/// (fewer than 2 chars) also short-circuit: single letters are almost
/// never what the user means when they want a definition.
export async function updateDictResults(query: string): Promise<void> {
    const trimmed = query.trim();
    if (trimmed.length < 2) {
        _results.set([]);
        return;
    }
    try {
        const r = await invoke<DictResult[]>("waypointer_search_plugin", {
            pluginId: "core.dict",
            query,
        });
        _results.set(r);
    } catch (e) {
        console.warn("[waypointer] dict search failed:", e);
        _results.set([]);
    }
}

export function clearDictResults(): void {
    _results.set([]);
}
