/// How much is in the trash, on the launcher's own row for it.
///
/// `shell.shortcuts` has a second half nothing in the tree used: `setState`
/// updates ONE registered entry's badge or enabled flag without re-sending the
/// list, and the shell honours both - `app_shortcuts.rs` skips a disabled
/// shortcut and folds a badge into the row's subtitle. The census carried it as
/// waiting for "a producer that has something to diff on its list". The file
/// manager's Trash row is one: the count changes without the list changing, which
/// is exactly the shape `register` cannot express.
///
/// THE COUNT COMES THROUGH THE ONE DOOR. `files_trash_list` is deliberately not a
/// command - a frontend calling it directly gets a failed trash read as a thrown
/// promise and, one catch later, an empty Trash. So this reads
/// `files_list_location("trash")` like the pane does, and gets a `ReadOutcome`
/// that can say it could not look.
///
/// WHICH MATTERS MORE HERE THAN IN THE PANE. A trash we could not read must not
/// publish a badge, and must not clear one either: both would say "the trash is
/// empty" over a question nobody answered, and clearing is the worse of the two
/// because it looks like news. So a non-`rows` outcome leaves whatever the row
/// is already wearing.

import { invoke } from "@tauri-apps/api/core";
import { shortcuts } from "@arlen/tauri-plugin-shell";
import type { FileEntry } from "@arlen/ui-kit/components/browser";
import type { ReadOutcome } from "$lib/read-outcome";

/// The action id the badge belongs to. The same string `appShortcuts` registers,
/// because `setState` addresses an entry by its action and silently does nothing
/// on a miss - so a typo here is a badge that never appears and never complains.
export const TRASH_ACTION = "go.trash";

/// The badge text for a trash holding `count` entries.
///
/// Zero is the empty string, which is what the wire uses to CLEAR a badge rather
/// than draw a nought: an empty trash is the absence of something waiting, not a
/// thing waiting zero times. Same rule as mail's unread badge, and the same
/// treatment for a count that is not a whole number at or above zero - it comes
/// from a listing's length, so it cannot be either today.
export function trashBadge(count: number): string {
  if (!Number.isFinite(count) || count <= 0) return "";
  return String(Math.floor(count));
}

/// Read the trash and put the count on the launcher's Trash row.
///
/// Best-effort against the shell, like the menu and the list registration: under
/// vite there is no relay and the call rejects, and a window whose launcher row
/// has no number is still a window. Not best-effort about the READ - see the
/// header: a trash that could not be read publishes nothing at all.
export async function publishTrashBadge(): Promise<void> {
  let outcome: ReadOutcome<FileEntry>;
  try {
    outcome = await invoke<ReadOutcome<FileEntry>>("files_list_location", {
      location: "trash",
    });
  } catch {
    return; // Could not ask. Leave the row as it is.
  }
  if (outcome.state !== "rows") return;
  try {
    await shortcuts.setState(TRASH_ACTION, { badge: trashBadge(outcome.rows.length) });
  } catch {
    // No shell relay: the launcher row carries no count.
  }
}
