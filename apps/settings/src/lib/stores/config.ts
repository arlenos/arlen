/// Generic config store factory backed by Tauri commands.
///
/// Each store owns a copy of the parsed config and a defaults snapshot.
/// `setValue(key, value)` writes through to disk via `config_set` and
/// optimistically updates the local state. `isModified(key)` compares the
/// current value against the defaults snapshot, which powers the "reset"
/// button per row.

import { writable, derived, get, type Readable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

export type ConfigFile =
  | "appearance"
  | "compositor"
  | "shell"
  | "notifications"
  | "modules";

export interface ConfigState<T> {
  data: T | null;
  defaults: T | null;
  loading: boolean;
  error: string | null;
  lastSaved: Date | null;
}

export interface ConfigStore<T> extends Readable<ConfigState<T>> {
  load: () => Promise<void>;
  setValue: (key: string, value: unknown) => Promise<void>;
  reset: (key?: string) => Promise<void>;
  isModified: (key: string) => boolean;
  getValue: <V = unknown>(key: string) => V | undefined;
}

/// Create a typed config store for a specific file.
export function createConfigStore<T>(file: ConfigFile): ConfigStore<T> {
  const inner = writable<ConfigState<T>>({
    data: null,
    defaults: null,
    loading: false,
    error: null,
    lastSaved: null,
  });

  async function load(): Promise<void> {
    inner.update((s) => ({ ...s, loading: true, error: null }));
    try {
      const [data, defaults] = await Promise.all([
        invoke<T>("config_get", { file, key: null }),
        invoke<T>("config_get_default", { file, key: null }),
      ]);
      inner.set({
        data,
        defaults,
        loading: false,
        error: null,
        lastSaved: new Date(),
      });
    } catch (e) {
      inner.update((s) => ({
        ...s,
        loading: false,
        error: String(e),
      }));
    }
  }

  async function setValue(key: string, value: unknown): Promise<void> {
    // Optimistic update.
    inner.update((s) => {
      if (!s.data) return s;
      const next = structuredClone(s.data) as T;
      setByPath(next, key, value);
      return { ...s, data: next };
    });
    try {
      await invoke("config_set", { file, key, value });
      inner.update((s) => ({ ...s, lastSaved: new Date(), error: null }));
    } catch (e) {
      inner.update((s) => ({ ...s, error: String(e) }));
      await load(); // Rollback by re-reading disk.
    }
  }

  async function reset(key?: string): Promise<void> {
    try {
      await invoke("config_reset", { file, key: key ?? null });
      await load();
    } catch (e) {
      inner.update((s) => ({ ...s, error: String(e) }));
    }
  }

  function isModified(key: string): boolean {
    const s = get(inner);
    if (!s.data || !s.defaults) return false;
    const a = getByPath(s.data, key);
    const b = getByPath(s.defaults, key);
    return JSON.stringify(a) !== JSON.stringify(b);
  }

  function getValue<V = unknown>(key: string): V | undefined {
    const s = get(inner);
    if (!s.data) return undefined;
    return getByPath(s.data, key) as V | undefined;
  }

  return {
    subscribe: inner.subscribe,
    load,
    setValue,
    reset,
    isModified,
    getValue,
  };
}

/// Dot-notation property getter. Returns undefined for missing paths.
function getByPath(obj: unknown, path: string): unknown {
  const parts = path.split(".");
  let cur: unknown = obj;
  for (const p of parts) {
    if (cur === null || typeof cur !== "object") return undefined;
    cur = (cur as Record<string, unknown>)[p];
  }
  return cur;
}

/// Dot-notation property setter. Creates intermediate objects.
function setByPath(obj: unknown, path: string, value: unknown): void {
  const parts = path.split(".");
  let cur = obj as Record<string, unknown>;
  for (let i = 0; i < parts.length - 1; i++) {
    const p = parts[i];
    if (typeof cur[p] !== "object" || cur[p] === null) {
      cur[p] = {};
    }
    cur = cur[p] as Record<string, unknown>;
  }
  cur[parts[parts.length - 1]] = value;
}

/// Convenience derived store: extract a single key from a config store.
export function configValue<T, V>(
  store: ConfigStore<T>,
  key: string
): Readable<V | undefined> {
  return derived(store, ($s) => {
    if (!$s.data) return undefined;
    return getByPath($s.data, key) as V | undefined;
  });
}
