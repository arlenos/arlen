/// Power-action results from the `core.power` plugin.
///
/// The Waypointer frontend historically bypassed the generic plugin
/// manager — each plugin had its own Tauri command (`search_apps`,
/// `update_window_results`, `evaluate_waypointer_input`, …). That
/// worked as long as every plugin needed its own specialised UX,
/// but it left `core.power` stranded: the backend registered it in
/// the manager, but no frontend path ever invoked the manager. The
/// result was "shutdown"/"sleep"/"lock" returning zero visible
/// results despite the plugin being healthy.
///
/// Rather than bolt on a per-plugin Tauri command for every new
/// trait implementation, this store goes through the generic
/// `waypointer_search_plugin(plugin_id, query)` bridge added to the
/// manager in the same change. Future plugins that want a dedicated
/// CommandGroup section can do the same — create a parallel store,
/// call the bridge with their plugin id.

import { writable, type Readable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// Mirrors the Rust `SearchResult` struct. Kept minimal — the Svelte
/// side only reads `id`, `title`, `description`, `icon`; the whole
/// object is round-tripped back to Rust via `waypointer_execute`.
export interface PowerActionResult {
    id: string;
    title: string;
    description: string | null;
    icon: string | null;
    relevance: number;
    action: unknown;
    plugin_id: string;
}

const _results = writable<PowerActionResult[]>([]);
export const powerResults: Readable<PowerActionResult[]> = {
    subscribe: _results.subscribe,
};

/// Fetch fresh results for the given query. Empty query → empty
/// store (the frontend hides the section on .length === 0). Failure
/// is silent — a broken plugin or backend hiccup shouldn't surface
/// as an error in the Waypointer.
export async function updatePowerResults(query: string): Promise<void> {
    if (!query.trim()) {
        _results.set([]);
        return;
    }
    try {
        const r = await invoke<PowerActionResult[]>("waypointer_search_plugin", {
            pluginId: "core.power",
            query,
        });
        _results.set(r);
    } catch (e) {
        console.warn("[waypointer] power search failed:", e);
        _results.set([]);
    }
}

export function clearPowerResults(): void {
    _results.set([]);
}

/// Invoke a power action via the manager's generic `execute` path.
/// The result object carries `plugin_id` + `action::Custom` so the
/// Rust side dispatches correctly to `PowerPlugin::execute`.
export async function invokePowerAction(
    result: PowerActionResult,
): Promise<void> {
    try {
        await invoke("waypointer_execute", { result });
    } catch (e) {
        console.warn("[waypointer] power execute failed:", e);
    }
}
