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
};

/// The open file, or null when the editor was launched with none.
export const openDocument = writable<OpenDocument | null>(null);

/// Why the file the editor was asked to open is not on screen. Null when there
/// was nothing to open or the open succeeded.
export const openError = writable<string | null>(null);

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

/// Load the launch file, if there is one. Safe to call when no Tauri runtime is
/// present: the browser and the screenshot loop simply have no launch file.
export async function loadInitialFile(): Promise<void> {
  if (!tauriAvailable) return;
  let path: string | null = null;
  try {
    path = await invoke<string | null>("initial_file");
  } catch (e) {
    openError.set(String(e));
    return;
  }
  if (!path) return;
  openTarget.set(path.split("/").pop() || path);
  try {
    const opened = await invoke<{ path: string; text: string }>("editor_open", { path });
    openDocument.set({
      path: opened.path,
      name: opened.path.split("/").pop() || opened.path,
      content: opened.text,
      type: canvasType(opened.path),
    });
    openError.set(null);
  } catch (e) {
    // The host's message names the path and the reason; it is more useful than
    // anything this layer could invent, and it is about a file the user chose.
    openError.set(String(e));
  }
}
