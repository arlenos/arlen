import { writable, derived } from "svelte/store";

export interface WindowInfo {
  id: string;
  app_id: string;
  title: string;
  focused: boolean;
}

export const windows = writable<WindowInfo[]>([]);

export const focusedWindow = derived(windows, ($windows) =>
  $windows.find((w) => w.focused) ?? null
);

export const activeAppName = derived(focusedWindow, ($focused) => {
  if (!$focused) return "";
  // Use app_id as fallback if title is empty
  return $focused.app_id || $focused.title || "";
});
