/// Extension/module store.
///
/// Wraps the `modules_list` / `modules_set_enabled` / `modules_uninstall`
/// Tauri commands. The store is not backed by `createConfigStore`
/// because `modules.toml` is an internal index (just a disabled list),
/// not a user-facing document — we refresh via `modules_list()` which
/// handles discovery + merge in one shot.

import { writable, derived, type Readable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

export type ModuleSource = "system" | "user";
export type ExtensionType = "waypointer" | "topbar" | "settings";

export interface ModuleSummary {
  id: string;
  name: string;
  version: string;
  description: string;
  author: string;
  moduleType: string; // "system" | "first-party" | "third-party"
  source: ModuleSource;
  enabled: boolean;
  hasWaypointer: boolean;
  hasTopbar: boolean;
  hasSettings: boolean;
  icon: string;
  path: string;
  warnings: string[];
}

interface ModulesState {
  data: ModuleSummary[];
  loading: boolean;
  error: string | null;
  /// Set to true after any enable/disable/uninstall action.
  restartRequired: boolean;
}

function createStore() {
  const { subscribe, set, update } = writable<ModulesState>({
    data: [],
    loading: false,
    error: null,
    restartRequired: false,
  });

  async function load() {
    update((s) => ({ ...s, loading: true, error: null }));
    try {
      const data = await invoke<ModuleSummary[]>("modules_list");
      update((s) => ({ ...s, data, loading: false }));
    } catch (e) {
      update((s) => ({
        ...s,
        loading: false,
        error: e instanceof Error ? e.message : String(e),
      }));
    }
  }

  async function setEnabled(id: string, enabled: boolean) {
    // Optimistic.
    update((s) => ({
      ...s,
      data: s.data.map((m) => (m.id === id ? { ...m, enabled } : m)),
      restartRequired: true,
    }));
    try {
      await invoke("modules_set_enabled", { id, enabled });
    } catch (e) {
      // Revert on failure and re-load from disk.
      console.error("[modules] set_enabled failed", e);
      await load();
    }
  }

  async function uninstall(id: string) {
    try {
      await invoke("modules_uninstall", { id });
      await load();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      update((s) => ({ ...s, error: msg }));
      throw e;
    }
  }

  function dismissRestartBanner() {
    update((s) => ({ ...s, restartRequired: false }));
  }

  return {
    subscribe,
    load,
    setEnabled,
    uninstall,
    dismissRestartBanner,
  };
}

export const modules = createStore();

/// Groups for the UI. Each module can fall into multiple groups;
/// the UI shows them under their primary extension type. If a module
/// declares no extension at all, it goes to "other".
export interface ModuleGroup {
  label: string;
  items: ModuleSummary[];
}

export const moduleGroups: Readable<ModuleGroup[]> = derived(
  modules,
  ($m) => {
    const waypointer: ModuleSummary[] = [];
    const topbar: ModuleSummary[] = [];
    const settings: ModuleSummary[] = [];
    const other: ModuleSummary[] = [];

    for (const m of $m.data) {
      if (m.hasWaypointer) waypointer.push(m);
      else if (m.hasTopbar) topbar.push(m);
      else if (m.hasSettings) settings.push(m);
      else other.push(m);
    }

    const groups: ModuleGroup[] = [];
    if (waypointer.length > 0)
      groups.push({ label: "Waypointer Extensions", items: waypointer });
    if (topbar.length > 0)
      groups.push({ label: "Top Bar Extensions", items: topbar });
    if (settings.length > 0)
      groups.push({ label: "Settings Panels", items: settings });
    if (other.length > 0) groups.push({ label: "Other", items: other });
    return groups;
  },
);
