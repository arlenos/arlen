/// Why the last attempt to open a file did not open it.
///
/// The file manager asks the shell's launch socket to open a document, and the
/// shell answers with an OUTCOME rather than a boolean: not-configured,
/// not-installed, badly-packaged and would-not-start are four different things
/// for a person to do next, and the contract draws those distinctions on
/// purpose. Until 21 August the window drew none of them - `openPath` caught the
/// error and dropped it, with a comment promising a status line "later" - so
/// pressing Enter on a file nothing opens did nothing at all, silently.
///
/// The sentence is written HERE rather than in the host, for the reason every
/// other sentence in this app is: a German reader must not be handed English.
/// The host sends the outcome as the contract's own tagged JSON and this turns
/// it into a message id plus the one value worth naming.

import { writable } from "svelte/store";

/// A refusal to show, or `null` when the last open worked.
export type OpenFailure = {
  /// The message id in this app's catalogue.
  key: string;
  /// The mime type or application the outcome named, interpolated into it.
  what: string;
};

/// Set by `openPath`, read by the status bar. Cleared by the next successful
/// open rather than on a timer: a message about the file you just tried is worth
/// leaving on screen until something else happens.
export const openFailure = writable<OpenFailure | null>(null);

/// Turn the host's answer into something to say.
///
/// UNPARSEABLE INPUT IS SHOWN, not swallowed. If the host ever sends a plain
/// string - an I/O failure before the socket, say - that string is the best
/// thing available and dropping it would put this window back where it was.
export function launchProblem(raw: string): OpenFailure {
  // Tauri stringifies a command error, so the JSON arrives inside the message.
  const start = raw.indexOf("{");
  const json = start >= 0 ? raw.slice(start) : "";
  let outcome: Record<string, unknown> | null = null;
  try {
    outcome = json ? (JSON.parse(json) as Record<string, unknown>) : null;
  } catch {
    outcome = null;
  }
  if (!outcome || typeof outcome.outcome !== "string") {
    return { key: "f.open.failed", what: raw };
  }
  switch (outcome.outcome) {
    case "no_handler":
      return { key: "f.open.noHandler", what: String(outcome.mime ?? "") };
    case "unknown_application":
      return { key: "f.open.notInstalled", what: String(outcome.app_id ?? "") };
    case "malformed_entry":
      return { key: "f.open.packagedWrong", what: String(outcome.app_id ?? "") };
    case "did_not_start":
      return { key: "f.open.didNotStart", what: String(outcome.app_id ?? "") };
    case "refused":
      return { key: "f.open.refused", what: "" };
    default:
      return { key: "f.open.failed", what: raw };
  }
}
