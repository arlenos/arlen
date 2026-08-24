/// The user's ~/Templates entries (`files_templates`), shared between the
/// context menu's "New from template" submenu and the global app menu - one
/// load, two surfaces.
import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// One template file the backend found.
export interface Template {
  label: string;
  icon: string;
  path: string;
}

export const templates = writable<Template[]>([]);

/// Read the list once at startup. Best-effort: no Templates folder, no items.
export async function loadTemplates(): Promise<void> {
  try {
    templates.set(await invoke<Template[]>("files_templates"));
  } catch {
    // No backend (vite) or no folder: the submenu simply stays away.
  }
}
