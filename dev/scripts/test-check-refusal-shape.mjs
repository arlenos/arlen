// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// A refused D-Bus caller must get an error, not an empty value, and this check has
// to see the refusal however the method is written.
//
// The two return forms are the point. `EMPTY` used to require the literal `return`
// keyword, so a method ending in a bare `Vec::new()` after a refusal warning - the
// tail expression, which is how anyone writing this fresh writes it - passed
// completely. It was built against nine existing instances that all happened to
// use `return`. Found on 11 August by writing the tenth.
//
// The last two cases guard the other direction, which matters more here than in
// most of these gates: the correct fix returns an error, and an ordinary empty
// list is not a defect. A check that flagged either would push people away from
// the shape it is asking for.
//
// Run: node dev/scripts/test-check-refusal-shape.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-refusal-shape.py");

const failures = [];

function check(name, method, expect) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-refusal-"));
  const src = join(dir, "daemons/probe/src");
  mkdirSync(src, { recursive: true });
  writeFileSync(
    join(src, "dbus.rs"),
    `#[zbus::interface(name = "org.arlen.Probe1")]\nimpl Probe {\n${method}}\n`,
  );
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  const got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  const ok = expect(got.code, got.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
  rmSync(dir, { recursive: true, force: true });
}

console.log("check-refusal-shape:");

check(
  "a refusal answered with an explicit return of nothing is caught",
  `    async fn listing(&self) -> Vec<String> {
        tracing::warn!("refused: caller is not permitted");
        return Vec::new();
    }
`,
  (code, out) => code === 1 && out.includes("listing"),
);

// The tail expression, which the return-keyword version could not see.
check(
  "a refusal answered with a bare tail expression is caught",
  `    async fn listing(&self) -> Vec<String> {
        tracing::warn!("refused: caller is not permitted");
        Vec::new()
    }
`,
  (code, out) => code === 1 && out.includes("listing"),
);

check(
  "a refusal that returns an error passes",
  `    async fn listing(&self) -> zbus::fdo::Result<Vec<String>> {
        tracing::warn!("refused: caller is not permitted");
        Err(zbus::fdo::Error::AccessDenied("not permitted".into()))
    }
`,
  (code) => code === 0,
);

check(
  "an ordinary empty answer with no refusal in it passes",
  `    async fn listing(&self) -> Vec<String> {
        Vec::new()
    }
`,
  (code) => code === 0,
);

// The second rule, in the other medium: a launcher that refuses an argv by
// exiting and printing nothing. `launcher` writes the file the rule reads.
function launcher(name, body, expect) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-refusal-"));
  const src = join(dir, "daemons/arlen-run/src");
  mkdirSync(src, { recursive: true });
  writeFileSync(join(src, "main.rs"), body);
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  const got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  const ok = expect(got.code, got.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
  rmSync(dir, { recursive: true, force: true });
}

launcher(
  "a launcher refusal that prints nothing is caught",
  'fn parse() -> Result<(), u8> {\n    return Err(exit::BAD_ARGS);\n}\n',
  (code, out) => code === 1 && out.includes("stops in silence"),
);

launcher(
  "a refusal routed through the helper passes",
  'fn parse() -> Result<(), u8> {\n    return Err(bad_args("no --app-id"));\n}\n',
  (code) => code === 0,
);

// The fix documents the shape it replaced, so the doc quotes the very pattern
// the rule looks for. Reading a comment as code would make the fix fail its own
// check - which is what the first version of this rule did.
launcher(
  "a doc comment quoting the bad shape is not a finding",
  '/// Every refusal used to be a bare `Err(exit::BAD_ARGS)`, which printed nothing.\nfn parse() -> Result<(), u8> {\n    return Err(bad_args("x"));\n}\n',
  (code) => code === 0,
);

// Test expectations name the code on purpose; demanding they print would be
// backwards.
launcher(
  "an assertion in the test module is not a refusal site",
  'fn parse() -> Result<(), u8> {\n    return Err(bad_args("x"));\n}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() { assert_eq!(parse(), Err(exit::BAD_ARGS)); }\n}\n',
  (code) => code === 0,
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("all refusal-shape cases passed");
