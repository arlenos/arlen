/// What the agent did on its own, surfaced in the live chat. The agent is
/// event-triggered (not tied to a chat turn), so its completed-action receipts
/// are polled here and shown in a tray beside the composer, each with an undo.
///
/// Receipts only. A proposal that needs a yes is asked through the consent
/// path, which draws its own prompt; an approve button in an app window is the
/// shape that design closed, so this store carries no pending list.
///
/// Svelte-5 caveat: IPC results ride `writable` stores, never `$state` mutated
/// from a callback (that does not re-render reliably).

import { writable } from "svelte/store";
import { invoke, isTauri } from "@tauri-apps/api/core";

/// A structured change carried by a proposal / receipt (a file move, an edit).
/// Absent (null) for graph writes, which have no file body.
export interface ChangeProposal {
  kind?: string;
  summary?: string;
  from?: string;
  to?: string;
  /// A unified diff, when the change is a content edit.
  diff?: string;
}

/// A completed (applied) action (`completed_actions`); the undo target.
export interface CompletedAction {
  /// The decision's correlation id; the handle for undo.
  id: string;
  behaviour: string;
  /// One line: what was done.
  what: string;
  change: ChangeProposal | null;
}

/// The recently applied actions. `null` means COULD NOT READ - a refusal or an
/// unreachable bridge - and it is never collapsed into `[]`: an empty tray says
/// "nothing happened", and saying that about a denied read is the one sentence
/// this surface must not get wrong.
export const completedActions = writable<CompletedAction[] | null>([]);

let seq = 0;

/// Poll the list once. A monotonic token drops a stale response that lands
/// after a newer one. Anything that is not a JSON array is a failed read, and
/// failure sets the list to null - unread, not empty.
export async function refresh(): Promise<void> {
  const mine = ++seq;
  try {
    const parsed = JSON.parse(await invoke<string>("completed_actions"));
    if (!Array.isArray(parsed)) throw new Error("not a list");
    if (mine !== seq) return;
    completedActions.set(parsed as CompletedAction[]);
  } catch {
    if (mine !== seq) return;
    // A real session whose read failed is unread. Under vite there is no
    // bridge to refuse; the fixtures come from `_gatetest`, and the live
    // chat's tray simply stays out of the way.
    completedActions.set(isTauri() ? null : []);
  }
}

/// Start the background poll (only while the window is visible). Returns a
/// cleanup to clear the interval.
export function startPoll(intervalMs = 6000): () => void {
  void refresh();
  const timer = setInterval(() => {
    if (!document.hidden) void refresh();
  }, intervalMs);
  return () => clearInterval(timer);
}

/// Undo a completed action (the executor compensate), then refresh. Keyed on the
/// completed action's correlation id, matching the drawer/agent-page semantics.
export async function undoAction(id: string): Promise<string> {
  const status = await invoke<string>("undo_action", { id });
  await refresh();
  return status;
}
