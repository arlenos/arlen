/// The operations layer: clipboard, the running-operation state and
/// the conflict hand-off. Every mutation goes through `runOp`, which
/// refreshes the active tab on success, surfaces a conflict dialog on
/// a name collision, and reports anything else as a lay-readable
/// error line.

import { get, writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { activeController } from "$lib/stores/tabs";

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
export const opBusy = writable<string | null>(null);

/// The last failed operation, as one plain sentence. Cleared by the
/// next successful operation.
export const opError = writable<string | null>(null);

/// A pending name conflict: the dialog offers skip / keep both /
/// replace and re-runs the operation with the chosen policy.
export const conflict = writable<{
  name: string;
  retry: (policy: "skip" | "rename" | "replace") => void;
} | null>(null);

/// Lay-readable label for the progress surface.
function busyLabel(kind: OpKind, count: number): string {
  const things = count === 1 ? "1 item" : `${count} items`;
  switch (kind) {
    case "copy":
      return `Copying ${things}`;
    case "move":
      return `Moving ${things}`;
    case "trash":
      return `Moving ${things} to the trash`;
    case "delete":
      return `Deleting ${things}`;
    case "duplicate":
      return `Duplicating ${things}`;
    case "rename":
      return "Renaming";
    case "new_folder":
      return "Creating the folder";
  }
}

export async function runOp(
  kind: OpKind,
  src: string[],
  dst?: string,
  policy?: "skip" | "rename" | "replace",
): Promise<boolean> {
  opBusy.set(busyLabel(kind, src.length));
  try {
    await invoke("files_op", { kind, src, dst: dst ?? null, policy: policy ?? null });
    opError.set(null);
    await get(activeController)?.refresh();
    return true;
  } catch (e) {
    const message = String(e);
    const exists = message.match(/already exists(?::\s*(.+))?/);
    if (exists && !policy) {
      conflict.set({
        name: exists[1] ?? src.map((s) => s.split("/").pop()).join(", "),
        retry: (chosen) => {
          conflict.set(null);
          void runOp(kind, src, dst, chosen);
        },
      });
    } else {
      opError.set(message);
    }
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
