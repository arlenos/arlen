/// Chrome-to-surface signals: the titlebar's context-sensitive "+" (the macOS
/// pattern - one fixed add affordance in the chrome) pings the active surface,
/// which owns what "add" means there.
import { writable } from "svelte/store";

/// Bumped by the chrome "+"; the active surface reacts (alarms open the
/// editor dialog, world focuses the city search).
export const addSignal = writable(0);

/// Fire the add affordance.
export function requestAdd(): void {
  addSignal.update((n) => n + 1);
}
