/// Extension/module store.
///
/// Unifies two sources that feed the Extensions panel:
///
///   1. Filesystem modules — discovered by `modules_list` in
///      `/usr/share/lunaris/modules/` and `~/.local/share/lunaris/modules/`.
///
///   2. Built-in Waypointer plugins — compiled into the desktop-shell
///      binary, exposed via `waypointer_list_plugins` which reads the
///      shell-written registry at `~/.local/share/lunaris/waypointer-plugins.toml`.
///
/// Both sources are normalised into the same `ModuleSummary` shape so
/// downstream components (filter, grouping, card) don't need to know
/// where a row came from. Toggle routing picks the right Tauri command
/// based on `source`.

import { writable, derived, type Readable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

export type ModuleSource = "system" | "user" | "builtin";
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

/// Raw shape returned by `waypointer_list_plugins` in app-settings
/// (mirrors the on-disk registry file).
interface PluginSummary {
  id: string;
  name: string;
  description: string;
  source: "builtin";
  enabled: boolean;
  priority: number;
  prefix: string | null;
  pattern: string | null;
}

function pluginToSummary(p: PluginSummary): ModuleSummary {
  return {
    id: p.id,
    name: p.name,
    version: "",
    description: p.description,
    author: "Lunaris",
    moduleType: "system",
    source: "builtin",
    enabled: p.enabled,
    hasWaypointer: true,
    hasTopbar: false,
    hasSettings: false,
    icon: "",
    path: p.prefix ? `built-in · prefix "${p.prefix}"` : "built-in",
    warnings: [],
  };
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
      const [fsModules, plugins] = await Promise.all([
        invoke<ModuleSummary[]>("modules_list"),
        invoke<PluginSummary[]>("waypointer_list_plugins").catch((e) => {
          console.warn("[modules] waypointer_list_plugins failed:", e);
          return [] as PluginSummary[];
        }),
      ]);

      const builtins = plugins.map(pluginToSummary);
      const data = [...builtins, ...fsModules];
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
    // Optimistic update applies to both sources.
    let source: ModuleSource | null = null;
    update((s) => {
      const match = s.data.find((m) => m.id === id);
      source = match?.source ?? null;
      return {
        ...s,
        data: s.data.map((m) => (m.id === id ? { ...m, enabled } : m)),
        restartRequired: true,
      };
    });

    const command =
      source === "builtin" ? "waypointer_set_plugin_enabled" : "modules_set_enabled";
    try {
      await invoke(command, { id, enabled });
    } catch (e) {
      console.error(`[modules] ${command} failed`, e);
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
/// declares no extension at all, it goes to "other". Built-in plugins
/// get their own group to make the difference between shipped-with-OS
/// and user-installed obvious at a glance.
export interface ModuleGroup {
  label: string;
  items: ModuleSummary[];
}

export const moduleGroups: Readable<ModuleGroup[]> = derived(
  modules,
  ($m) => {
    const builtins: ModuleSummary[] = [];
    const waypointer: ModuleSummary[] = [];
    const topbar: ModuleSummary[] = [];
    const settings: ModuleSummary[] = [];
    const other: ModuleSummary[] = [];

    for (const m of $m.data) {
      if (m.source === "builtin") builtins.push(m);
      else if (m.hasWaypointer) waypointer.push(m);
      else if (m.hasTopbar) topbar.push(m);
      else if (m.hasSettings) settings.push(m);
      else other.push(m);
    }

    const groups: ModuleGroup[] = [];
    if (builtins.length > 0)
      groups.push({ label: "Built-in Plugins", items: builtins });
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
