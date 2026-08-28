// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-mime-claims-decodable.
//
// Both cases are real ones from 26 August: the entry claimed `image/x-raw`,
// whose decoder is "a later slice", and it claimed `image/x-tga`, which the
// const had missed - so the runtime handler registration never covered a format
// the app opens fine. The gate found the second one within a minute of existing,
// after I had eyeballed the same two lists and missed it.
//
// Run: node dev/scripts/test-check-mime-claims-decodable.mjs

import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-mime-claims-decodable.py");
const failures = [];

function run(name, files, expect) {
  const dir = mint("arlen-mime-");
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  const out = `${r.stdout ?? ""}${r.stderr ?? ""}`;
  const ok = expect(r.status ?? 1, out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, code: r.status, out });
  cleanup(dir);
}

const SRC = (mimes) => `
pub const IMAGE_MIMES: &[&str] = &[
${mimes.map((m) => `    "${m}",`).join("\n")}
];

pub const AUDIO_MIMES: &[&str] = &["audio/flac"];
`;

const ENTRY = (mimes) =>
  `[Desktop Entry]\nType=Application\nName=Viewers\n# MimeType=image/never-read-from-a-comment\nMimeType=${mimes.join(";")};\n`;

const P_ENTRY = "apps/viewers/dist/arlen-viewers.desktop";
const P_SRC = "apps/viewers/core/src/lib.rs";

run(
  "a claimed type the core cannot decode is caught",
  {
    [P_SRC]: SRC(["image/png"]),
    [P_ENTRY]: ENTRY(["image/png", "image/x-raw", "audio/flac"]),
  },
  (code, out) => code === 1 && out.includes("image/x-raw"),
);

// The other direction, and the one that is easy to forget: a format the app
// decodes and the entry omits is never OFFERED, so the app can open it and no
// file manager ever sends it one.
run(
  "a decodable type the entry omits is caught",
  {
    [P_SRC]: SRC(["image/png", "image/x-tga"]),
    [P_ENTRY]: ENTRY(["image/png", "audio/flac"]),
  },
  (code, out) => code === 1 && out.includes("image/x-tga"),
);

run(
  "agreement passes",
  {
    [P_SRC]: SRC(["image/png", "image/x-tga"]),
    [P_ENTRY]: ENTRY(["image/png", "image/x-tga", "audio/flac"]),
  },
  (code) => code === 0,
);

// The entry's MimeType comment NAMES the types it decided against - `image/*`,
// the video types, `image/x-raw` and the condition for putting it back. Reading
// a comment as a claim would report the reasoning as the defect.
run(
  "a type named in a comment is reasoning, not a claim",
  {
    [P_SRC]: SRC(["image/png"]),
    [P_ENTRY]:
      "[Desktop Entry]\nType=Application\n# no image/svg+xml here: this app cannot open one\nMimeType=image/png;audio/flac;\n",
  },
  (code) => code === 0,
);

// A renamed const must stop the check rather than compare against nothing: an
// empty capability set would report every claim as unkeepable, which reads as a
// broken app instead of a broken check.
run(
  "a renamed const refuses rather than comparing against nothing",
  {
    [P_SRC]: "pub const OTHER_NAME: &[&str] = &[\"image/png\"];\n",
    [P_ENTRY]: ENTRY(["image/png"]),
  },
  (code, out) => code === 2 && out.includes("NOTHING WAS READ"),
);

run(
  "a missing entry refuses rather than passing",
  { [P_SRC]: SRC(["image/png"]) },
  (code, out) => code === 2 && out.includes("NOTHING WAS READ"),
);

for (const f of failures) console.error(`\n--- ${f.name}\nexit=${f.code}\n${f.out}`);
if (failures.length) process.exit(1);
console.log("a desktop entry may claim exactly what its app can decode, in both directions");
