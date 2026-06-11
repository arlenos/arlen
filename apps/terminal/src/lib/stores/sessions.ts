/// Session state for the console shell. IPC results land in writable
/// stores (the project-documented Svelte 5 pattern: `$state` mutated
/// from Tauri callbacks does not re-render reliably).

import { writable, get } from "svelte/store";
import {
  terminalSessions,
  terminalNewSession,
  type Session,
} from "$lib/contract";

/// All known sessions, in backend order.
export const sessions = writable<Session[]>([]);

/// The id of the session whose blocks the stream shows; null before
/// the first load (or when no session exists).
export const activeSessionId = writable<string | null>(null);

/// True once the first sessions load answered — before that the UI
/// shows nothing instead of claiming "no sessions".
export const sessionsLoaded = writable(false);

/// Loads the session list. Keeps the active selection when it still
/// exists, otherwise falls back to the first session.
export async function loadSessions(): Promise<void> {
  try {
    const list = await terminalSessions();
    sessions.set(list);
    const active = get(activeSessionId);
    if (!active || !list.some((s) => s.id === active)) {
      activeSessionId.set(list[0]?.id ?? null);
    }
  } catch {
    // Unreachable backend: keep whatever we had; the stream renders
    // its own empty state.
  }
  sessionsLoaded.set(true);
}

/// Opens a new shell and selects it. Safe to call repeatedly — every
/// call is its own session by contract.
export async function newSession(): Promise<void> {
  try {
    const s = await terminalNewSession();
    await loadSessions();
    if (s.id) activeSessionId.set(s.id);
  } catch {
    // The backend refused; the list stays as it is.
  }
}

export function selectSession(id: string): void {
  activeSessionId.set(id);
}
