// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// `starts_with("dev.")` in an admission gate admits every locally-built binary in
// the tree, so this check has to find it wherever it is written - and excuse it
// only where it is genuinely a test talking about the shape.
//
// The placement cases are the point. The predecessor scanned backwards for the
// nearest `#[cfg(test)]` within 400 lines and excused anything below one, so an
// admission added at the BOTTOM of a file - after the test module Rust convention
// puts there - was invisible. Found on 10 August by injecting the same line into
// `event-bus/src/socket.rs` twice: caught above the test module, missed below it.
//
// Run: node dev/scripts/test-check-dev-prefix-admission.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-dev-prefix-admission.py");

const failures = [];

function check(name, body, expect) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-devprefix-"));
  const abs = join(dir, "daemons/thing/src/socket.rs");
  mkdirSync(dirname(abs), { recursive: true });
  writeFileSync(abs, body);
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  const got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  const ok = expect(got.code, got.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
  rmSync(dir, { recursive: true, force: true });
}

const ADMIT = 'fn admit(app_id: &str) -> bool { app_id.starts_with("dev.") }\n';

const TESTS = `#[cfg(test)]
mod tests {
    #[test]
    fn the_release_list_carries_no_debug_id() {
        assert!(!ID.starts_with("dev."));
    }
}
`;

console.log("check-dev-prefix-admission:");

check(
  "an admission above the test module is caught",
  `${ADMIT}\n${TESTS}`,
  (code, out) => code === 1 && out.includes("socket.rs"),
);

// The case the backwards-scan version passed.
check(
  "an admission BELOW the test module is caught",
  `${TESTS}\n${ADMIT}`,
  (code, out) => code === 1 && out.includes("socket.rs"),
);

check(
  "a test asserting about the prefix is still excused",
  TESTS,
  (code) => code === 0,
);

// A `#[cfg(test)]` on a single item must not excuse the rest of the file either,
// which is the same defect in its smaller form.
check(
  "a cfg(test) helper does not excuse the code after it",
  `#[cfg(test)]\nfn helper() -> bool { true }\n\n${ADMIT}`,
  (code, out) => code === 1 && out.includes("socket.rs"),
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("all dev-prefix gate cases passed");
