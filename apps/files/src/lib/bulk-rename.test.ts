// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

/// The TypeScript half of the shared bulk-rename contract.
///
/// Its twin is `apps/files/core/tests/bulk_rename_vectors.rs`, and both run the
/// SAME vector file. The reason this exists: the rename rules are implemented
/// twice - the core performs the rename, this module draws the preview without a
/// round-trip per keystroke - and until now nothing checked the two agreed. A
/// divergence does not throw or log; it shows one name in the dialog and writes
/// another, on an operation the person approved because of what the dialog said.
///
/// The core is authoritative. If this test fails, the answer is to change this
/// module to match the core, never to loosen the vectors.

import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { planRename, type RenameRule } from "./bulk-rename";

const VECTORS = fileURLToPath(
  new URL("../../core/tests/bulk-rename-cases.json", import.meta.url),
);

interface Expected {
  from: string;
  to: string;
  conflict: string;
}

interface Case {
  name: string;
  names: string[];
  rule: Partial<RenameRule>;
  expect: Expected[];
}

const cases: Case[] = JSON.parse(readFileSync(VECTORS, "utf8")).cases;

describe("the preview satisfies the shared rename vectors", () => {
  it("has vectors to run", () => {
    // A file that silently stopped being found would make every case below pass
    // by not existing, which reads exactly like agreement.
    expect(cases.length).toBeGreaterThan(0);
  });

  for (const c of cases) {
    it(c.name, () => {
      // The JSON carries only the fields a case exercises, which is also how the
      // core reads it (serde fills the rest with defaults); the two required
      // fields are spelled here so the shapes match without repeating them in
      // every vector.
      const rule: RenameRule = {
        replace: "",
        find_case_insensitive: false,
        ...c.rule,
      };
      const got = planRename(c.names, rule);
      expect(got.map((r) => ({ from: r.old, to: r.new, conflict: r.conflict }))).toEqual(
        c.expect,
      );
    });
  }
});
