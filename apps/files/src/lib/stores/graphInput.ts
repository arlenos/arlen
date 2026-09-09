/// What this window tells the knowledge graph about itself.
///
/// NOT in `topbar.ts`, which is the other thing this app publishes, and the
/// difference matters: the toolbar goes to the SHELL and is gated on
/// `shell_present`, because a breadcrumb with nobody drawing it is pointless.
/// These go to the knowledge daemon over the bus, which is running whether or
/// not a desktop shell is. Gating them on the shell would have made the graph's
/// input depend on something unrelated to it.
///
/// PRESENCE is where the person is: the folder in front of them, republished as
/// they navigate and cleared when the window loses focus. TIMELINE is what they
/// finished: a copy, a move, a trash or a delete that the backend confirmed.
///
/// WHY AN APP SAYS THIS AT ALL. The sensor sees the syscalls a copy makes. It
/// cannot see that they were ONE copy, that a person asked for it, how many
/// files it was, or that the two hundred `unlink`s at 14:03 were a deliberate
/// clear-out rather than something's cache expiring. That is the difference
/// between a trace and a history.
///
/// Paths only, and paths are already what the sensor records for every open. No
/// file's contents are read here, let alone published.

import { presence, timeline, type PresenceParams, type TimelineParams } from "@arlen/tauri-plugin-shell";
import { get } from "svelte/store";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { focusedController } from "$lib/stores/panes";
import type { OpKind } from "./ops";

/// The presence for a folder in front of somebody, or null when there is none.
export function browsingPresence(dir: string | null): PresenceParams | null {
  if (!dir) return null;
  return { activity: "browsing", subject: dir, auto_clear: "on-blur" };
}

/// The timeline record for an operation the backend confirmed.
///
/// The SUBJECT is where the result ENDED UP: the destination when the operation
/// has one (copy, move, rename, duplicate, new folder) and the source when it
/// does not (trash, delete). That is the place a person would look for it
/// afterwards, which is what a subject is for - and stating the rule this way
/// rather than listing the kinds means a new `OpKind` gets a sensible record
/// instead of a null.
///
/// With several files it names the first and says how many. That is an
/// approximation and it is the right one: the honest alternative is a record per
/// file, which turns one thing somebody did into two hundred rows.
///
/// The label is a TOKEN and not a sentence, for the reason the editor's save
/// record gives: a record is written once and read later, possibly by a surface
/// in another language, so the wording belongs to whoever draws it.
export function opRecord(kind: OpKind, src: string[], dst?: string): TimelineParams | null {
  const subject = dst ?? src[0];
  if (!subject) return null;
  return {
    label: kind,
    subject,
    type: kind,
    metadata: { count: String(src.length) },
  };
}

/// Publish the presence for the folder in front of somebody, or take the last
/// one down. Best-effort: under vite there is no relay and the call rejects.
export async function publishPresence(dir: string | null): Promise<void> {
  const params = browsingPresence(dir);
  try {
    if (params) await presence.set(params);
    else await presence.clear();
  } catch {
    // No relay: the graph hears nothing, and nothing here depends on it.
  }
}

/// Record an operation that landed. Best-effort for the same reason.
export async function recordOp(kind: OpKind, src: string[], dst?: string): Promise<void> {
  const record = opRecord(kind, src, dst);
  if (!record) return;
  try {
    await timeline.record(record);
  } catch {
    // Same seam.
  }
}

/// Follow the focused pane and publish where the person is, clearing on blur.
///
/// The subscription shape mirrors `topbar.ts`, which follows the same path for
/// the breadcrumb - but this one is NOT gated on `shell_present`, because the
/// knowledge daemon is running whether or not a desktop shell is and the graph's
/// input has no business depending on one.
///
/// Idempotent: a second call is a no-op, so a re-mount does not stack
/// subscriptions.
let started = false;

export async function initGraphInput(): Promise<void> {
  if (started) return;
  started = true;

  let unPath: (() => void) | null = null;
  focusedController.subscribe((c) => {
    unPath?.();
    unPath = null;
    if (!c) {
      void publishPresence(null);
      return;
    }
    unPath = c.path.subscribe((p) => void publishPresence(p));
  });

  // PRESENCE IS EPHEMERAL, so somebody has to end it: the SDK emits and leaves
  // the WHEN to the app, and for a browser it is the window losing focus.
  try {
    await getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      const c = get(focusedController);
      void publishPresence(focused && c ? get(c.path) : null);
    });
  } catch {
    // No toplevel (vite): nothing to lose focus, so nothing to clear.
  }
}
