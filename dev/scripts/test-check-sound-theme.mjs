// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-sound-theme.
//
// This was the one check in dev/scripts that no control drove, found on 26
// August by asking which checks nothing spawns. The gate exists because a build
// step reported success while shipping six cues eleven decibels apart; a gate
// with nothing behind it is the same shape one level up.
//
// The level cases need ffmpeg, which the gate itself requires - without it the
// gate SKIPS, and that path is asserted rather than silently passed over.
//
// Run: node dev/scripts/test-check-sound-theme.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-sound-theme.py");
const THEME = "themes/sounds/arlen/stereo";
const SOUND_RS = "daemons/notification-daemon/src/sound.rs";
const failures = [];
const haveFfmpeg = spawnSync("ffmpeg", ["-version"], { encoding: "utf8" }).status === 0;

/// A `sound_name` body the gate can parse, mapping each given cue.
const soundRs = (names) => `
impl SoundEvent {
    fn sound_name(self) -> &'static str {
        match self {
${names.map((n) => `            SoundEvent::X => "${n}",`).join("\n")}
        }
    }
}
`;

/// One real .oga at a chosen amplitude, so the level maths has something to read.
function cue(path, amplitude) {
  mkdirSync(dirname(path), { recursive: true });
  const r = spawnSync(
    "ffmpeg",
    ["-v", "error", "-y", "-f", "lavfi", "-i",
     `sine=frequency=440:duration=0.2:sample_rate=48000`,
     "-af", `volume=${amplitude}`, "-c:a", "libvorbis", path],
    { encoding: "utf8" }
  );
  return r.status === 0;
}

/// Without ffmpeg the gate measures nothing and says so. Asserting THAT is the
/// point: `!haveFfmpeg || ...` reads as "covered" while asserting nothing, and a
/// case guarded that way passed here for a day while failing on CI, where ffmpeg
/// is absent and the fixture it never built was the thing under test.
const skipped = (code, out) => code === 0 && out.includes("SKIPPED");

function run(name, build, expect) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-sound-"));
  build(dir);
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  const out = `${r.stdout ?? ""}${r.stderr ?? ""}`;
  const ok = expect(r.status ?? 1, out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, code: r.status, out });
  rmSync(dir, { recursive: true, force: true });
}

console.log("sound theme:");

run(
  "a tree with no cue directory reports that it read nothing",
  (dir) => {
    mkdirSync(join(dir, dirname(SOUND_RS)), { recursive: true });
    writeFileSync(join(dir, SOUND_RS), soundRs(["message-new"]));
  },
  (code, out) => code === 2 && out.includes("NOTHING WAS READ"),
);

run(
  "a sound.rs the gate can find no cue name in reports that it read nothing",
  (dir) => {
    mkdirSync(join(dir, THEME), { recursive: true });
    mkdirSync(join(dir, dirname(SOUND_RS)), { recursive: true });
    writeFileSync(
      join(dir, SOUND_RS),
      "impl SoundEvent {\n    fn sound_name(self) -> &'static str {\n        match self {\n        }\n    }\n}\n"
    );
  },
  (code, out) =>
    haveFfmpeg ? code === 2 && out.includes("NOTHING WAS READ") : skipped(code, out),
);

run(
  "a cue the daemon resolves with no file is caught",
  (dir) => {
    mkdirSync(join(dir, THEME), { recursive: true });
    mkdirSync(join(dir, dirname(SOUND_RS)), { recursive: true });
    writeFileSync(join(dir, SOUND_RS), soundRs(["message-new"]));
  },
  (code, out) =>
    haveFfmpeg
      ? code === 1 && out.includes("falls through to the synth")
      : skipped(code, out),
);

run(
  "a file no event resolves is caught",
  (dir) => {
    mkdirSync(join(dir, THEME), { recursive: true });
    mkdirSync(join(dir, dirname(SOUND_RS)), { recursive: true });
    writeFileSync(join(dir, SOUND_RS), soundRs(["message-new"]));
    if (haveFfmpeg) {
      cue(join(dir, THEME, "message-new.oga"), 1.0);
      cue(join(dir, THEME, "orphan.oga"), 1.0);
    }
  },
  (code, out) =>
    haveFfmpeg ? code === 1 && out.includes("never plays") : skipped(code, out),
);

run(
  "cues more than the allowed spread apart are caught",
  (dir) => {
    mkdirSync(join(dir, THEME), { recursive: true });
    mkdirSync(join(dir, dirname(SOUND_RS)), { recursive: true });
    writeFileSync(join(dir, SOUND_RS), soundRs(["loud", "quiet"]));
    if (haveFfmpeg) {
      cue(join(dir, THEME, "loud.oga"), 0.5);
      cue(join(dir, THEME, "quiet.oga"), 0.02);
    }
  },
  (code, out) =>
    haveFfmpeg ? code === 1 && out.includes("dB apart") : skipped(code, out),
);

run(
  "a matched, level set passes",
  (dir) => {
    mkdirSync(join(dir, THEME), { recursive: true });
    mkdirSync(join(dir, dirname(SOUND_RS)), { recursive: true });
    writeFileSync(join(dir, SOUND_RS), soundRs(["a", "b"]));
    if (haveFfmpeg) {
      cue(join(dir, THEME, "a.oga"), 0.3);
      cue(join(dir, THEME, "b.oga"), 0.3);
    }
  },
  (code, out) => (haveFfmpeg ? code === 0 : skipped(code, out)),
);

if (!haveFfmpeg) {
  console.log("  note ffmpeg is absent, so only the skip path was exercised");
}
if (failures.length) {
  console.log(`\n${failures.length} case(s) failed`);
  for (const f of failures) console.log(`\n--- ${f.name}\n${f.out}`);
  process.exit(1);
}
console.log("a missing cue, an orphan file and an uneven set are all caught");
