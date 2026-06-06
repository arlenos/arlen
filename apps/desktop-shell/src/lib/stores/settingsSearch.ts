/// Settings search store for Waypointer integration.
///
/// Queries the backend's cached settings index when the Waypointer
/// input changes, returning results with pre-read current values so
/// inline actions can render the correct state without a round-trip.

import { writable, type Readable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

export interface SelectOption {
  value: string;
  label: string;
}

export interface InlineAction {
  actionType: "toggle" | "select" | "slider";
  configFile: string;
  configKey: string;
  options?: SelectOption[];
}

export interface IndexedSetting {
  id: string;
  title: string;
  description: string;
  keywords: string[];
  panel: string;
  section: string;
  deepLink: string;
  inlineAction?: InlineAction;
}

export interface SettingsResult {
  setting: IndexedSetting;
  score: number;
  currentValue: unknown;
}

const { subscribe, set } = writable<SettingsResult[]>([]);

export const settingsResults: Readable<SettingsResult[]> = { subscribe };

/// Reload the index from disk. Called when Waypointer opens.
export async function reloadSettingsIndex(): Promise<void> {
  try {
    await invoke("settings_reload_index");
  } catch {
    // Index file missing — no settings results, silent.
  }
}

/// Run a search against the cached index.
export async function searchSettings(query: string): Promise<void> {
  if (!query.trim()) {
    set([]);
    return;
  }
  try {
    const results = await invoke<SettingsResult[]>("settings_search", {
      query,
      limit: 5,
    });
    set(results);
  } catch {
    set([]);
  }
}

/// Clear results (when Waypointer closes or query clears).
export function clearSettingsResults(): void {
  set([]);
}

/// Read the current value of a setting's config key. Used lazily by
/// inline action components after the result is rendered — NOT during
/// bulk search (avoids TOML I/O per keystroke).
export async function getSettingValue(
  configFile: string,
  configKey: string,
): Promise<unknown> {
  try {
    return await invoke("settings_get_value", { configFile, configKey });
  } catch {
    return null;
  }
}

/// Write a value via the generic config writer. The file watchers
/// in the daemon / shell / compositor pick up the change.
export async function setSettingValue(
  configFile: string,
  configKey: string,
  value: unknown,
): Promise<void> {
  await invoke("settings_set_value", { configFile, configKey, value });
}

/// Open the Settings app at the given deep link.
export async function openSettingsDeepLink(
  panel: string,
  anchor?: string,
): Promise<void> {
  await invoke("settings_open_deep_link", { panel, anchor: anchor ?? null });
}
