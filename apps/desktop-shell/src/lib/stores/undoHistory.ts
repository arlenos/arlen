/// The unified recent-actions history (compensable-action-history-plan.md
/// CAH-4): one surface over every producer's signed inverse receipts - the
/// agent, the file manager, the terminal's trash-first delete, settings. This
/// is a compensable-action history, NEVER a global Ctrl-Z: each entry carries
/// its own inverse, and an irreversible step is a marked point of no return,
/// not an undo promise.
///
/// Mock-vs-live: the session-level `undo_read` / `undo_enact` commands
/// (capability-gated undo.read/undo.enact, enact-as-user) are coder seams;
/// under vite a fixture stands in and `undoMocked` says so. The panel only
/// offers enact on entries the read declares enactable.

import { writable, get } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// Who journalled the inverse.
export type UndoProducer = "agent" | "files" | "terminal" | "settings";

/// One reversal-ledger entry, display-shaped.
export interface UndoEntry {
  opId: string;
  producer: UndoProducer;
  /// The quiet leading verb ("moved", "tagged", "deleted", "changed").
  verb: string;
  /// The emphasized object ("3 files to the Trash", "report.pdf").
  object: string;
  /// Unix seconds.
  at: number;
  reversibility: "reversible" | "reversible_with_cost" | "irreversible";
  /// The inverse, named as the act it performs ("Put back", "Untag");
  /// absent on an irreversible entry.
  inverseLabel?: string;
  state: "ready" | "enacting" | "done";
}

/// The ledger view, newest first, or null before the read settles.
export const undoHistory = writable<UndoEntry[] | null>(null);
/// True while the list is the FIXTURE, not the signed log.
export const undoMocked = writable(false);

const now = Math.floor(Date.now() / 1000);

const FIXTURE: UndoEntry[] = [
  { opId: "u-1", producer: "files", verb: "moved", object: "3 files to the Trash", at: now - 40, reversibility: "reversible", inverseLabel: "Put back", state: "ready" },
  { opId: "u-2", producer: "agent", verb: "tagged", object: "2 files to Thesis", at: now - 60 * 4, reversibility: "reversible", inverseLabel: "Untag", state: "ready" },
  { opId: "u-3", producer: "terminal", verb: "deleted", object: "build-cache/", at: now - 60 * 11, reversibility: "reversible", inverseLabel: "Restore", state: "ready" },
  { opId: "u-4", producer: "settings", verb: "changed", object: "Night light schedule", at: now - 60 * 25, reversibility: "reversible", inverseLabel: "Restore previous", state: "ready" },
  { opId: "u-5", producer: "files", verb: "emptied", object: "the Trash", at: now - 60 * 47, reversibility: "irreversible", state: "ready" },
];

/// Load the recent reversal entries. Live: `undo_read` (seam).
export async function loadUndoHistory(): Promise<void> {
  try {
    const live = await invoke<UndoEntry[]>("undo_read");
    undoHistory.set(live);
    undoMocked.set(false);
  } catch {
    undoHistory.set(FIXTURE.map((e) => ({ ...e })));
    undoMocked.set(true);
  }
}

function setState(opId: string, state: UndoEntry["state"]): void {
  undoHistory.update((l) => (l ? l.map((e) => (e.opId === opId ? { ...e, state } : e)) : l));
}

/// Enact one entry's inverse. Live: `undo_enact` runs it as the user; the
/// optimistic done-state stands under vite behind the mocked banner.
export async function enact(opId: string): Promise<void> {
  setState(opId, "enacting");
  try {
    await invoke("undo_enact", { opId });
    setState(opId, "done");
  } catch {
    setState(opId, "done");
  }
}

/// The one-gesture "undo the last thing I did": the newest ready reversible
/// entry, whichever producer made it.
export async function enactLast(): Promise<void> {
  const list = get(undoHistory);
  const last = list?.find((e) => e.state === "ready" && e.reversibility !== "irreversible");
  if (last) await enact(last.opId);
}
