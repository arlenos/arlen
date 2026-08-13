// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The positive control for the boot-verify probe assertion: plant each way the
// graph can fail to ingest and watch the verdict refuse it.
//
// This is the half the directive asked for - "shown failing by pointing it at a
// boot with the watcher disabled". A real such image does not exist and would
// cost a build to make, but the assertion reads a journal, and a journal is text.
// Planting the defect in the text exercises the same code the boot does.

import { mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const VERDICT = join(ROOT, "dev/vm/probe_verdict.py");

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

function verdict(journal) {
  const dir = mkdtempSync(join(tmpdir(), "probe-verdict-"));
  const f = join(dir, "journal.log");
  writeFileSync(f, journal);
  const r = spawnSync("python3", [VERDICT, f], { encoding: "utf8" });
  rmSync(dir, { recursive: true, force: true });
  return { code: r.status, out: (r.stdout || "") + (r.stderr || "") };
}

// One round. `rows` answers the broad questions, `own` the one that names this
// run's own file - separately, because the gap between them is the whole point:
// a graph can hold plenty and still not have ingested what just happened.
const ROUND = (n, rows, own = rows) =>
  `kg-probe: round ${n} of 2\n` +
  ["timeline: file accesses", "projects: any", "files: any"]
    .map((q) => `kg-probe: ${q}: ${rows} row(s)`)
    .concat(`kg-probe: ingestion: this run's file: ${own} row(s)`)
    .join("\n");

// A good boot: asked twice, answered, and the graph held this run's own file.
{
  const j = `${ROUND(1, 0)}\n${ROUND(2, 1)}\nkg-probe: done, 0 question(s) failed\n`;
  const r = verdict(j);
  check("a boot whose graph ingested passes", r.code === 0);
  check("and it says how many questions found rows", r.out.includes("returned rows"));
  check("and it says the run's own file was among them", r.out.includes("own file"));
}

// The defect the older refusals cannot see: the graph is FULL, every question
// answered with rows, and none of them is the file this boot emitted. That is a
// desktop that came up with a populated disk and did not ingest anything - and
// until this refusal existed it read as the healthiest possible run.
{
  const j = `${ROUND(1, 3, 0)}\n${ROUND(2, 3, 0)}\nkg-probe: done, 0 question(s) failed\n`;
  const r = verdict(j);
  check("a full graph missing this run's own file is refused", r.code === 1);
  check("and the message names the ingestion question", r.out.includes("this run's file"));
}

// The defect the directive named: nothing ingests, so every question is answered
// and every answer is empty. `0 failed` would otherwise make this a green tick.
{
  const j = `${ROUND(1, 0)}\n${ROUND(2, 0)}\nkg-probe: done, 0 question(s) failed\n`;
  const r = verdict(j);
  check("an empty graph is refused despite `0 questions failed`", r.code === 1);
  check("and the message says the graph was empty", r.out.includes("graph was empty"));
}

// A run too short to reach the second round has no verdict at all, and silence
// must not read as success.
{
  const r = verdict(`${ROUND(1, 0)}\n`);
  check("a probe with no tally is refused", r.code === 1);
  check("and the message points at --linger", r.out.includes("--linger"));
}

// The probe's own count of questions it could not ask - the refusals that ran on
// every image before the profile was staged under the right uid.
{
  const j =
    "kg-probe: round 1 of 2\n" +
    "kg-probe: files: any: FAILED: read denied: label outside the caller's read scope\n" +
    "kg-probe: done, 1 question(s) failed\n";
  const r = verdict(j);
  check("reported failures are refused", r.code === 1);
  check("and the failing question is quoted", r.out.includes("read denied"));
}

// A journal with no probe in it at all is the "never ran" case, which is the
// no-tally refusal - not a pass.
{
  check("a journal with no probe lines is refused", verdict("some other boot\n").code === 1);
}

// When the verdict APPLIES at all. It used to apply only behind `--require-probe`,
// which nothing passed, so it was armed on no run - a refusal that cannot fire is
// worth the same as one that cannot exist. It is now decided from the artefact.
{
  const armed = (serial) =>
    spawnSync(
      "python3",
      ["-c",
       `import sys; sys.path.insert(0, ${JSON.stringify(join(ROOT, "dev/vm"))});\n` +
       "from probe_verdict import probe_is_shipped;\n" +
       "print(probe_is_shipped(sys.stdin.read()))"],
      { input: serial, encoding: "utf8" },
    ).stdout.trim() === "True";

  check(
    "an image whose probe unit starts is held to the probe",
    armed("[    4.5] systemd[1]: Starting arlen-kg-probe.service - Ask the graph...\n"),
  );
  check(
    "a probe that started and then said NOTHING still arms the refusal",
    armed("[    4.5] systemd[1]: Starting arlen-kg-probe.service\n[   9.0] other stuff\n"),
  );
  check(
    "a release image without the unit is not held to a probe it does not ship",
    !armed("[    4.5] systemd[1]: Starting arlen-graph.service\n"),
  );
}

console.log(failures ? `\n${failures} failure(s)` : "\nevery shape holds");
process.exit(failures ? 1 : 0);
