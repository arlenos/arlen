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

/**
 * Raise `key` as a toast, for a refusal that did not come from an `invoke`.
 *
 * The clipboard is the case that needed this: a copy that fails is not a host
 * command refusing, so `shellAction` has nothing to wrap, and the popover that
 * offered the button is gone before anybody could read a line in it.
 *
 * It lives HERE rather than in the component for a mechanical reason worth
 * knowing: `check-i18n-reactivity` refuses `get(t)` inside a `.svelte` file,
 * because a string read that way keeps whichever locale rendered first. A toast
 * is not markup - it is a snapshot raised at the moment of the failure, and the
 * current locale is the right one - so the rule and the need meet in a helper.
 */
export function shellToastError(key: string): void {
  toast.error(get(t)(key));
}
