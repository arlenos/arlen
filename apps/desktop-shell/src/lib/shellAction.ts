/// Run a shell control's action and say something if it does not take.
///
/// Every tile swallowed its own click - `try { invoke(...) } catch {}` in eight
/// files - so the grid was a page of controls that could each do nothing without
/// a word. A tile is about sixty pixels and has no room for a line, and the panel
/// closes the moment you click outside it, so the message has to outlive the
/// surface that raised it.
///
/// The toast is that channel, and it is the one the quick ACTIONS already use
/// from the backend (`quick_action_run` emits `arlen://toast`). This is the same
/// notice from the frontend side rather than a second invention.
///
/// Each caller passes its own message key, because the person clicked a named
/// thing: "Bluetooth did not change" is worth more than "that did not work".
///
/// It was `quicksettings/action.ts` for about ten minutes, until the top-bar
/// badges turned out to need exactly the same thing - a small control, no room
/// for a line, on a surface that may be gone before the answer arrives. A helper
/// named after the first caller lies to the second.

import { invoke } from "@tauri-apps/api/core";
import { get } from "svelte/store";
import { toast } from "svelte-sonner";
import { t } from "$lib/i18n/messages";

/**
 * Invoke `command`, and on refusal raise `failureKey` as a toast.
 *
 * Returns whether it took, so a caller that applied something optimistically can
 * put it back. Never throws: these run in event callbacks, and an unhandled
 * rejection there goes exactly where this exists to stop it going.
 */
export async function shellAction(
  command: string,
  args: Record<string, unknown>,
  failureKey: string,
): Promise<boolean> {
  try {
    await invoke(command, args);
    return true;
  } catch {
    toast.error(get(t)(failureKey));
    return false;
  }
}
