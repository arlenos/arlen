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
  /// Which side failed, so the page can say the true sentence. A failed READ
  /// leaves the page unable to show the current values; a failed WRITE leaves the
  /// controls showing a value the daemon does not have. Both used to render "Can't
  /// read these settings right now. Changes are paused." - wrong for the second
  /// case, and worse than wrong: changes were not paused, one was kept on screen
  /// and dropped on the way out.
  errorKind: "read" | "write" | null;
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
  errorKind: null,
  lastSaved: null,
});

export const touchpad: Readable<TouchpadState> = { subscribe: inner.subscribe };

let saveTimer: ReturnType<typeof setTimeout> | null = null;

export async function load(): Promise<void> {
  inner.update((s) => ({ ...s, loading: true, error: null, errorKind: null }));
  try {
    const config = await invoke<TouchpadConfig>("touchpad_get_config");
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
    inner.update((s) => ({ ...s, lastSaved: new Date(), error: null, errorKind: null }));
  } catch (e) {
    inner.update((s) => ({ ...s, error: String(e), errorKind: "write" }));
  }
}

function getState(): TouchpadState {
  let state!: TouchpadState;
  inner.subscribe((s) => {
    state = s;
  })();
  return state;
}
