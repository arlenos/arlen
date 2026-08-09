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
  /// Which side failed, so the page can say the true sentence. A failed READ
  /// leaves the page unable to show the current values; a failed WRITE leaves the
  /// controls showing a value the daemon does not have. Both used to render "Can't
  /// read these settings right now. Changes are paused." - wrong for the second
  /// case, and worse than wrong: changes were not paused, one was kept on screen
  /// and dropped on the way out.
  errorKind: "read" | "write" | null;
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
  errorKind: null,
  lastSaved: null,
});

export const mouse: Readable<MouseState> = { subscribe: inner.subscribe };

let saveTimer: ReturnType<typeof setTimeout> | null = null;

export async function load(): Promise<void> {
  inner.update((s) => ({ ...s, loading: true, error: null, errorKind: null }));
  try {
    const config = await invoke<MouseConfig>("mouse_get_config");
    inner.set({
      config,
      loading: false,
      error: null,
      errorKind: null,
      lastSaved: new Date(),
    });
  } catch (e) {
    inner.update((s) => ({ ...s, loading: false, error: String(e), errorKind: "read" }));
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
    inner.update((s) => ({ ...s, lastSaved: new Date(), error: null, errorKind: null }));
  } catch (e) {
    inner.update((s) => ({ ...s, error: String(e), errorKind: "write" }));
  }
}

function getState(): MouseState {
  let state!: MouseState;
  inner.subscribe((s) => {
    state = s;
  })();
  return state;
}
