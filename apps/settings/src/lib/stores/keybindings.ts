/// Keybindings store.
///
/// Reads the full catalogue + user overrides from the backend, exposes
/// categorised lists for the UI, and runs a local conflict scan so the
/// UI can warn the user while they are editing.

import { writable, derived, get, type Readable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

export interface KeybindingEntry {
  id: string;
  action: string;
  binding: string | null;
  defaultBinding: string | null;
  isCustom: boolean;
  category: string;
  label: string;
  description: string | null;
  /// Owning module id for entries coming from
  /// compositor.d/keybindings.d/ fragments.
  moduleId: string | null;
}

export interface Conflict {
  binding: string;
  actions: string[];
}

export interface KeybindingsState {
  entries: KeybindingEntry[];
  conflicts: Conflict[];
  loading: boolean;
  error: string | null;
  lastSaved: Date | null;
}

interface RawEntry {
  id: string;
  action: string;
  binding: string | null;
  default_binding: string | null;
  is_custom: boolean;
  category: string;
  label: string;
  description: string | null;
  module_id?: string | null;
}

function fromRaw(r: RawEntry): KeybindingEntry {
  return {
    id: r.id,
    action: r.action,
    binding: r.binding,
    defaultBinding: r.default_binding,
    isCustom: r.is_custom,
    category: r.category,
    label: r.label,
    description: r.description,
    moduleId: r.module_id ?? null,
  };
}

const inner = writable<KeybindingsState>({
  entries: [],
  conflicts: [],
  loading: false,
  error: null,
  lastSaved: null,
});

export const keybindings: Readable<KeybindingsState> = { subscribe: inner.subscribe };

/// Load everything fresh from disk.
export async function load(): Promise<void> {
  inner.update((s) => ({ ...s, loading: true, error: null }));
  try {
    const [entriesRaw, conflicts] = await Promise.all([
      invoke<RawEntry[]>("keybindings_get_all"),
      invoke<Conflict[]>("keybindings_get_conflicts"),
    ]);
    inner.set({
      entries: entriesRaw.map(fromRaw),
      conflicts,
      loading: false,
      error: null,
      lastSaved: new Date(),
    });
  } catch (e) {
    inner.update((s) => ({ ...s, loading: false, error: String(e) }));
  }
}

/// Change the accelerator for a catalogue action. Pass `null` to clear.
export async function setBinding(
  action: string,
  binding: string | null
): Promise<void> {
  try {
    await invoke("keybindings_set", { action, binding });
    await load();
  } catch (e) {
    inner.update((s) => ({ ...s, error: String(e) }));
    throw e;
  }
}

/// Insert a custom binding (action string is already composed by the caller).
export async function addCustom(
  binding: string,
  action: string
): Promise<void> {
  try {
    await invoke("keybindings_add_custom", { binding, action });
    await load();
  } catch (e) {
    inner.update((s) => ({ ...s, error: String(e) }));
    throw e;
  }
}

/// Remove a binding by accelerator or catalogue action.
export async function remove(keyOrAction: string): Promise<void> {
  try {
    await invoke("keybindings_remove", { keyOrAction });
    await load();
  } catch (e) {
    inner.update((s) => ({ ...s, error: String(e) }));
    throw e;
  }
}

/// Reset an entry to its default binding, or clear if no default exists.
export async function resetOne(action: string): Promise<void> {
  const state = get(inner);
  const entry = state.entries.find((e) => e.action === action);
  if (!entry) return;
  await setBinding(action, entry.defaultBinding);
}

/// Reset the whole section to compositor defaults.
export async function resetAll(): Promise<void> {
  try {
    await invoke("keybindings_reset_all");
    await load();
  } catch (e) {
    inner.update((s) => ({ ...s, error: String(e) }));
    throw e;
  }
}

/// Remove the `[keybindings]` section entirely so the compositor
/// uses its built-in defaults. Preferred over `resetAll` when the
/// user wants to "start fresh" — future default changes will apply
/// automatically instead of being frozen in the user's TOML.
export async function resetToBuiltinDefaults(): Promise<void> {
  try {
    await invoke("keybindings_reset_all_to_defaults");
    await load();
  } catch (e) {
    inner.update((s) => ({ ...s, error: String(e) }));
    throw e;
  }
}

/// Delete every fragment file under
/// `~/.config/lunaris/compositor.d/keybindings.d/`. Returns the
/// number of files removed.
export async function resetModuleFragments(): Promise<number> {
  try {
    const removed = await invoke<number>("keybindings_reset_module_fragments");
    await load();
    return removed;
  } catch (e) {
    inner.update((s) => ({ ...s, error: String(e) }));
    throw e;
  }
}

/// Ask the compositor's D-Bus service which dynamic bindings would
/// collide with the given accelerator. Soft-fails to `[]` when the
/// compositor is not running, so the UI can keep working offline.
export interface LiveConflict {
  binding: string;
  existingAction: string;
  existingScope: string;
  existingOwner: string;
}

interface RawLiveConflict {
  binding: string;
  existing_action: string;
  existing_scope: string;
  existing_owner: string;
}

export async function queryLiveConflicts(
  binding: string
): Promise<LiveConflict[]> {
  try {
    const raw = await invoke<RawLiveConflict[]>(
      "keybindings_query_live_conflicts",
      { binding }
    );
    return raw.map((r) => ({
      binding: r.binding,
      existingAction: r.existing_action,
      existingScope: r.existing_scope,
      existingOwner: r.existing_owner,
    }));
  } catch {
    return [];
  }
}

/// Check whether a proposed accelerator is already bound.
///
/// Excludes the given action from the search so that the currently edited
/// row does not conflict with itself. Returns the conflicting action or
/// null if the binding is free.
export function findConflict(
  binding: string,
  excludeAction?: string
): string | null {
  const state = get(inner);
  const match = state.entries.find(
    (e) =>
      e.binding === binding &&
      (!excludeAction || e.action !== excludeAction)
  );
  return match ? match.action : null;
}

// -----------------------------------------------------------------------
// Derived slices for the UI
// -----------------------------------------------------------------------

export const CATEGORIES = [
  { id: "window", label: "Window Management" },
  { id: "focus", label: "Focus" },
  { id: "move", label: "Move Window" },
  { id: "tiling", label: "Tiling" },
  { id: "workspace", label: "Workspaces" },
  { id: "workspace_move", label: "Move to Workspace" },
  { id: "workspace_map", label: "Workspace Map" },
  { id: "keyboard", label: "Keyboard" },
  { id: "shell", label: "Shell" },
  { id: "apps", label: "Apps & Launchers" },
  { id: "custom", label: "Custom" },
  { id: "module", label: "Modules" },
] as const;

/// Internal Workspace Map keybindings — handled entirely by the
/// desktop-shell frontend while the overlay is open. These aren't
/// part of the compositor's keybinding system (no config entry, no
/// conflict detection) so we expose them via the Shortcuts page as
/// read-only reference rows. The frontend hardcodes these values in
/// `WorkspaceIndicator.svelte`'s `onKeydown` — if the UI text here
/// ever disagrees with the handler, the handler is the source of
/// truth.
export interface WorkspaceMapInternalBinding {
  keys: string;
  label: string;
  description?: string;
  group: "navigation" | "actions" | "multi";
}

export const WORKSPACE_MAP_INTERNAL_BINDINGS: WorkspaceMapInternalBinding[] = [
  // Navigation
  { keys: "h / ←", label: "Previous workspace", group: "navigation" },
  { keys: "l / →", label: "Next workspace", group: "navigation" },
  { keys: "k / ↑", label: "Previous window in column", group: "navigation" },
  { keys: "j / ↓", label: "Next window in column", group: "navigation" },
  { keys: "Tab", label: "Next window (cycle)", group: "navigation" },
  { keys: "Shift+Tab", label: "Previous window", group: "navigation" },
  { keys: "1-9", label: "Focus workspace N", group: "navigation" },
  { keys: "g then 1-9", label: "Go to workspace N and close Map", group: "navigation" },
  // Single-window actions
  { keys: "Enter", label: "Activate / restore focused window and close Map", group: "actions" },
  { keys: "Escape", label: "Clear selection, then close Map", group: "actions" },
  { keys: "d / Delete", label: "Close focused window", group: "actions" },
  { keys: "m", label: "Toggle minimize / restore on focused window", group: "actions" },
  { keys: "f", label: "Toggle fullscreen on focused window", group: "actions" },
  { keys: "Space", label: "Toggle selection on focused window", group: "actions" },
  // Multi-select actions
  { keys: "Ctrl/Cmd+Click", label: "Toggle selection", group: "multi" },
  { keys: "d / Delete", label: "Close all selected", group: "multi" },
  { keys: "m", label: "Minimize / restore all selected", group: "multi" },
];

export type CategoryId = (typeof CATEGORIES)[number]["id"];

export const entriesByCategory: Readable<Record<string, KeybindingEntry[]>> =
  derived(keybindings, ($s) => {
    const out: Record<string, KeybindingEntry[]> = {};
    for (const cat of CATEGORIES) {
      out[cat.id] = [];
    }
    for (const e of $s.entries) {
      if (!out[e.category]) out[e.category] = [];
      out[e.category].push(e);
    }
    return out;
  });

/// Number of custom entries (user-added spawn/shell/etc).
export const customCount: Readable<number> = derived(
  keybindings,
  ($s) => $s.entries.filter((e) => e.category === "apps" || e.category === "shell" || e.category === "custom").length
);
