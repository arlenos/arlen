/// Touchpad config store. Same shape and debounce policy as the mouse
/// store — kept separate because the two surfaces have different schemas
/// and the UI reads one at a time.

import { writable, type Readable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

export interface TouchpadConfig {
  tap_to_click: boolean;
  natural_scroll: boolean;
  two_finger_scroll: boolean;
  disable_while_typing: boolean;
  acceleration: number;
  /// `"clickfinger"` | `"areas"`.
  click_method: string;
  tap_drag: boolean;
}

export interface TouchpadState {
  config: TouchpadConfig;
  loading: boolean;
  error: string | null;
  lastSaved: Date | null;
}

const DEFAULT: TouchpadConfig = {
  tap_to_click: true,
  natural_scroll: true,
  two_finger_scroll: true,
  disable_while_typing: true,
  acceleration: 0.0,
  click_method: "clickfinger",
  tap_drag: true,
};

const inner = writable<TouchpadState>({
  config: { ...DEFAULT },
  loading: false,
  error: null,
  lastSaved: null,
});

export const touchpad: Readable<TouchpadState> = { subscribe: inner.subscribe };

let saveTimer: ReturnType<typeof setTimeout> | null = null;

export async function load(): Promise<void> {
  inner.update((s) => ({ ...s, loading: true, error: null }));
  try {
    const config = await invoke<TouchpadConfig>("touchpad_get_config");
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

export function set<K extends keyof TouchpadConfig>(
  key: K,
  value: TouchpadConfig[K]
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
    await invoke("touchpad_set_config", { config: state.config });
    inner.update((s) => ({ ...s, lastSaved: new Date(), error: null }));
  } catch (e) {
    inner.update((s) => ({ ...s, error: String(e) }));
  }
}

function getState(): TouchpadState {
  let state!: TouchpadState;
  inner.subscribe((s) => {
    state = s;
  })();
  return state;
}
