/// The operations layer: clipboard, the running-operation state and
/// the conflict hand-off. Every mutation goes through `runOp`, which
/// refreshes the active tab on success, surfaces a conflict dialog on
/// a name collision, and reports anything else as a lay-readable
/// error line.

import { get, writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { activeController } from "$lib/stores/tabs";
import { type RenameRule } from "$lib/bulk-rename";

export type OpKind =
  | "copy"
  | "move"
  | "rename"
  | "trash"
  | "delete"
  | "duplicate"
  | "new_folder";

/// Cut/copied paths waiting for paste.
export const clipboard = writable<{ kind: "copy" | "move"; paths: string[] } | null>(null);

/// The label of the operation in flight (the progress surface), or null.
/// A line the surface shows, as the catalogue key and its values rather than a
/// resolved sentence.
///
/// The TYPE is the point. Both of these stores held a `string`, and a string is
/// what `String(e)` and `` `Renaming ${n} items` `` both are - so seven writers
/// of one and four of the other put the host's words, or English written in this
/// file, straight onto a surface. Keeping the pair unresolved refuses that at
/// every writer, and lets a line already on screen follow a locale change.
export type OpMessage = { key: string; values?: Record<string, unknown> };

export const opBusy = writable<OpMessage | null>(null);

/// The message KEY for the last failed operation, cleared by the next successful
/// one. A key rather than a sentence: the overlay renders `{$opError}` and used
/// to render whatever Rust formatted, so a German window carried an English
/// clause or a bare errno in a red bar.
export const opError = writable<OpMessage | null>(null);

/// Read the host's answer, which arrives as an object on one path and as a
/// string with the JSON inside it on another. Accepting only one sends every
/// named cause down the vague branch, which looks exactly like the code working.
export function problemBag(e: unknown): Record<string, unknown> | null {
  if (e && typeof e === "object") return e as Record<string, unknown>;
  const raw = String(e);
  const at = raw.indexOf("{");
  try {
    return at >= 0 ? (JSON.parse(raw.slice(at)) as Record<string, unknown>) : null;
  } catch {
    return null;
  }
}

/// The sentence for a refused operation.
///
/// `bad-request` is a malformed call rather than something the person did, so it
/// gets the vague sentence and the detail goes to the console: naming a missing
/// destination argument to somebody who pressed Rename explains nothing.
export function opProblemKey(e: unknown): string {
  const bag = problemBag(e);
  switch (bag?.problem) {
    case "already-exists":
      return "f.op.exists";
    case "invalid-name":
      return "f.op.badName";
    case "partial":
      return "f.op.partial";
    case "io":
      return "f.op.refused";
    case "bad-request":
      console.warn("files: the operation was asked for wrongly", bag.why);
      return "f.op.failed";
    default:
      if (bag?.problem) console.warn("files: unrecognised operation problem", bag.problem);
      return "f.op.failed";
  }
}

/// A pending name conflict: the dialog offers skip / keep both /
/// replace and re-runs the operation with the chosen policy.
export const conflict = writable<{
  name: string;
  retry: (policy: "skip" | "rename" | "replace") => void;
} | null>(null);

/// Lay-readable label for the progress surface.
///
/// One message per operation rather than a verb joined to a count phrase. The old
/// shape built "Copying " + ("1 item" | "N items"), which is two English
/// assumptions at once: that the sentence can be cut after the verb, and that a
/// plural splits at one. Each message now carries its own plural selector, so the
/// language decides both.
function busyLabel(kind: OpKind, count: number): OpMessage {
  switch (kind) {
    case "copy":
      return { key: "f.op.copying", values: { n: count } };
    case "move":
      return { key: "f.op.moving", values: { n: count } };
    case "trash":
      return { key: "f.op.trashing", values: { n: count } };
    case "delete":
      return { key: "f.op.deleting", values: { n: count } };
    case "duplicate":
      return { key: "f.op.duplicating", values: { n: count } };
    case "rename":
      return { key: "f.op.renaming" };
    case "new_folder":
      return { key: "f.op.newFolder" };
  }
}

/// What the last operation DID, for the status bar, or null.
///
/// A successful trash left no trace: the row vanished from the list, which is
/// feedback of a kind, and nothing said where the file went or that the app can
/// put it back. The undo is real - `files_undo` inverts the last op - and lived
/// entirely in a keyboard shortcut nobody is told about.
///
/// Only the two operations that MOVE somebody's data out of sight say anything.
/// A copy or a rename is visible in the list it just changed; a trash and a
/// delete are the ones where a person wants to know what happened, and they get
/// different sentences because only one of them can be taken back.
export const opDone = writable<{ key: string; count: number } | null>(null);

export async function runOp(
  kind: OpKind,
  src: string[],
  dst?: string,
  policy?: "skip" | "rename" | "replace",
): Promise<boolean> {
  opBusy.set(busyLabel(kind, src.length));
  // The previous answer goes the moment a new one is asked for; a line about the
  // last delete sitting over a running copy is worse than no line.
  opDone.set(null);
  try {
    await invoke("files_op", { kind, src, dst: dst ?? null, policy: policy ?? null });
    opError.set(null);
    if (kind === "trash") opDone.set({ key: "f.done.trash", count: src.length });
    else if (kind === "delete") opDone.set({ key: "f.done.delete", count: src.length });
    else opDone.set(null);
    await get(activeController)?.refresh();
    return true;
  } catch (e) {
    // The TAG, not the sentence. This used to read
    // `String(e).match(/already exists/)`, so the Replace/Skip dialog - a choice,
    // not a message - hung on the exact English wording of a Rust error. Rewording
    // it, or translating it, would have silently turned the choice into a red bar.
    const bag = problemBag(e);
    if (bag?.problem === "already-exists" && !policy) {
      conflict.set({
        name: String(bag.name ?? "") || src.map((s) => s.split("/").pop()).join(", "),
        retry: (chosen) => {
          conflict.set(null);
          void runOp(kind, src, dst, chosen);
        },
      });
    } else {
      opError.set({ key: opProblemKey(e) });
    }
    return false;
  } finally {
    opBusy.set(null);
  }
}

/// Apply a bulk rename to `names` in `dir` under `rule`. The backend recomputes
/// the plan over the same core the preview mirrors and renames safely (ordering
/// + intermediate collisions are its concern), then this refreshes the active
/// view. The `files_bulk_rename` command is the backend half.
export async function bulkRename(
  dir: string,
  names: string[],
  rule: RenameRule,
): Promise<boolean> {
  opBusy.set({ key: "f.op.bulkRenaming", values: { n: names.length } });
  try {
    await invoke("files_bulk_rename", { dir, names, rule });
    opError.set(null);
    await get(activeController)?.refresh();
    return true;
  } catch (e) {
    console.warn("files: bulk rename refused", e);
    opError.set({ key: "f.op.renameFailed" });
    return false;
  } finally {
    opBusy.set(null);
  }
}

/// Extract an archive into `dest`, surfacing progress and errors like every
/// other mutation, then refreshing so the extracted contents show.
export async function extractArchive(archive: string, dest: string): Promise<boolean> {
  opBusy.set({ key: "f.op.extracting" });
  try {
    await invoke("files_extract", { archive, dest });
    opError.set(null);
    await get(activeController)?.refresh();
    return true;
  } catch (e) {
    console.warn("files: extract refused", e);
    opError.set({ key: "f.op.extractFailed" });
    return false;
  } finally {
    opBusy.set(null);
  }
}

/// Compress `sources` into a new archive at `dest`, then refresh.
export async function compressPaths(sources: string[], dest: string): Promise<boolean> {
  opBusy.set({ key: "f.op.compressing", values: { n: sources.length } });
  try {
    await invoke("files_compress", { sources, dest });
    opError.set(null);
    await get(activeController)?.refresh();
    return true;
  } catch (e) {
    console.warn("files: compress refused", e);
    opError.set({ key: "f.op.compressFailed" });
    return false;
  } finally {
    opBusy.set(null);
  }
}

/// Paste the clipboard into `dest`; a cut clipboard empties itself
/// after the move (paste-again would find nothing there).
export async function paste(dest: string): Promise<void> {
  const clip = get(clipboard);
  if (!clip) return;
  const ok = await runOp(clip.kind, clip.paths, dest);
  if (ok && clip.kind === "move") clipboard.set(null);
}

/// Undo the last reversible file operation (the `files_undo` / `UndoStack`
/// backend), refreshing the active tab so the reverted state shows. A `false`
/// result means the undo stack was empty (a no-op, not an error); a thrown
/// error is surfaced on the op-error line. Bound to Ctrl+Z.
export async function undoLast(): Promise<void> {
  opBusy.set({ key: "f.op.undoing" });
  try {
    await invoke<boolean>("files_undo");
    opError.set(null);
    // AND THE LINE THAT DESCRIBED WHAT WAS JUST UNDONE. `runOp` clears `opDone`
    // when the NEXT operation starts, which is not the same as clearing it when
    // this one is reverted: after Ctrl+Z the file is back in the list and the
    // status line still read "Moved to Trash. Ctrl+Z puts it back." - a sentence
    // about a state that no longer holds, offering an action that no longer
    // applies. Only on success: if the undo failed, the file IS still in the
    // trash and that line is still the truth, with `opError` beside it.
    opDone.set(null);
    await get(activeController)?.refresh();
  } catch (e) {
    console.warn("files: undo refused", e);
    opError.set({ key: "f.op.undoFailed" });
  } finally {
    opBusy.set(null);
  }
}
