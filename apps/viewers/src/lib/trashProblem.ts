/// Why a delete did not happen, in the reader's language.
///
/// The host used to write seven English sentences for the seven ways a
/// freedesktop trash can refuse a file. They were good sentences - somebody had
/// thought about each case - and they were on the side of the boundary where a
/// German window cannot reach them, so the German build showed a German frame
/// around English content. The host sends the WORD now and this writes the
/// sentence.
///
/// UNRECOGNISED INPUT IS SHOWN, not swallowed, which is the rule the file
/// manager already follows: if the host ever answers with something this does
/// not model, that string is the only thing anybody has.

/// A message id and the one value worth putting in it.
export type TrashProblem = { key: string; detail: string };

/// Turn the host's answer into something to say.
export function trashProblem(raw: string): TrashProblem {
  // Tauri stringifies a command error, so the JSON arrives inside the message.
  const start = raw.indexOf("{");
  let parsed: Record<string, unknown> | null = null;
  try {
    parsed = start >= 0 ? (JSON.parse(raw.slice(start)) as Record<string, unknown>) : null;
  } catch {
    parsed = null;
  }
  if (!parsed || typeof parsed.problem !== "string") {
    return { key: "v.couldNotDelete", detail: raw };
  }
  switch (parsed.problem) {
    case "cross-device":
      return { key: "v.trash.crossDevice", detail: "" };
    case "no-trash-here":
      return { key: "v.trash.noTrashHere", detail: String(parsed.why ?? "") };
    case "not-found":
      return { key: "v.trash.notFound", detail: "" };
    case "unsupported":
      return { key: "v.trash.unsupported", detail: "" };
    case "no-slot":
      return { key: "v.trash.noSlot", detail: "" };
    case "non-canonical":
      return { key: "v.trash.nonCanonical", detail: "" };
    case "io":
      return { key: "v.trash.io", detail: String(parsed.message ?? "") };
    default:
      return { key: "v.couldNotDelete", detail: raw };
  }
}

/// Why a RESTORE did not happen. Same rule, a different set of words: the layer
/// below answers with a rename error, and the one a person meets is "something
/// is using that name again", which the window has to say rather than print
/// `DestinationExists` at them.
export function restoreProblem(raw: string): TrashProblem {
  const start = raw.indexOf("{");
  let parsed: Record<string, unknown> | null = null;
  try {
    parsed = start >= 0 ? (JSON.parse(raw.slice(start)) as Record<string, unknown>) : null;
  } catch {
    parsed = null;
  }
  if (!parsed || typeof parsed.problem !== "string") {
    return { key: "v.couldNotRestore", detail: raw };
  }
  switch (parsed.problem) {
    case "destination-exists":
      return { key: "v.restore.nameTaken", detail: "" };
    case "unsupported":
      return { key: "v.restore.unsupported", detail: "" };
    case "cross-device":
      return { key: "v.restore.crossDevice", detail: "" };
    case "other":
      return { key: "v.couldNotRestore", detail: String(parsed.message ?? "") };
    default:
      return { key: "v.couldNotRestore", detail: raw };
  }
}

/// Why a print did not start.
///
/// The shell plugin answers with a word now (`no-portal`, `no-bus`,
/// `file-unreadable`, `portal-refused`, `other`), each carrying what the layer
/// below said. On a machine with no printing set up, `no-portal` is not an edge
/// case - it is what every print does - and it deserves a sentence rather than a
/// D-Bus error in English.
export function printProblem(raw: string): TrashProblem {
  const start = raw.indexOf("{");
  let parsed: Record<string, unknown> | null = null;
  try {
    parsed = start >= 0 ? (JSON.parse(raw.slice(start)) as Record<string, unknown>) : null;
  } catch {
    parsed = null;
  }
  if (!parsed || typeof parsed.problem !== "string") {
    return { key: "v.couldNotPrint", detail: raw };
  }
  const detail = String(parsed.message ?? "");
  switch (parsed.problem) {
    case "no-portal":
      return { key: "v.print.noPortal", detail: "" };
    case "no-bus":
      return { key: "v.print.noBus", detail: "" };
    case "file-unreadable":
      return { key: "v.print.fileUnreadable", detail };
    default:
      // `portal-refused` and `other` both mean the machine CAN print and this
      // attempt did not, which is the case where the detail is the useful part.
      return { key: "v.couldNotPrint", detail };
  }
}
