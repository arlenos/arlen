// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control: a picture the catch-all is the only thing covering has to fail, a
// picture with a block about it has to pass, an SVG stating its own licence has
// to pass, and a tree with no pictures in it has to REFUSE rather than report a
// clean scan of nothing.
import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

// REUSE-IgnoreStart
//
// Everything below is FIXTURE text: `REUSE.toml` fragments and an SVG header that
// exist to be fed to the gate, not to declare anything about this file. `reuse
// lint` reads every file in the tree and cannot tell a sample expression from a
// real one, so without these markers the samples are parsed as this repository's
// own declaration and the `license` job goes red - which is exactly what happened
// on the commit that added this test.
//
// THE NEXT FIXTURE INHERITS THIS: any test planting a licence header, a copyright
// line or a `REUSE.toml` fragment needs the same pair, and
// `check-license-headers-agree.py` now fails a file that carries one without them.

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const GATE = join(ROOT, "dev/scripts/check-image-licensing.py");

let failures = 0;
const ok = (n) => console.log(`  ok   ${n}`);
const bad = (n, d) => {
  console.log(`  FAIL ${n}: ${d}`);
  failures += 1;
};

const CATCH_ALL = `version = 1

[[annotations]]
path = "**"
precedence = "closest"
SPDX-FileCopyrightText = "2026 Tim Kicker"
SPDX-License-Identifier = "AGPL-3.0-only"
`;

function tree(reuse, images) {
  const dir = mint("image-licensing-");
  writeFileSync(join(dir, "REUSE.toml"), reuse);
  for (const [path, body] of Object.entries(images)) {
    const abs = join(dir, path);
    mkdirSync(dirname(abs), { recursive: true });
    writeFileSync(abs, body ?? "not really a png");
  }
  return dir;
}

function run(dir) {
  try {
    execFileSync("python3", [GATE, dir], { encoding: "utf8" });
    return { code: 0, out: "" };
  } catch (e) {
    return { code: e.status ?? 1, out: (e.stdout || "") + (e.stderr || "") };
  }
}

{
  const d = tree(CATCH_ALL, { "apps/one/src-tauri/icons/32x32.png": null });
  const r = run(d);
  if (r.code === 1 && r.out.includes("32x32.png")) {
    ok("a picture only the catch-all covers is caught");
  } else bad("a picture only the catch-all covers is caught", `exit ${r.code} ${r.out}`);
  cleanup(d);
}

{
  const d = tree(
    CATCH_ALL +
      `
[[annotations]]
path = "apps/one/src-tauri/icons/**"
precedence = "closest"
SPDX-FileCopyrightText = "Somebody Else"
SPDX-License-Identifier = "MIT"
`,
    { "apps/one/src-tauri/icons/32x32.png": null },
  );
  const r = run(d);
  if (r.code === 0) ok("a picture with a block about it passes");
  else bad("a picture with a block about it passes", `exit ${r.code} ${r.out}`);
  cleanup(d);
}

// `*` must not cross a separator, or `apps/*/icons/**` would quietly cover a
// tree it does not name and the check would pass on a false answer.
{
  const d = tree(
    CATCH_ALL +
      `
[[annotations]]
path = "apps/*/icons/**"
precedence = "closest"
SPDX-FileCopyrightText = "Somebody Else"
SPDX-License-Identifier = "MIT"
`,
    { "apps/one/nested/icons/32x32.png": null },
  );
  const r = run(d);
  if (r.code === 1) ok("a single star does not cross a directory separator");
  else bad("a single star does not cross a directory separator", `exit ${r.code} ${r.out}`);
  cleanup(d);
}

{
  const d = tree(CATCH_ALL, {
    "apps/one/static/logo.svg":
      "<!-- SPDX-License-Identifier: CC0-1.0 -->\n<svg></svg>\n",
  });
  const r = run(d);
  if (r.code === 0) ok("an SVG stating its own licence needs no block");
  else bad("an SVG stating its own licence needs no block", `exit ${r.code} ${r.out}`);
  cleanup(d);
}

{
  const d = tree(CATCH_ALL, { "apps/one/src/main.rs": "fn main() {}\n" });
  const r = run(d);
  if (r.code === 2) ok("a tree with no pictures refuses instead of reporting a clean scan");
  else bad("a tree with no pictures refuses instead of reporting a clean scan", `exit ${r.code}`);
  cleanup(d);
}

console.log(failures === 0 ? "all image-licensing cases passed" : `${failures} case(s) failed`);
process.exit(failures === 0 ? 0 : 1);

// REUSE-IgnoreEnd
