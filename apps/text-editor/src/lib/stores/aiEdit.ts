/// The sovereign gated AI-edit (text-editor-app.md #1): the reason to build the
/// editor. The assistant is a DISTINCT, capability-scoped principal (never the
/// user); it proposes edits as per-hunk diffs, never silent. The three-tier gate
/// sits BEFORE execution: reversible edits auto-apply (autonomous, with undo),
/// impactful-but-recoverable notify-and-allow-undo, irreversible/external HARD-
/// confirm. You SEE which hunks it took on its own versus which it holds for you.
///
/// Mock-vs-live: fixture-backed. The real path (`ai_edit` -> ACT-layer proxy ->
/// gate via the gate-class registry -> execute -> compensation-store -> HMAC audit,
/// + per-hunk apply/undo) is a coder seam behind pi's executor-live; under vite the
/// store serves a fixture proposal.

import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { tauriAvailable } from "$lib/tauri";
import type { DiffLine } from "@arlen/ui-kit/components/diff";

/// The gate class of a hunk: reversible-autonomous / applied-with-undo / held-for-
/// confirm. Reversibility gates autonomy, not impact.
export type GateClass = "auto" | "notify" | "confirm";
export type HunkStatus = "applied" | "pending" | "rejected" | "undone";

/// One proposed hunk, with its gate class + the plain-terms reason for it.
export interface EditHunk {
  header: string;
  lines: DiffLine[];
  gate: GateClass;
  rationale: string;
  status: HunkStatus;
}

/// The assistant's proposed edit to a file.
export interface AiEditProposal {
  principal: string;
  scope: string;
  prompt: string;
  file: string;
  hunks: EditHunk[];
}

const FIXTURE_PROMPT = "Tighten the intro and add a reference";

function hunk(
  gate: GateClass,
  header: string,
  rationale: string,
  lines: [DiffLine["kind"], string][],
): EditHunk {
  return {
    header,
    gate,
    rationale,
    status: gate === "confirm" ? "pending" : "applied",
    lines: lines.map(([kind, text]) => ({ kind, text })),
  };
}

const FIXTURE: AiEditProposal = {
  principal: "The assistant",
  scope: "Edit this file",
  prompt: FIXTURE_PROMPT,
  file: "the-kg-lens.md",
  hunks: [
    hunk("auto", "Intro paragraph", "Reworded text, fully reversible. Applied on its own; you can undo it.", [
      ["context", "This file is a "],
      ["del", "first-class citizen of the knowledge graph."],
      ["add", "first-class citizen of the graph, and the assistant is a bounded principal that can edit it."],
    ]),
    hunk("notify", "Focus mode", "A larger rewrite, still recoverable. Applied, with undo.", [
      ["del", "Turn this on and every paragraph but the one you are in fades away."],
      ["add", "Focus mode fades every paragraph but the one you are in, so the writing is all that is left."],
    ]),
    hunk("confirm", "New reference", "Adds a link to an external site. Held for your confirmation.", [
      ["context", "principal that can edit this file."],
      ["add", "See also the [design notes](https://example.com/notes)."],
    ]),
  ],
};

/// The proposal on screen, or null.
export const proposal = writable<AiEditProposal | null>(null);

/// True while the proposal on screen is the FIXTURE rather than a real one from
/// the assistant. It names a principal, a scope and concrete diff hunks against
/// the open file, so unlabelled it reads as a real pending edit to accept.
export const mocked = writable(false);

/// The last action failure, for the review to show. Empty when all is well.
export const lastError = writable("");

/// Ask the assistant to edit. Live: `ai_edit`; fixture under vite.
export async function proposeEdit(prompt: string): Promise<void> {
  lastError.set("");
  try {
    proposal.set(await invoke<AiEditProposal>("ai_edit", { prompt }));
    mocked.set(false);
  } catch {
    proposal.set({ ...FIXTURE, prompt: prompt || FIXTURE_PROMPT });
    mocked.set(true);
  }
}

function setStatus(index: number, status: HunkStatus): void {
  proposal.update((p) =>
    p ? { ...p, hunks: p.hunks.map((h, i) => (i === index ? { ...h, status } : h)) } : p,
  );
}

/// Drive one hunk action optimistically, then reconcile with the backend.
///
/// A REAL refusal restores the previous status and says so. Swallowing it (as
/// this did) makes the review lie about the file: an "applied" hunk that was
/// never written, a "rejected" one the backend never held back, or - worst - an
/// "undone" one whose edit is still in the file, which would falsify the
/// reversibility the whole gated-edit model rests on. Without the Tauri runtime
/// there is no backend to refuse, so the optimistic mock stands.
async function driveHunk(
  index: number,
  next: HunkStatus,
  cmd: string,
  failure: string,
): Promise<void> {
  let previous: HunkStatus | undefined;
  proposal.update((p) => {
    previous = p?.hunks[index]?.status;
    return p;
  });
  setStatus(index, next);
  try {
    await invoke(cmd, { index });
  } catch (e) {
    if (tauriAvailable) {
      if (previous) setStatus(index, previous);
      lastError.set(`${failure}: ${String(e)}`);
    }
  }
}

/// Confirm a held (confirm-class) hunk. Live: `ai_edit_accept`.
export async function acceptHunk(index: number): Promise<void> {
  await driveHunk(index, "applied", "ai_edit_accept", "Could not apply that change");
}
/// Reject a held hunk. Live: `ai_edit_reject`.
export async function rejectHunk(index: number): Promise<void> {
  await driveHunk(index, "rejected", "ai_edit_reject", "Could not reject that change");
}
/// Undo an applied hunk (the compensation). Live: `ai_edit_undo`.
export async function undoHunk(index: number): Promise<void> {
  await driveHunk(index, "undone", "ai_edit_undo", "Could not undo that change - it is still in the file");
}
/// Dismiss the whole review.
export function dismiss(): void {
  proposal.set(null);
}
