/// The agent's gate/act/undo loop, surfaced in the live chat. The autonomous
/// agent is event-triggered (not tied to a chat turn), so its pending proposals
/// and completed-action receipts are polled here and shown in a tray beside the
/// composer. `executor_live` is flipped; the user must be able to see, approve,
/// and undo what the agent writes to the real graph.
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

/// An agent action awaiting the gate (`pending_proposals`).
export interface PendingProposal {
  /// The audit-ledger index; the handle for approve/deny.
  id: number;
  behaviour: string;
  tool: string;
  /// One line: the proposed action.
  summary: string;
  /// Why the agent wants it (predict-before-act).
  reason: string;
  /// The concrete graph effects ("FILE_PART_OF: system.File -> system.Project").
  effects: string[];
  operands: [string, string][];
  change: ChangeProposal | null;
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

/// The agent's proposals awaiting approval, and the recently applied actions.
/// `null` means COULD NOT READ - a refusal or an unreachable bridge - and it is
/// never collapsed into `[]`: an empty tray says "nothing is pending", and
/// saying that about a denied read is the one sentence this surface must not
/// get wrong (the `ai_access_grants` rule, applied here).
export const pendingProposals = writable<PendingProposal[] | null>([]);
export const completedActions = writable<CompletedAction[] | null>([]);

let seq = 0;

/// Parse one list. Anything that is not a JSON array THROWS instead of
/// becoming `[]`: the daemons answer refusals as errors now, and turning one
/// back into an empty list at the last step is how a denial used to render as
/// "nothing happened". Today the bridge's `call_string` still substitutes
/// `"[]"` before an error can reach us - the moment it signals instead (its
/// own `try_call_string`), this path shows the refusal without another change.
async function fetchList<T>(command: string): Promise<T[]> {
  const raw = await invoke<string>(command);
  const parsed = JSON.parse(raw);
  if (!Array.isArray(parsed)) throw new Error("not a list");
  return parsed as T[];
}

/// Poll both lists once. A monotonic token drops a stale response that lands
/// after a newer one. Failure sets both to null - unread, not empty.
export async function refresh(): Promise<void> {
  const mine = ++seq;
  try {
    const [pending, completed] = await Promise.all([
      fetchList<PendingProposal>("pending_proposals"),
      fetchList<CompletedAction>("completed_actions"),
    ]);
    if (mine !== seq) return;
    pendingProposals.set(pending);
    completedActions.set(completed);
  } catch {
    if (mine !== seq) return;
    if (isTauri()) {
      // A real session whose read failed: unread, not empty.
      pendingProposals.set(null);
      completedActions.set(null);
    } else {
      // Under vite there is no bridge to refuse; the design fixtures come from
      // `_gatetest`, and the live chat's tray simply stays out of the way.
      pendingProposals.set([]);
      completedActions.set([]);
    }
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

/// Approve a proposal (the act), then refresh. Returns the status string.
export async function approveProposal(id: number): Promise<string> {
  const status = await invoke<string>("approve", { id });
  await refresh();
  return status;
}

/// Deny a proposal (drop it), then refresh. Returns the status string.
export async function denyProposal(id: number): Promise<string> {
  const status = await invoke<string>("deny", { id });
  await refresh();
  return status;
}

/// Undo a completed action (the executor compensate), then refresh. Keyed on the
/// completed action's correlation id, matching the drawer/agent-page semantics.
export async function undoAction(id: string): Promise<string> {
  const status = await invoke<string>("undo_action", { id });
  await refresh();
  return status;
}
