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

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("all refusal-shape cases passed");
