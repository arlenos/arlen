/// The focused app's own quick actions, as a launcher surface.
///
/// **Every layer of this strand was built except the one a person can see.** An
/// app publishes its shortcuts through the shell plugin, the SDK emits
/// `app.shortcut.register`, the shell subscribes to it, forwards it, and keeps a
/// per-app store; the `core.app-shortcuts` Waypointer plugin filters that store
/// by the focused window's app id and returns one result per shortcut; and
/// `app_shortcut_invoke` sends the click back over the bus. The two siblings of
/// that store - badges and ambient effects - are both rendered. This one was not,
/// so an app could publish "Run tests" and nothing would ever list it.
///
/// The plugin is asked by id rather than through the aggregate search, the same
/// way the other dedicated sections work: these are the FOCUSED app's actions and
/// they belong under their own heading, not mixed into the general results where
/// "Push" from the editor sits beside a file called Push.
///
/// See `shortcuts-api.md` FA5.

import { writable, type Readable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// One shortcut the focused app published, in the shell's plugin result shape.
export interface ShortcutResult {
  id: string;
  title: string;
  description: string | null;
  icon: string | null;
  relevance: number;
  action: {
    type?: string;
    handler?: string;
    data?: { appId?: string; action?: string; confirm?: string | null };
  };
  plugin_id: string;
}

const _results = writable<ShortcutResult[]>([]);
export const shortcutResults: Readable<ShortcutResult[]> = {
  subscribe: _results.subscribe,
};

/// The shortcuts this surface may offer.
///
/// A shortcut carrying `confirm` is LEFT OUT rather than listed. The app asked
/// for a yes/no before that action happens (FA7) and the launcher has no dialog
/// to ask it with, so dispatching anyway would run a destructive action the app
/// wanted guarded, and listing it without dispatching would be a row that does
/// nothing. Offering it is what needs the dialog, so it waits for the dialog.
function offerable(list: ShortcutResult[]): ShortcutResult[] {
  return list.filter((r) => !r.action?.data?.confirm);
}

/// Fetch the focused app's shortcuts for a query.
///
/// An empty query is a real question here, not a cleared one: the plugin answers
/// it with the whole list, which is what a person opening the launcher to see
/// what this app can do is asking.
export async function updateShortcutResults(query: string): Promise<void> {
  try {
    const found = await invoke<ShortcutResult[]>("waypointer_search_plugin", {
      pluginId: "core.app-shortcuts",
      query,
    });
    _results.set(offerable(found));
  } catch (e) {
    console.warn("[waypointer] app-shortcut search failed:", e);
    _results.set([]);
  }
}

export function clearShortcutResults(): void {
  _results.set([]);
}

/// Dispatch one shortcut back to the app that published it.
///
/// `windowId` goes empty: the shortcut is per-app by design (FA1), and the
/// receiving side treats an empty window as an app-wide broadcast. A multi-window
/// app that wants the focused window specifically is the case the field exists
/// for, and the launcher does not know it any better than the app does.
export async function runShortcutResult(result: ShortcutResult): Promise<void> {
  const data = result.action?.data;
  if (!data?.appId || !data?.action) return;
  try {
    await invoke("app_shortcut_invoke", {
      appId: data.appId,
      windowId: "",
      action: data.action,
    });
  } catch (e) {
    console.warn("[waypointer] app-shortcut dispatch failed:", e);
  }
}
