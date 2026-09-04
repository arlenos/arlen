/// The file the editor was launched on.
///
/// `arlen-text-editor <path>`, or a `.desktop` `Exec=<bin> %f` from the file
/// manager, reaches the frontend through the host's `initial_file`; the contents
/// come back from `editor_open`. Launched bare, this stays null and the page
/// falls back to its two demo documents, which describe the editor itself and
/// claim nothing about the machine.
///
/// There is no fixture on the failure path. A file the editor was asked to open
/// and could not read is reported with the reason the host gave - a missing file,
/// a permission error, a binary file this editor will not open - because the one
/// thing it must never do is show made-up text under a real filename.
import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { tauriAvailable } from "$lib/tauri";

/// A file read from disk, ready to render.
export type OpenDocument = {
  /// The absolute path, as the host will want it back on save.
  path: string;
  /// The basename, for the titlebar.
  name: string;
  /// The contents.
  content: string;
  /// Which canvas treatment: markdown prose or a whole highlighted file.
  type: "markdown" | "code";
  /// What the file looked like when it was read, handed back at save so a write
  /// that would clobber somebody else's change is refused instead.
  stamp: string;
};

/// The open file, or null when the editor was launched with none.
export const openDocument = writable<OpenDocument | null>(null);

/// Why the file the editor was asked to open is not on screen. Null when there
/// was nothing to open or the open succeeded.
export type OpenProblem =
  | { problem: "not-absolute" }
  | { problem: "unreadable"; why: string }
  | { problem: "not-text" }
  | { problem: "other" };

export const openError = writable<OpenProblem | null>(null);

/// Why a save did not happen, as the host now names it (`SaveProblem`).
///
/// `file-changed-on-disk` is not in this union on purpose: the page treats it as
/// a question with an answer rather than a failure, and it has its own state.
export type SaveProblem =
  | { problem: "not-absolute" }
  | { problem: "no-parent" }
  | { problem: "unwritable"; why: string }
  | { problem: "other" };

/// The message key for a refused save.
///
/// The reason used to be `String(e)`, which the host built as `"{path}: {e}"` -
/// so a read-only file put "/home/tim/notes.md: Permission denied (os error 13)"
/// inside a translated sentence. The path is already on screen in the titlebar
/// and the errno is for the log.
export function saveProblemKey(e: unknown): string {
  const bag = problemBag(e);
  switch (bag?.problem) {
    case "not-absolute":
      return "te.save.notAbsolute";
    case "no-parent":
      return "te.save.noParent";
    // EVERYTHING FOR THIS WAS BUILT EXCEPT THIS LINE. The host has a
    // `ChangedOnDisk` variant, it carries the `file-changed-on-disk` tag, and
    // `te.save.changedOnDisk` has said the right sentence in both languages the
    // whole time - the Rust doc even states that "the page's existing branch on
    // that word keeps working". There was no such branch. So a save refused
    // because somebody else had written the file fell through to `other`,
    // "Could not save. The detail is in the log.", and the one sentence written
    // for exactly this moment could not be reached. The `console.warn` below
    // fired every time, into the one place nobody reads.
    case "file-changed-on-disk":
      return "te.save.changedOnDisk";
    case "unwritable":
      return "te.save.unwritable";
    default:
      // A tag this does not know is a host that changed. The console is where
      // that belongs; the page says the vague true thing.
      if (bag?.problem) console.warn("text-editor: unrecognised save problem", bag.problem);
      return "te.save.other";
  }
}

/// Read the host's answer, which arrives as an object here and as a string with
/// JSON inside it elsewhere in the tree. Both are accepted rather than one
/// assumed: guessing wrong sends every named cause down `other`, which looks
/// exactly like the code working.
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

function named(e: unknown): OpenProblem {
  const bag = problemBag(e);
  if (bag?.problem === "not-absolute") return { problem: "not-absolute" };
  if (bag?.problem === "unreadable")
    return { problem: "unreadable", why: String(bag.why ?? "") };
  if (bag?.problem === "not-text") return { problem: "not-text" };
  // No reason carried. The page drew this field bare - `{$openError.reason}` -
  // so whatever the host formatted was the whole detail line under "Diese Datei
  // konnte nicht geöffnet werden", in every language. The three named problems
  // above have their own sentences; this one gets the general one and the host's
  // words go to the log.
  console.warn("text-editor: that file could not be opened", e);
  return { problem: "other" };
}

/// The basename of the file the editor was launched on, whether or not it opened.
/// The titlebar needs this: on a failed open the first version fell back to a demo
/// document's name, so the window said `the-kg-lens.md` above a pane that said the
/// file could not be opened - a filename over content that is not its content, the
/// same defect as a fixture, reached from the other side.
export const openTarget = writable<string | null>(null);

/// Markdown gets the prose treatment, everything else the whole-file gutter.
function canvasType(name: string): "markdown" | "code" {
  return /\.(md|markdown|mdown|mkd)$/i.test(name) ? "markdown" : "code";
}

/// Open one file by path, replacing whatever is open.
///
/// Shared by the launch path and the lens's related-file links, so a file opened
/// either way lands in the same place and fails the same way. A failure sets the
/// error rather than throwing: both callers are user gestures, and the surface
/// already renders `openError` where the text would be.
export async function openPath(path: string): Promise<void> {
  if (!tauriAvailable) return;
  openTarget.set(path.split("/").pop() || path);
  try {
    const opened = await invoke<{ path: string; text: string; stamp: string }>(
      "editor_open",
      { path },
    );
    openDocument.set({
      path: opened.path,
      name: opened.path.split("/").pop() || opened.path,
      content: opened.text,
      type: canvasType(opened.path),
      stamp: opened.stamp,
    });
    openError.set(null);
  } catch (e) {
    openError.set(named(e));
  }
}

/// Load the launch file, if there is one. Safe to call when no Tauri runtime is
/// present: the browser and the screenshot loop simply have no launch file.
export async function loadInitialFile(): Promise<void> {
  if (!tauriAvailable) return;
  let path: string | null = null;
  try {
    path = await invoke<string | null>("initial_file");
  } catch (e) {
    openError.set(named(e));
    return;
  }
  if (!path) return;
  // The host's message names the path and the reason, which is more useful than
  // anything this layer could invent and is about a file the user chose.
  await openPath(path);
}
