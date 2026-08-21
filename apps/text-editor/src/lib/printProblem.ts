/// Why a print did not start, in the reader's language.
///
/// The shell plugin answers a failed print with a WORD (`no-portal`, `no-bus`,
/// `file-unreadable`, `portal-refused`, `other`) rather than an English
/// sentence, so a German window can say what happened instead of framing a
/// D-Bus error. On a machine with no printing stack `no-portal` is not an edge
/// case: it is what every print does.
///
/// THE TWIN OF THIS LIVES IN `apps/viewers/src/lib/trashProblem.ts`, and the two
/// are deliberately separate copies. The shared home for a helper both windows
/// want is `sdk/ui-kit`, which is another lane's to change; a copy that says so
/// is better than reaching into somebody else's work, and better than each
/// window inventing its own mapping quietly.
///
/// An answer this does not model is shown as it came, because the string is then
/// the only thing anybody has.

/// A message id and the one value worth putting in it.
export type PrintProblem = { key: string; detail: string };

/// Turn the plugin's answer into something to say.
export function printProblem(raw: string): PrintProblem {
  const start = raw.indexOf("{");
  let parsed: Record<string, unknown> | null = null;
  try {
    parsed = start >= 0 ? (JSON.parse(raw.slice(start)) as Record<string, unknown>) : null;
  } catch {
    parsed = null;
  }
  if (!parsed || typeof parsed.problem !== "string") {
    return { key: "te.print.failed", detail: raw };
  }
  const detail = String(parsed.message ?? "");
  switch (parsed.problem) {
    case "no-portal":
      return { key: "te.print.noPortal", detail: "" };
    case "no-bus":
      return { key: "te.print.noBus", detail: "" };
    case "file-unreadable":
      return { key: "te.print.fileUnreadable", detail };
    default:
      return { key: "te.print.failed", detail };
  }
}
