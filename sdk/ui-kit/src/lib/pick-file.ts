// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: MIT

import { invoke } from "@tauri-apps/api/core";

/// One entry in the chooser's kind list: the name a person reads and the
/// glob patterns behind it.
export type PickKind = { name: string; globs: string[] };

type PickerResult = { type: "picked"; uris: string[] } | { type: "cancelled" };

/// The path a `file://` URI names, or null for anything that is not a local
/// file. The portal answers in URIs and every host command here takes a path;
/// a name with a space or an umlaut arrives percent-encoded.
export function pathOfUri(uri: string): string | null {
  let url: URL;
  try {
    url = new URL(uri);
  } catch {
    return null;
  }
  if (url.protocol !== "file:") return null;
  try {
    return decodeURIComponent(url.pathname);
  } catch {
    return null;
  }
}

/// Ask the system's file chooser for one file. The path, or null when the
/// person closed the chooser without choosing; that is an answer, not a
/// failure. It throws when the chooser could not be shown at all, and the
/// caller says so with `k.picker.unavailable`.
export async function pickOneFile(kinds: PickKind[] = []): Promise<string | null> {
  const result = await invoke<PickerResult>("plugin:arlen-portal|pick_file", {
    options: {
      filters: kinds.map((k) => ({
        name: k.name,
        patterns: k.globs.map((pattern) => ({ kind: "glob", pattern })),
      })),
    },
  });
  if (result.type !== "picked") return null;
  const first = result.uris[0];
  return first ? pathOfUri(first) : null;
}
