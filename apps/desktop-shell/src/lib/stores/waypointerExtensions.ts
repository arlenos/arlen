/// What the installed extensions found, as a launcher surface.
///
/// **Nothing in this app asked the module runtime for results until this
/// existed.** The aggregate command that does (`waypointer_search`) has no
/// caller, so the whole Tier 1 half of the launcher - discovery, capabilities,
/// consent, the WASM host - could work perfectly and never put a row in front
/// of a person. This is the one call that closes that.
///
/// Modules only, never the aggregate: every builtin already has its own
/// section, so asking for both would render files, clipboard and the rest
/// twice.
///
/// "Extensions" here, "modules" in the daemon: the word a person reads is the
/// one Settings uses, and the word the runtime uses is the one in `modulesd`.
/// Distinct from `$lib/modules/moduleSearchStore.js`, which is the Tier 2
/// iframe path over postMessage and is a different mechanism entirely.
///
/// The runtime is asked best-effort. A module host that is down, slow or absent
/// leaves this empty and the rest of the launcher answers as it always did; a
/// launcher that stalls because an extension host is wedged is worse than one
/// that shows fewer rows.

import { writable, type Readable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// One result from a module, in the shape the shell's own plugins use.
///
/// `action` stays opaque here on purpose. What a module may ask the shell to do
/// is bounded on the Rust side (`module_results::accept`, then `dispatch` again
/// at execute time, because the value has been out to the webview and back);
/// giving this file an opinion about the shape would be a second, weaker copy of
/// that rule.
export interface ExtensionResult {
    id: string;
    title: string;
    description: string | null;
    icon: string | null;
    relevance: number;
    action: unknown;
    plugin_id: string;
}

const _results = writable<ExtensionResult[]>([]);
export const extensionResults: Readable<ExtensionResult[]> = {
    subscribe: _results.subscribe,
};

/// Fetch fresh results for a query. An empty query clears.
export async function updateExtensionResults(query: string): Promise<void> {
    if (!query.trim()) {
        _results.set([]);
        return;
    }
    try {
        _results.set(await invoke<ExtensionResult[]>("waypointer_search_modules", { query }));
    } catch (e) {
        console.warn("[waypointer] module search failed:", e);
        _results.set([]);
    }
}

export function clearExtensionResults(): void {
    _results.set([]);
}

/// Act on a module's result, in the shell's own hands.
///
/// The value goes back to Rust rather than being interpreted here, and
/// `waypointer_execute` re-checks what the module asked for before doing it -
/// the result has been out to the webview and back since the shell last looked
/// at it, so the check on the way out is not enough.
export async function runExtensionResult(result: ExtensionResult): Promise<void> {
    try {
        await invoke("waypointer_execute", { result });
    } catch (e) {
        console.warn("[waypointer] module action refused:", e);
    }
}
