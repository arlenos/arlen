/// What this window tells the knowledge graph about itself.
///
/// The same two surfaces the text editor publishes, and the same reason: the
/// eBPF sensor sees a process open a path, and cannot see that the path was
/// being LOOKED AT, that it was a picture rather than a sound, or that a print
/// at 14:03 actually went to a printer. Only the window knows that, which is why
/// the daemon has promoted `app.presence.*` and `app.timeline.record` into
/// UserAction nodes since they were written while nothing sent one.
///
/// PRESENCE IS EPHEMERAL and cleared when this window loses focus; a TIMELINE
/// record is a completed moment and stays. Opening a file is not one - the
/// sensor already saw the open, and the timeline is for what was
/// user-meaningful. Printing is: it left the machine.
///
/// The subject is the file path, which the sensor already records for every
/// open. The picture, the sound and their contents never leave.

import { presence, timeline, type PresenceParams, type TimelineParams } from "@arlen/tauri-plugin-shell";

/// What a person is doing with this kind of file.
///
/// A closed map rather than a free string: `activity` is what a graph query
/// groups by, and an app inventing a synonym for a verb another app already uses
/// makes the graph less answerable, not more precise.
export function activityFor(kind: string): string {
  return kind === "audio" ? "listening" : "viewing";
}

/// The presence for an open file, or null when the window is showing none.
export function viewingPresence(path: string | null, kind: string): PresenceParams | null {
  if (!path) return null;
  return {
    activity: activityFor(kind),
    subject: path,
    // The kind as well as the verb: "viewing" and "listening" already differ,
    // and a reader asking what sort of thing it was should not have to infer it
    // from the extension.
    //
    // OMITTED WHEN UNKNOWN, rather than sent empty. Presence is published the
    // moment the window has a path, which is before the file has been decoded
    // and the kind is known - the drive caught it doing exactly that and
    // publishing `kind: ""`. An empty value is a claim that the kind is empty;
    // an absent one is the truth, and the re-publish a moment later carries the
    // real answer.
    ...(kind ? { metadata: { kind } } : {}),
    auto_clear: "on-blur",
  };
}

/// The record for a print the portal actually sent.
///
/// A TOKEN, not a sentence, for the reason the text editor's save record gives:
/// a record is written once and read later, possibly by a surface in another
/// language, so the wording belongs to whoever draws it.
export function printedRecord(path: string, kind: string): TimelineParams {
  return { label: "printed", subject: path, type: "print", metadata: { kind } };
}

/// Publish the presence for an open file, or take the last one down.
///
/// Best-effort, like every other publish in this tree: under vite there is no
/// shell relay and the call rejects, and a viewer that cannot reach the bus is
/// still a viewer.
export async function publishPresence(path: string | null, kind: string): Promise<void> {
  const params = viewingPresence(path, kind);
  try {
    if (params) await presence.set(params);
    else await presence.clear();
  } catch {
    // No shell relay: the graph hears nothing, and nothing here depends on it.
  }
}

/// Record a print that was sent. Only `sent` - a cancelled or refused print is
/// not a moment, and recording it would put something in somebody's history that
/// did not happen.
export async function recordPrint(path: string, kind: string): Promise<void> {
  try {
    await timeline.record(printedRecord(path, kind));
  } catch {
    // Same seam.
  }
}
