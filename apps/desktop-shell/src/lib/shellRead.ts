/// Read a piece of shell state, and say so in the log when it cannot be read.
///
/// The sibling of `shellAction`, for the other half of the problem. An ACTION
/// that fails has a person waiting for it, so it gets a sentence; a READ that
/// fails usually has nobody waiting and no room to speak - a top-bar badge is
/// about twenty pixels - so it gets a line in the log instead.
///
/// The silence was the problem, not the swallowing. `catch {}` on a poll leaves
/// the last known value, which is the right BEHAVIOUR: a badge that blanks on
/// one failed read would flicker, and inventing a state is worse than keeping
/// one. But it also means a read that has been failing for an hour looks exactly
/// like a state that has not changed - and for the tray, whose empty list HIDES
/// the indicator, a subsystem that cannot be read looks like a machine with
/// nothing in it.
///
/// `log_frontend` rather than `console.error`, because WebKitGTK does not
/// reliably put console output anywhere a diagnostic session can see it - the
/// same reason the workspace overlay's drag logging goes through it.

import { invoke } from "@tauri-apps/api/core";

/// The last failure printed for each reader, so a poll that keeps failing the
/// same way says it once.
const said = new Map<string, string>();

/**
 * Invoke `command` and return its value, or `null` when it could not be read.
 *
 * `tag` names the reader in the log line, so an empty tray and an unchanging
 * layout can be told apart in a session that is full of both.
 */
export async function shellRead<T>(command: string, tag: string): Promise<T | null> {
  try {
    const value = await invoke<T>(command);
    said.delete(tag);
    return value;
  } catch (e) {
    // ONCE per distinct answer, not once per poll. The audio tile polls every
    // four seconds and a machine without `wpctl` fails every one of them: the
    // first run of this wrote thirteen identical lines into a sixty-second boot
    // log, which is how a diagnostic line becomes something people scroll past.
    // A repeat is only worth printing when the ANSWER changed, and a success
    // clears the memory so a subsystem that comes back and goes again says so.
    const message = String(e);
    if (said.get(tag) === message) return null;
    said.set(tag, message);
    invoke("log_frontend", {
      message: `[${tag}] ${command} could not be read: ${message}`,
    }).catch(() => {
      // The line did not get written, so do not remember having written it: the
      // next poll tries again rather than staying quiet about a failure nobody
      // has ever seen. `check-optimistic-write` asked for this and was right -
      // an empty handler here would have left the memory claiming something the
      // log never received.
      said.delete(tag);
    });
    return null;
  }
}
