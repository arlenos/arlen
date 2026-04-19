/// Mouse config store.
///
/// Debounced save: sliders fire `set()` on every value change but the
/// actual disk write only happens 300ms after the last update to avoid
/// thrashing the compositor's inotify watcher.

import { writable, type Readable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

export interface MouseConfig {
  acceleration: number;
  natural_scroll: boolean;
  left_handed: boolean;
  scroll_speed: number;
}

export interface MouseState {
  config: MouseConfig;
  loading: boolean;
  error: string | null;
  lastSaved: Date | null;
}

const DEFAULT: MouseConfig = {
  acceleration: 0.0,
  natural_scroll: false,
  left_handed: false,
  scroll_speed: 1.0,
};

const inner = writable<MouseState>({
  config: { ...DEFAULT },
  loading: false,
  error: null,
  lastSaved: null,
});

export const mouse: Readable<MouseState> = { subscribe: inner.subscribe };

let saveTimer: ReturnType<typeof setTimeout> | null = null;

export async function load(): Promise<void> {
  inner.update((s) => ({ ...s, loading: true, error: null }));
  try {
    const config = await invoke<MouseConfig>("mouse_get_config");
    inner.set({
      config,
      loading: false,
      error: null,
      lastSaved: new Date(),
    });
  } catch (e) {
    inner.update((s) => ({ ...s, loading: false, error: String(e) }));
  }
}

/// Optimistically update a single field and schedule a debounced write.
export function set<K extends keyof MouseConfig>(
  key: K,
  value: MouseConfig[K]
): void {
  inner.update((s) => ({
    ...s,
    config: { ...s.config, [key]: value },
  }));
  if (saveTimer) clearTimeout(saveTimer);
  saveTimer = setTimeout(flush, 300);
}

export async function flush(): Promise<void> {
  if (saveTimer) {
    clearTimeout(saveTimer);
    saveTimer = null;
  }
  const state = getState();
  try {
    await invoke("mouse_set_config", { config: state.config });
    inner.update((s) => ({ ...s, lastSaved: new Date(), error: null }));
  } catch (e) {
    inner.update((s) => ({ ...s, error: String(e) }));
  }
}

function getState(): MouseState {
  let state!: MouseState;
  inner.subscribe((s) => {
    state = s;
  })();
  return state;
}
