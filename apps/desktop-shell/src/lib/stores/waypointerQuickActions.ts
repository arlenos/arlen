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
import { get } from "svelte/store";
import { toast } from "svelte-sonner";
import { t } from "$lib/i18n/messages";

export interface QuickActionResult {
  id: string;
  /// Message ids, because the plugin runs where the reader's language is
  /// unknown. Absent for a third-party plugin's own words.
  title_key?: string | null;
  description_key?: string | null;
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
/// key - `quick_action_run` switches on it server-side, performs
/// the work, and emits a `arlen://toast` event for the post-
/// state confirmation.
///
/// A refusal comes back here instead, and needs saying: the launcher closes on
/// Enter, so the only evidence a person has that anything happened is what
/// appears afterwards. Nothing appearing means it worked, and that reading was
/// wrong for every action that failed. The name is the one they just picked
/// from the list, which is why the line can be specific about it.
///
/// IT HAS TO BE SAID IN THE OTHER WINDOW. This raised the toast right here, and
/// the launcher hides in the same breath - so the notice was drawn into a webview
/// on its way out and nobody ever saw it. Measured on the machine: pressing
/// `WLAN umschalten` on an image with no NetworkManager logged the refusal and
/// showed the person an empty desktop. The `arlen://toast` event is the channel
/// that already exists for exactly this - the backend uses it because the
/// waypointer cannot be trusted to still be on screen - and it is a channel this
/// side can use too.
export async function invokeQuickAction(id: string, label?: string): Promise<void> {
  try {
    await invoke("quick_action_run", { id });
  } catch (e) {
    console.warn(`[waypointer] quick action ${id} failed:`, e);
    const { emit } = await import("@tauri-apps/api/event");
    await emit("arlen://toast", {
      kind: "error",
      key: "sh.wp.qaFailed",
      params: { action: label ?? id },
      // Only read if the catalog is missing the id, and legible when it is.
      message: "sh.wp.qaFailed",
    }).catch(() => {
      // No host to carry it: say it here rather than not at all.
      toast.error(get(t)("sh.wp.qaFailed", { action: label ?? id }));
    });
  }
}
