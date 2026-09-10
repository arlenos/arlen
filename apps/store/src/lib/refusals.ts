// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

/// The causes a refused install action can have, and how the host's own
/// sentence is read into one of them.
///
/// The four writing commands answer a refusal with prose: a sentence the Tauri
/// layer wrote, or installd's, wrapped in a D-Bus error name, or a filesystem
/// path with an errno. None of that is a person's sentence and none of it is
/// German. Until the boundary answers with a token, this is the one place that
/// reads the prose, on the markers the host is known to send, and hands the
/// surface a cause it can put into the reader's language. An unrecognised text
/// is a cause of its own, and the text goes to the console, where it is useful.

/// Why the host would not do it.
export type Cause =
  | "notInstalled"
  | "layerRemove"
  | "layerUpdate"
  | "packageFile"
  | "notRemovable"
  | "notAllowed"
  | "noDaemon"
  | "skipNotRecorded"
  | "tooLarge"
  | "other";

/// Markers in the order they are tried. The first match wins, so the specific
/// installd sentences come before the bare `AccessDenied` that wraps them.
const MARKERS: [RegExp, Cause][] = [
  [/is not installed\b/, "notInstalled"],
  [/recorded as installed from .* no way to remove/, "layerRemove"],
  [/recorded as installed from .* no way to update/, "layerUpdate"],
  [/installed as a package file/, "packageFile"],
  [/part of the desktop itself/, "notRemovable"],
  [/may not change what is installed/, "notAllowed"],
  [/AccessDenied|resolve caller|unknown binary path/, "notAllowed"],
  [/store transport error|socket path unresolved|ServiceUnknown|name is not owned|Connection refused|No such file or directory/, "noDaemon"],
  [/no data directory|serialise the skips|\.toml/, "skipNotRecorded"],
  [/too large to send/, "tooLarge"],
];

/// The cause behind a refusal the host answered with.
export function causeOf(e: unknown): Cause {
  const text = typeof e === "string" ? e : e instanceof Error ? e.message : "";
  for (const [marker, cause] of MARKERS) if (marker.test(text)) return cause;
  console.warn("store: unrecognised refusal", e);
  return "other";
}

/// The message key that says a cause in the reader's language, as the clause
/// after the colon of "Not updated:" or "Not uninstalled:".
export function whyKey(cause: Cause): string {
  return `st.why.${cause}`;
}
