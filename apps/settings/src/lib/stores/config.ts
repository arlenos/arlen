/// Generic config store factory backed by Tauri commands.
///
/// Each store owns a copy of the parsed config and a defaults snapshot.
/// `setValue(key, value)` writes through to disk via `config_set` and
/// optimistically updates the local state. `isModified(key)` compares the
/// current value against the defaults snapshot, which powers the "reset"
/// button per row.

import { writable, derived, get, type Readable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { tauriAvailable } from "$lib/tauri";

export type ConfigFile =
  | "appearance"
  | "compositor"
  | "shell"
  | "notifications"
  | "modules"
  | "graph"
  | "ai"
  | "locale";

export interface ConfigState<T> {
  data: T | null;
  defaults: T | null;
  loading: boolean;
  error: string | null;
  /// True when the LAST WRITE was refused, kept apart from `error` and from the
  /// rollback that follows it.
  ///
  /// It used to be neither. `setValue`'s catch put the reason in `error` and then
  /// called `load()` to roll back - and `load()` begins by setting `error: null`,
  /// so the write failure was erased one line later by the very thing meant to
  /// recover from it. What a person saw was a switch they had just moved sliding
  /// back on its own with nothing said anywhere, on every control in this app
  /// that goes through this store.
  ///
  /// And had it survived, it would have said the wrong thing: `error` is rendered
  /// as "your configuration could not be read", which after a successful rollback
  /// is false twice over - the read worked, and it is the write that did not.
  writeFailed: boolean;
  /// True when there is no Tauri host to ask at all - a browser tab, a
  /// screenshot run, `vite dev`. NOT an error: nothing failed, there is simply
  /// nobody to answer. Kept apart from `error` because a page that renders "could
  /// not load your settings" in a window that was never going to have a backend
  /// describes a broken system to someone looking at a working one.
  hostless: boolean;
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
    writeFailed: false,
    loading: false,
    error: null,
    hostless: false,
    lastSaved: null,
  });

  async function load(): Promise<void> {
    // Asked before the try, because "no host" is not a failure to catch. The
    // invoke would throw a `window.__TAURI_INTERNALS__` TypeError, which pages
    // then had to pattern-match out of the message to avoid printing a stack
    // trace at the user - the state is cleaner than the string.
    if (!tauriAvailable) {
      inner.update((s) => ({ ...s, loading: false, error: null, hostless: true }));
      return;
    }
    // `writeFailed` is deliberately NOT cleared here: this runs as the rollback
    // for a refused write, and clearing it is what erased the only record that
    // anything had gone wrong.
    inner.update((s) => ({ ...s, loading: true, error: null, hostless: false }));
    try {
      const [data, defaults] = await Promise.all([
        invoke<T>("config_get", { file, key: null }),
        invoke<T>("config_get_default", { file, key: null }),
      ]);
      inner.update((s) => ({
        ...s,
        data,
        defaults,
        loading: false,
        error: null,
        hostless: false,
        lastSaved: new Date(),
      }));
    } catch (e) {
      inner.update((s) => ({
        ...s,
        loading: false,
        error: String(e),
      }));
    }
  }

  async function setValue(key: string, value: unknown): Promise<void> {
    inner.update((s) => ({ ...s, writeFailed: false }));
    // Optimistic update.
    inner.update((s) => {
      if (!s.data) return s;
      const next = structuredClone(s.data) as T;
      setByPath(next, key, value);
      return { ...s, data: next };
    });
    try {
      await invoke("config_set", { file, key, value });
      inner.update((s) => ({ ...s, lastSaved: new Date(), error: null, writeFailed: false }));
    } catch {
      await load(); // Rollback by re-reading disk.
      inner.update((s) => ({ ...s, writeFailed: true }));
    }
  }

  async function reset(key?: string): Promise<void> {
    inner.update((s) => ({ ...s, writeFailed: false }));
    try {
      await invoke("config_reset", { file, key: key ?? null });
      await load();
    } catch {
      // A refused reset is a refused WRITE, and it was left on `error` when its
      // sibling `setValue` was moved off it - so pressing "put this back" and
      // being refused still said the settings could not be read. The same defect
      // one method down, missed by the fix for it an hour earlier.
      inner.update((s) => ({ ...s, writeFailed: true }));
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
