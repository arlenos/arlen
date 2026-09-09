/// What this window tells the knowledge graph about itself.
///
/// PRESENCE ONLY, and the absence of the other half is deliberate. The editor
/// records a save and the viewer records a print, because each of those is a
/// completed, user-meaningful moment. A reader has none: opening the document is
/// something the sensor already saw, and reaching page forty is not an event -
/// it is the middle of one. Inventing a timeline record here would put rows in
/// somebody's history that mean nothing when they read them back.
///
/// What the sensor cannot see, and this can: that the path was being READ, and
/// how long the document is. A hundred-page report and a one-page receipt are
/// the same `openat` to a kernel probe.
///
/// The subject is the file path, which the sensor already records for every
/// open. The document's contents never leave.

import { presence, type PresenceParams } from "@arlen/tauri-plugin-shell";

/// The presence for an open document, or null when the window is showing none.
///
/// The document's LENGTH and not the current page: presence is republished on
/// every change, and a page number changes on every scroll - which would put a
/// row on the bus for each one and tell a later reader nothing they wanted.
export function readingPresence(path: string | null, pages: number): PresenceParams | null {
  if (!path) return null;
  const known = Number.isFinite(pages) && pages > 0;
  return {
    activity: "reading",
    subject: path,
    ...(known ? { metadata: { pages: String(Math.trunc(pages)) } } : {}),
    auto_clear: "on-blur",
  };
}

/// Publish the presence for an open document, or take the last one down.
///
/// Best-effort, like every other publish in this tree: under vite there is no
/// shell relay and the call rejects, and a reader that cannot reach the bus is
/// still a reader.
export async function publishPresence(path: string | null, pages: number): Promise<void> {
  const params = readingPresence(path, pages);
  try {
    if (params) await presence.set(params);
    else await presence.clear();
  } catch {
    // No shell relay: the graph hears nothing, and nothing here depends on it.
  }
}
