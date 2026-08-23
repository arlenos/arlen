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
    raiseRefusal(failureKey);
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
  raiseRefusal(key);
}

/**
 * Raise a refusal where it will actually be read.
 *
 * A notice drawn into the window that is closing is a notice nobody sees. The
 * launcher hides the instant an item is picked, and a popover closes on the
 * click that failed, so the surface that raised the line is often the one on its
 * way out. Measured on the machine: a quick action refused because the daemon it
 * needs is not on the image logged the refusal, raised its toast in the
 * launcher, and showed the person an empty desktop.
 *
 * `arlen://toast` is the channel that already exists for this. The backend uses
 * it because it cannot know which window is up; the same is true of any frontend
 * code that runs in the launcher. The main window's bridge renders it, and a
 * window that is hidden rendering it too costs nothing.
 *
 * Falls back to a local toast when there is no host to carry the event - a plain
 * vite session, the render harness - because saying it in the wrong window still
 * beats not saying it.
 */
export function raiseRefusal(key: string, params?: Record<string, string>): void {
  void (async () => {
    try {
      const { emit } = await import("@tauri-apps/api/event");
      await emit("arlen://toast", {
        kind: "error",
        key,
        params,
        // Read only if the catalog is missing the id, and legible when it is.
        message: key,
      });
    } catch {
      toast.error(get(t)(key, params));
    }
  })();
}
