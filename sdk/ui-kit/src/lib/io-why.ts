// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

/// A host says why a file operation failed with the text of an `io::Error`
/// ("Read-only file system (os error 30)"), and more than one application was
/// splicing that text after a sentence of its own. The errno texts are finite
/// and stable, so this reads them into a kit catalogue key the sentence can say
/// in the reader's language; a text it does not know yields nothing, so the
/// framing sentence stands alone, and the text goes to the console.
const MARKERS: [RegExp, string][] = [
  [/Permission denied|os error 13\b/, "k.why.permission"],
  [/Read-only file system|os error 30\b/, "k.why.readOnly"],
  [/No space left|os error 28\b/, "k.why.noSpace"],
  [/No such file or directory|os error 2\b/, "k.why.gone"],
  [/Is a directory|Not a directory|os error 2[01]\b/, "k.why.notAFile"],
];

/// The kit catalogue key for an errno text, or null for one this does not know.
export function ioWhyKey(why: string | null | undefined): string | null {
  if (!why) return null;
  for (const [marker, key] of MARKERS) if (marker.test(why)) return key;
  console.warn("kit: unrecognised host reason", why);
  return null;
}
