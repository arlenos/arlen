/// Quick-Action results from the `core.quick_actions` plugin.
///
/// Same shape + dispatch pattern as `waypointerPower.ts`. The main
/// difference: dispatch goes through a dedicated
/// `quick_action_run(id)` Tauri command instead of the manager's
/// generic `waypointer_execute` — Quick-Actions need Tauri-managed
/// state (DND, network, theme, …) which the plugin trait can't
/// reach. The plugin's job ends with returning the catalog row
/// (id, title, icon, keywords); the action runs server-side via
/// the dedicated command.

import { writable, type Readable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

export interface QuickActionResult {
  id: string;
  title: string;
  description: string | null;
  icon: string | null;
  relevance: number;
  action: unknown;
  plugin_id: string;
}

const _results = writable<QuickActionResult[]>([]);
export const quickActionResults: Readable<QuickActionResult[]> = {
  subscribe: _results.subscribe,
};

export async function updateQuickActionResults(query: string): Promise<void> {
  if (!query.trim()) {
    _results.set([]);
    return;
  }
  try {
    const r = await invoke<QuickActionResult[]>(
      "waypointer_search_plugin",
      { pluginId: "core.quick_actions", query },
    );
    _results.set(r);
  } catch (e) {
    console.warn("[waypointer] quick-actions search failed:", e);
    _results.set([]);
  }
}

export function clearQuickActionResults(): void {
  _results.set([]);
}

/// Dispatch the action by id. The id is the catalog's `qa.<name>`
/// key — `quick_action_run` switches on it server-side, performs
/// the work, and emits a `lunaris://toast` event for the post-
/// state confirmation.
export async function invokeQuickAction(id: string): Promise<void> {
  try {
    await invoke("quick_action_run", { id });
  } catch (e) {
    console.warn(`[waypointer] quick action ${id} failed:`, e);
  }
}
