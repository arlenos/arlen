/// What this window tells the knowledge graph about itself.
///
/// PRESENCE ONLY, and it is the one in the tree a sensor comes least close to.
/// The editor's `editing` and the reader's `reading` at least sit on top of an
/// `openat` the kernel probe also saw; being IN A MEETING touches no file at all.
/// A person asking their own history "what was I doing at two" gets nothing for
/// the hour they spent in a call unless this window says so.
///
/// NO TIMELINE RECORD from here. The meeting itself is already filed into the
/// graph when the capture produces its note - `meetings_list` reads those nodes -
/// so recording "a meeting happened" would be the same fact twice, once well and
/// once thinly. What is missing is the INTERVAL, and an interval is what presence
/// is for.
///
/// THE SUBJECT IS THE MEETING THIS CAPTURE WILL BECOME. Until the graph files it
/// there is no id, and this app already has a word for that state: `currentId` is
/// documented as `"live"` right after a capture. Reusing it beats inventing a
/// second name for the same thing, and beats an empty subject, which would be a
/// claim that the subject is nothing.
///
/// No transcript, no notes, no participant ever leaves through here.

import { presence, type PresenceParams } from "@arlen/tauri-plugin-shell";

/// The word this app uses for a capture the graph has not filed yet.
export const LIVE = "live";

/// Is a microphone actually on?
///
/// POSITIVE EVIDENCE, and the first cut of this got it wrong in a way worth
/// keeping written down. It read `!unavailable && !stopFailed` - the same shape
/// the red dot's branch uses - and both of those are false in the moment between
/// pressing Start and the host answering. So on a machine with no ASR engine, a
/// window that correctly went on to say "nothing is being captured" had already
/// told the graph a meeting was happening. A wrong pixel is replaced a moment
/// later; a wrong interval stays in somebody's own history.
///
/// A refused STOP is deliberately still capturing: the microphone may be live,
/// and the app does not get to record the outcome the person hoped for.
export function isCapturing(capturing: boolean): boolean {
  return capturing;
}

/// The presence for a capture in progress, or null when there is none.
export function capturePresence(
  capturing: boolean,
  meetingId: string | null,
  transcribing: boolean,
): PresenceParams | null {
  if (!capturing) return null;
  return {
    activity: "meeting",
    subject: meetingId ?? LIVE,
    // Recording and transcribing are DIFFERENT CONSENTS in this app, and which
    // of the two was running is exactly the thing somebody would want to know
    // afterwards. It is the app's own state and nothing outside can see it.
    metadata: { transcribing: transcribing ? "true" : "false" },
    auto_clear: "on-blur",
  };
}

/// Publish the presence for a capture, or take the last one down.
///
/// Best-effort, like every publish in this tree: under vite there is no relay
/// and the call rejects, and a window that cannot reach the bus is still a
/// window.
export async function publishPresence(
  capturing: boolean,
  meetingId: string | null,
  transcribing: boolean,
): Promise<void> {
  const params = capturePresence(capturing, meetingId, transcribing);
  try {
    if (params) await presence.set(params);
    else await presence.clear();
  } catch {
    // No relay: the graph hears nothing, and nothing here depends on it.
  }
}
