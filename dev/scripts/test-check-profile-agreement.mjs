// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-profile-agreement.
//
// The planted defect is the real one: two packaging ids for one program that
// hand out different things, which is how `ghostty` ended up with no network
// while `com.ghostty.Ghostty` had it, and how `clamtk` briefly held the whole
// home tree while `com.gitlab.davem.ClamTk` held nothing.
//
// Run: node dev/scripts/test-check-profile-agreement.mjs

import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-profile-agreement.py");
const failures = [];

function tree(files) {
  const dir = mint("arlen-agree-");
  mkdirSync(join(dir, "sdk/permissions/profiles"), { recursive: true });
  mkdirSync(join(dir, "dev"), { recursive: true });
  for (const [name, body] of Object.entries(files)) {
    writeFileSync(join(dir, "sdk/permissions/profiles", `${name}.toml`), body);
  }
  return dir;
}

const prof = (id, fs, net) =>
  `# test\n[info]\napp_id = "${id}"\ntier = "third-party"\n` +
  (fs.length ? `\n[filesystem]\n${fs.map((k) => `${k} = true\n`).join("")}` : "") +
  (net ? `\n[network]\nallow_all = true\n` : "");

function run(dir, extra = []) {
  const r = spawnSync("python3", [GATE, dir, ...extra], { encoding: "utf8" });
  return { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
}

function check(name, ok, detail) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, detail });
}

console.log("check-profile-agreement:");

// Two ids, one program, different grants.
let dir = tree({
  ghostty: prof("ghostty", ["home"], false),
  "com.ghostty.Ghostty": prof("com.ghostty.Ghostty", ["home"], true),
});
let r = run(dir);
check(
  "two packaging ids that grant different things are reported",
  r.code === 1 && /ghostty/.test(r.out) && /net=True/.test(r.out),
  `exit=${r.code} out=${r.out}`,
);

// Recording it is how the pre-existing ones stay out of the way.
run(dir, ["--update"]);
r = run(dir);
check("a recorded disagreement no longer fails", r.code === 0, `exit=${r.code} out=${r.out}`);
cleanup(dir);

// Agreement is the passing case.
dir = tree({
  konsole: prof("konsole", ["home"], false),
  "org.kde.konsole": prof("org.kde.konsole", ["home"], false),
});
r = run(dir);
check("ids that agree are not reported", r.code === 0, `exit=${r.code} out=${r.out}`);
cleanup(dir);

// A category word is not an app name. Without this the three `*.Client` ids
// (Dropbox, Skype, Spotify) read as one program disagreeing with itself.
// `kitty` is here so the tree has something readable: a fixture of ONLY
// generic-named ids reads nothing at all, and the gate is right to refuse that
// rather than call it a pass. (This test first asserted exit 0 on such a tree
// and failed, which is the gate catching the test.)
dir = tree({
  "com.dropbox.Client": prof("com.dropbox.Client", ["documents"], true),
  "com.spotify.Client": prof("com.spotify.Client", [], true),
  kitty: prof("kitty", ["home"], false),
});
r = run(dir);
check(
  "unrelated apps sharing a generic last segment are not grouped",
  r.code === 0 && !/Client/.test(r.out),
  `exit=${r.code} out=${r.out}`,
);
cleanup(dir);

// A distro build carries a desktop-environment prefix the upstream id does not.
// Obfuscate held the whole home tree as `gnome-obfuscate` and Pictures as
// `com.belmoussaoui.Obfuscate` for as long as both existed, because nothing ever
// compared them.
dir = tree({
  "gnome-obfuscate": prof("gnome-obfuscate", ["home"], false),
  "com.belmoussaoui.Obfuscate": prof("com.belmoussaoui.Obfuscate", ["pictures"], false),
});
r = run(dir);
check(
  "a vendor-prefixed distro id is compared with its upstream sibling",
  r.code === 1 && /obfuscate/i.test(r.out),
  `exit=${r.code} out=${r.out}`,
);
cleanup(dir);

// But stripping that prefix must not turn every terminal into one program:
// `gnome-terminal` becomes `terminal`, which is a category, not an app.
dir = tree({
  "gnome-terminal": prof("gnome-terminal", ["home"], false),
  "xfce4-terminal": prof("xfce4-terminal", [], true),
  kitty: prof("kitty", ["home"], false),
});
r = run(dir);
check(
  "a vendor prefix over a category word does not group unrelated apps",
  r.code === 0 && !/terminal/.test(r.out),
  `exit=${r.code} out=${r.out}`,
);
cleanup(dir);

// Two desktops each ship a calculator, and they are not the same program. The
// last segment is the plain word for what the app IS, which is the same trap as
// the category words above wearing an app's clothes - and here the fix it would
// have invited is making GNOME's calculator match elementary's.
dir = tree({
  "org.gnome.Calculator": prof("org.gnome.Calculator", ["documents"], false),
  "io.elementary.calculator": prof("io.elementary.calculator", [], false),
  "org.gnome.Calendar": prof("org.gnome.Calendar", ["documents"], true),
  "io.elementary.calendar": prof("io.elementary.calendar", [], false),
  // One id whose last segment is a real app name, so the read is not empty:
  // every id above is stripped to nothing, and a tree the gate could read
  // nothing from refuses rather than passing.
  kitty: prof("kitty", ["home"], false),
});
r = run(dir);
check(
  "one desktop's calculator is not another's",
  r.code === 0,
  `exit=${r.code} out=${r.out}`,
);
cleanup(dir);

// A release channel is not an app name. Taking the last segment made Spotify,
// Chromium, Thunderbird and GNOME Snapshot one program, because all four ids end
// in `.snapshot` - a group that would have told somebody to give four unrelated
// applications the same grants.
dir = tree({
  "org.chromium.Chromium.snapshot": prof("org.chromium.Chromium.snapshot", ["downloads"], true),
  "com.spotify.Client.snapshot": prof("com.spotify.Client.snapshot", ["music"], true),
  "org.gnome.Snapshot": prof("org.gnome.Snapshot", ["pictures"], false),
});
r = run(dir);
check(
  "a release-channel segment does not make unrelated apps one program",
  r.code === 0,
  `exit=${r.code} out=${r.out}`,
);
cleanup(dir);

// The channel still resolves to its own program, so a real disagreement between
// a stable id and its snapshot channel is still found.
dir = tree({
  "org.chromium.Chromium": prof("org.chromium.Chromium", ["downloads"], true),
  "org.chromium.Chromium.snapshot": prof("org.chromium.Chromium.snapshot", ["home"], true),
});
r = run(dir);
check(
  "a channel id still groups with its own stable id",
  r.code === 1 && /chromium/i.test(r.out),
  `exit=${r.code} out=${r.out}`,
);
cleanup(dir);

// Reading nothing is not passing.
dir = mint("arlen-agree-empty-");
r = run(dir);
check(
  "a tree with no profiles refuses rather than passing",
  r.code === 2 && /NOTHING WAS READ/.test(r.out),
  `exit=${r.code} out=${r.out}`,
);
cleanup(dir);

for (const f of failures) console.error(`\n--- ${f.name}\n${f.detail}`);
if (failures.length) process.exit(1);
console.log("a disagreement is found, a recorded one is not, and an empty read refuses");
