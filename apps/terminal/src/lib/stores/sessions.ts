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

/// True when the last sessions load FAILED. An unreachable backend is
/// not the same as an honestly empty list; the page renders the two
/// differently and auto-create never fires into a dead backend.
export const sessionsError = writable(false);

/// True when the very first successful load found no sessions — a
/// fresh launch. The sidebar starts collapsed then: nothing to
/// switch between, the stream and composer get the room.
export const firstLoadWasEmpty = writable(false);

// One auto-create per app run. The pre-engine stubs answer
// new_session with an unlisted session, so without this guard the
// empty list would re-trigger the create on every reload.
let autoCreateTried = false;

/// Loads the session list. Keeps the active selection when it still
/// exists, otherwise falls back to the first session. On the first
/// successful EMPTY load it opens a session unasked, so a fresh
/// launch lands ready to type.
export async function loadSessions(): Promise<void> {
  let list: Session[];
  try {
    list = await terminalSessions();
  } catch {
    sessionsError.set(true);
    sessionsLoaded.set(true);
    return;
  }
  sessionsError.set(false);
  sessions.set(list);
  const active = get(activeSessionId);
  if (!active || !list.some((s) => s.id === active)) {
    activeSessionId.set(list[0]?.id ?? null);
  }
  sessionsLoaded.set(true);

  if (list.length === 0 && !autoCreateTried) {
    autoCreateTried = true;
    firstLoadWasEmpty.set(true);
    await newSession();
  }
}

/// Opens a new shell and selects it — but only adopts the returned id
/// when the backend actually lists the session afterwards (the
/// pre-engine stubs do not, and a phantom selection would point the
/// stream at nothing).
export async function newSession(): Promise<void> {
  try {
    const s = await terminalNewSession();
    const list = await terminalSessions();
    sessionsError.set(false);
    sessions.set(list);
    if (s.id && list.some((x) => x.id === s.id)) {
      activeSessionId.set(s.id);
    } else if (!get(activeSessionId) && list.length > 0) {
      activeSessionId.set(list[0].id);
    }
  } catch {
    // The backend refused; the list stays as it is.
  }
}

export function selectSession(id: string): void {
  activeSessionId.set(id);
}
