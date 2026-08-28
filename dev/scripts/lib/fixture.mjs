// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Temporary fixtures for the control scripts, and the one rule that matters:
// NOTHING IS DELETED THAT THIS MODULE DID NOT CREATE.
//
// WHY IT EXISTS. On 27 August a control in this directory passed the REPOSITORY
// ROOT to a cleanup helper that ended in `rmSync(dir, { recursive: true })`. It
// deleted most of the working tree, `.git` included, and stopped only on an
// unwritable cache directory. The pattern that allowed it is the ordinary one in
// every control here: a helper takes a path as a parameter, and one caller passes
// a path it did not mint.
//
// The fix is not care. Care is what was already being applied. The fix is that the
// delete refuses a path it has no record of creating, so the mistake is a failed
// test rather than a lost afternoon.

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

/// Every directory this process created, and the only ones it may remove.
///
/// Module-scoped rather than per-caller: a set the caller passes in is a set the
/// caller can forge, and the whole point is that the record is not the caller's to
/// write.
const minted = new Set();

/// Create a temporary directory and record it as removable.
///
/// The prefix is for a human reading `/tmp`, so name it after the check.
export function mint(prefix) {
  const dir = mkdtempSync(join(tmpdir(), prefix));
  minted.add(dir);
  return dir;
}

/// Remove a directory this module minted.
///
/// EXITS rather than throws on a path it did not mint. A throw can be caught by a
/// `try` somewhere up the stack and turned back into a warning, and the thing being
/// guarded against is exactly a helper that carries on; the process ending with a
/// named path is the loudest available answer.
export function cleanup(dir) {
  if (!minted.has(dir)) {
    console.error(
      `REFUSED to remove ${dir}: this is not a directory the fixture helper created.`,
    );
    console.error(
      "A control may only delete what it minted. If this is a real fixture, mint it",
    );
    console.error("with `mint()`; if it is a path from somewhere else, do not delete it.");
    process.exit(1);
  }
  minted.delete(dir);
  rmSync(dir, { recursive: true, force: true });
}

/// Whether a path is one this module minted, for a control that wants to assert
/// the guard rather than trip it.
export function isMinted(dir) {
  return minted.has(dir);
}
