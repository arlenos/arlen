// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-profile-claims.
//
// The planted defect is the real one, verbatim: `org.x.Warpinator` read "It gets
// network and Downloads" after its filesystem table was removed.
//
// The four negative cases are the four false positives this check produced while
// it was being written, each from the actual corpus. They are here because every
// one of them was a version of the check that would have shipped noisy, and a
// prose heuristic that cries wolf teaches people to reword around it rather than
// to fix anything.
//
// Run: node dev/scripts/test-check-profile-claims.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-profile-claims.py");
const failures = [];

function tree(files) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-claims-"));
  mkdirSync(join(dir, "sdk/permissions/profiles"), { recursive: true });
  for (const [name, body] of Object.entries(files)) {
    writeFileSync(join(dir, "sdk/permissions/profiles", `${name}.toml`), body);
  }
  return dir;
}

const run = (dir) => {
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  return { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
};

function check(name, ok, detail) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, detail });
}

console.log("check-profile-claims:");

// The defect, as it actually happened.
let d = tree({
  "org.x.Warpinator":
    "# Starting permission profile for Warpinator (local-network file transfer).\n" +
    "# It sends and receives files by function. It gets network and Downloads.\n" +
    '[info]\napp_id = "org.x.Warpinator"\ntier = "third-party"\n',
});
let r = run(d);
check(
  "a description claiming a grant the profile lost is reported",
  r.code === 1 && /Downloads/.test(r.out),
  `exit=${r.code} out=${r.out}`,
);
rmSync(d, { recursive: true, force: true });

// A claim the grants back up.
d = tree({
  ok1:
    "# Plays local media, so it gets Music.\n" +
    '[info]\napp_id = "ok1"\ntier = "third-party"\n\n[filesystem]\nmusic = true\n',
});
r = run(d);
check("a claim its grants honour passes", r.code === 0, `exit=${r.code} out=${r.out}`);
rmSync(d, { recursive: true, force: true });

// False positive 1: a later paragraph QUOTES the discarded reasoning.
d = tree({
  quoted:
    "# Opens images the user picks through the portal; no standing grant.\n" +
    "#\n" +
    "# This held the whole tree on the reasoning that it opens an image and saves\n" +
    "# the copy, so it gets home filesystem access. That does not follow.\n" +
    '[info]\napp_id = "quoted"\ntier = "third-party"\n',
});
r = run(d);
check(
  "a note quoting the reasoning it replaced is not a claim",
  r.code === 0,
  `exit=${r.code} out=${r.out}`,
);
rmSync(d, { recursive: true, force: true });

// False positive 2: the corpus saying what an app does NOT get.
d = tree({
  showtime:
    "# It plays local video and audio, so it gets Videos and Music - not Home.\n" +
    '[info]\napp_id = "showtime"\ntier = "third-party"\n\n' +
    "[filesystem]\nvideos = true\nmusic = true\n",
});
r = run(d);
check("a negated directory is not a claim", r.code === 0, `exit=${r.code} out=${r.out}`);
rmSync(d, { recursive: true, force: true });

// False positive 3 and 4: the verb and the common noun, which the corpus writes
// lowercase while the directory is capitalised.
d = tree({
  wardrobe:
    "# It downloads themes, so it gets network; no user files.\n" +
    '[info]\napp_id = "wardrobe"\ntier = "third-party"\n',
  finamp:
    "# Its function is online (music streaming), so it gets network only.\n" +
    '[info]\napp_id = "finamp"\ntier = "third-party"\n',
});
r = run(d);
check(
  "a lowercase verb or common noun is not a directory",
  r.code === 0,
  `exit=${r.code} out=${r.out}`,
);
rmSync(d, { recursive: true, force: true });

// Reading nothing is not passing.
d = mkdtempSync(join(tmpdir(), "arlen-claims-empty-"));
r = run(d);
check(
  "a tree with no profiles refuses rather than passing",
  r.code === 2 && /NOTHING WAS READ/.test(r.out),
  `exit=${r.code} out=${r.out}`,
);
rmSync(d, { recursive: true, force: true });

// A failing run must not END on the sentence a passing one prints. It did: the
// summary went to stdout before the findings went to stderr, so `tail` on a red
// CI log read "no description claims a directory its grants do not include"
// while the exit code said the opposite.
d = tree({
  x:
    "# Reads what the user points it at. It gets Pictures.\n" +
    '[info]\napp_id = "x"\ntier = "third-party"\n',
});
r = run(d);
check("a failing run is refused", r.code === 1, r.out);
check(
  "and does not also print the all-clear sentence",
  !/no description claims a directory/.test(r.out),
  r.out,
);
rmSync(d, { recursive: true, force: true });

for (const f of failures) console.error(`\n--- ${f.name}\n${f.detail}`);
if (failures.length) process.exit(1);
// The IMAGE corpus is read too, and this is the case that proves it: the same
// contradiction, planted under `dev/mkosi/.../permissions/`, must fail. The gate
// read only `sdk/permissions/profiles` until 22 August, so every hand-written
// profile the image ships - the ones with the longest descriptions and the most
// editing - was outside it.
{
  const root = mkdtempSync(join(tmpdir(), "claims-image-"));
  const dir = join(root, "dev/mkosi/mkosi.extra/var/lib/arlen/permissions/1000");
  mkdirSync(dir, { recursive: true });
  mkdirSync(join(root, "sdk/permissions/profiles"), { recursive: true });
  writeFileSync(
    join(dir, "probe.toml"),
    '# The probe gets Documents.\n\n[info]\napp_id = "probe"\ntier = "first-party"\n',
  );
  const r = run(root);
  check(
    "a claim in an image profile is caught, not only in the third-party corpus",
    r.code === 1 && r.out.includes("gets Documents"),
    `code ${r.code}: ${r.out}`,
  );
  rmSync(root, { recursive: true, force: true });
}

console.log("the stale claim is caught, and all four measured false positives stay false");
