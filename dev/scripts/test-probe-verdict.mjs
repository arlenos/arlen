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

// What the second reader prints: the event store, opened directly rather than
// asked of the daemon. The verdict now requires it, so every passing fixture
// carries it and the refusals below vary it on purpose.
const STORE = (n) => `kg-probe: store: ${n} event row(s) naming this run's file`;

// The two readings of an empty ingestion answer, separated 18 August. With the
// kernel sensor forwarding, a boot's own event routinely sits in the store while
// promotion works through ~10000 others, so "the desktop did not do its job" was
// pointing at a queue. Both still refuse the run; they differ in where they send
// the reader.
{
  const backlog =
    `${ROUND(1, 0)}\n${ROUND(2, 3, 0)}\n${STORE(27)}\nkg-probe: done, 0 question(s) failed\n`;
  const r = verdict(backlog);
  check("an empty graph answer WITH the event in the store blames the backlog", r.code === 1);
  check("and says the ingestion path is not broken", r.out.includes("not broken"));
  check("and does not blame the desktop", !r.out.includes("did not do its job"));
}

{
  const broken =
    `${ROUND(1, 0)}\n${ROUND(2, 3, 0)}\n${STORE(0)}\nkg-probe: done, 0 question(s) failed\n`;
  const r = verdict(broken);
  check("an empty answer with an empty store still blames the writer", r.code === 1);
  check("and says so", r.out.includes("did not do its job"));
}

// The two lines the probe prints when it POLLS for the ingestion answer instead
// of asking once, added 18 August with the poll itself. The risk is not the
// waiting, it is that this verdict greps for a shape and a new one it cannot read
// turns a working ingestion into a silent failure - which is the fault this whole
// file exists to prevent, aimed at itself.
{
  const won =
    `${ROUND(1, 0)}\n${ROUND(2, 1, 0)}\n` +
    `kg-probe: ingestion: this run's file: 3 row(s) after 12s of waiting\n` +
    `${STORE(1)}\nkg-probe: done, 0 question(s) failed\n`;
  const r = verdict(won);
  check("a polled ingestion answer is read as an answer", r.code === 0);
  check("and it still counts as the run's own file", r.out.includes("own file"));
}

{
  const lost =
    `${ROUND(1, 0)}\n${ROUND(2, 1, 0)}\n` +
    `kg-probe: ingestion: this run's file: still 0 row(s) after 90s of waiting\n` +
    `${STORE(1)}\nkg-probe: done, 0 question(s) failed\n`;
  const r = verdict(lost);
  check("a polled timeout is still a refusal", r.code === 1);
  // Specifically the ingestion sentence, not the generic "reported failures". The
  // probe deliberately does not raise its own failure count for this, because
  // doing so short-circuits the verdict into a message that says less.
  //
  // This fixture carries STORE(1) - the event IS in the store - so the accurate
  // reading is the backlog one, not "the writer dropped it". The assertion said
  // the latter until 18 August, which made it a control pinning a sentence that
  // was wrong for its own fixture.
  check("and the message reads it as a backlog, which is what this fixture is",
        r.out.includes("not broken"));
}

// A good boot: asked twice, answered, the graph held this run's own file, and the
// store agrees that the event behind it arrived.
{
  const j = `${ROUND(1, 0)}\n${ROUND(2, 1)}\n${STORE(1)}\nkg-probe: done, 0 question(s) failed\n`;
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

// The probe's third state, added 15 Aug: it could not be named, so it asked the
// graph nothing. Zero rows and zero failures - identical to the empty-graph case
// from the outside, and the opposite finding. Without this the run would report a
// broken ingestion path on a system whose ingestion is fine.
// A probe with no grants recorded is NOT a special verdict any more. It used to
// be: the probe inferred "identity: NOT RESOLVED" from empty grants, skipped every
// question, and the verdict handed Tim a fork between changing shipped release
// code and dropping the coverage. Empty grants mean no Grant node has been written
// yet - the binary route names the probe fine - so the inference was wrong and the
// fork was invented.
//
// What must hold now is that a run which reports no grants and then answers its
// questions is judged on the ANSWERS, like any other run.
{
  const j =
    "kg-probe: grants: none recorded for this caller yet.\n" +
    `${ROUND(1, 3)}\n${ROUND(2, 3)}\n${STORE(1)}\n` +
    "kg-probe: done, 0 question(s) failed\n";
  const r = verdict(j);
  check("no grants plus real answers is a pass", r.code === 0);
}
{
  // And a run whose questions were all refused still fails, through the ordinary
  // path rather than a branch about identity.
  const j =
    "kg-probe: grants: none recorded for this caller yet.\n" +
    `${ROUND(1, 0)}\n${ROUND(2, 0)}\n` +
    "kg-probe: done, 0 question(s) failed\n";
  const r = verdict(j);
  check("no grants plus no rows is still refused", r.code === 1);
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

// The three refusals the second reader makes possible. Each one is a boot the old
// verdict passed: the graph answered, the tally was clean, and nothing had looked
// at whether the event behind the node ever reached the store.
{
  const j = `${ROUND(1, 1)}\nkg-probe: done, 0 question(s) failed\n`;
  const r = verdict(j);
  check("a graph claim with no store reading is refused", r.code === 1);
  check("and it says the claim rests on the daemon alone", r.out.includes("about itself"));
}

{
  const j = `${ROUND(1, 1)}\nkg-probe: store: UNREADABLE: unable to open database file\nkg-probe: done, 0 question(s) failed\n`;
  const r = verdict(j);
  check("an unreadable store is refused", r.code === 1);
  check("and the reason is quoted", r.out.includes("UNREADABLE"));
}

{
  const j = `${ROUND(1, 1)}\n${STORE(0)}\nkg-probe: done, 0 question(s) failed\n`;
  const r = verdict(j);
  check("two readers disagreeing is refused", r.code === 1);
  check("and the message says they disagree", r.out.includes("disagree"));
}

console.log(failures ? `\n${failures} failure(s)` : "\nevery shape holds");
process.exit(failures ? 1 : 0);
