/// What this window tells the knowledge graph about itself.
///
/// Two surfaces, and the difference between them is the whole design
/// (`presence.rs`, `timeline.rs`, foundation §354 / §468):
///
///   * PRESENCE is what the user is doing NOW, and it is ephemeral. It is set
///     while a file is open and cleared when the window loses focus, so a query
///     like "what was I editing at two o'clock" answers from intervals rather
///     than from guesses.
///   * TIMELINE is a completed moment, and it is persistent. Saving a file is
///     one; opening it is not, because the sensor already saw the open and the
///     timeline is meant to hold what was user-MEANINGFUL rather than what
///     happened to touch the disk.
///
/// WHY AN APP SAYS THIS AT ALL. The eBPF sensor sees a process open a path. It
/// cannot see that the path was being EDITED rather than read, in which language,
/// or that a save at 14:03 was the moment the work landed. That distinction is
/// what the graph exists for and only the app knows it, which is why twenty-one
/// shell surfaces sat with both consumer ends built and no producer: the graph's
/// app-level input was dark and the sensor's view of files was all it had.
///
/// NO NEW EXPOSURE, and it is worth saying where somebody auditing will look.
/// The subject is the file path, which the sensor already records for every open;
/// what is added is the meaning, not the material. The buffer's CONTENTS never
/// leave, in either surface.

import { presence, timeline, type PresenceParams, type TimelineParams } from "@arlen/tauri-plugin-shell";

/// The presence for an open document, or null when there is no file - the
/// example buffer is not a thing anybody is editing and must not read as one.
export function editingPresence(path: string | null, language: string): PresenceParams | null {
  if (!path) return null;
  return {
    activity: "editing",
    subject: path,
    // Generously rather than minimally: the language is the difference between
    // "was at a file" and "was writing Rust", and it is the app's to know.
    metadata: { language },
    // The SDK emits; WHEN to clear is the consumer's choice, and for an editor
    // it is the window losing focus - after which "currently editing" is a claim
    // nobody can stand behind.
    auto_clear: "on-blur",
  };
}

/// The timeline record for a save that landed.
///
/// THE LABEL IS A TOKEN, NOT A SENTENCE, and this is a deliberate divergence from
/// the SDK's own wording. `TimelineParams.label` is documented as a "user-facing
/// summary, e.g. Exported PDF" - but a timeline record is written once and read
/// LATER, by a different surface, possibly in a different language from the one
/// the writer had that afternoon. The knowledge app's timeline already learnt
/// this and stores its verb as a MESSAGE ID rather than a word, with the reason
/// written beside it: the word rendered verbatim, so every row on a German page
/// was English. A stored sentence freezes the reader's language at write time.
///
/// So this publishes `saved`, and wording it stays with whoever draws it. If the
/// SDK doc is right and mine is wrong, the fix belongs in one place rather than
/// in each app that ever records anything.
///
/// `chars` rather than bytes: it is what this window actually knows without
/// re-encoding, and a length is a size rather than a content.
export function savedRecord(path: string, chars: number, language: string): TimelineParams {
  return {
    label: "saved",
    subject: path,
    type: "save",
    metadata: { language, chars: String(Math.max(0, Math.trunc(chars))) },
  };
}

/// Publish the presence for an open document, or take the last one down.
///
/// Best-effort, the same rule the menu registration and the badge follow: under
/// vite there is no shell relay and the call rejects, and an editor that cannot
/// reach the bus is still an editor.
export async function publishPresence(path: string | null, language: string): Promise<void> {
  const params = editingPresence(path, language);
  try {
    if (params) await presence.set(params);
    else await presence.clear();
  } catch {
    // No shell relay: the graph hears nothing, and nothing here depends on it.
  }
}

/// Record a save on the timeline. Best-effort for the same reason.
export async function recordSave(path: string, chars: number, language: string): Promise<void> {
  try {
    await timeline.record(savedRecord(path, chars, language));
  } catch {
    // Same seam.
  }
}
