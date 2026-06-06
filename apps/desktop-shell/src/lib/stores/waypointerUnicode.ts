import { invoke } from "@tauri-apps/api/core";
import { writable } from "svelte/store";

export interface UnicodeChar {
    codepoint: number;
    char_str: string;
    name: string;
    codepoint_hex: string;
}

/// Filtered unicode results for Waypointer.
export const unicodeResults = writable<UnicodeChar[]>([]);

/// Searches unicode characters by name or codepoint.
export function updateUnicodeResults(query: string) {
    if (!query) {
        unicodeResults.set([]);
        return;
    }
    invoke<UnicodeChar[]>("search_unicode", { query })
        .then((r) => { unicodeResults.set(r); })
        .catch(() => { unicodeResults.set([]); });
}

/// Clears unicode results.
export function clearUnicodeResults() {
    unicodeResults.set([]);
}
