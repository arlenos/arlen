// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// `event.proto` is vendored nine times, once per daemon that decodes Event Bus
// traffic. Protobuf forgives a copy that lags a field, because an absent field
// decodes to its default. It does not forgive two copies giving one field two
// numbers: that is a silent mis-decode across a socket, with no build error on
// either side, because the copies are only ever compiled separately.
//
// The gate has taken a tree argument since it was written, with a comment saying
// it was so it could be pointed at a fixture and shown to fail. Nobody ever did,
// and the third case here is what that cost: the field pattern could not spell a
// `map<K, V>` type, so `AppActionPayload.metadata`, `PresenceSetPayload.metadata`
// and `TimelineRecordPayload.metadata` were invisible - twenty-seven declarations
// across the nine copies, none of them held to anything. They agree on their
// numbers today (checked 12 Aug), so this was coverage rather than a live break,
// which is exactly the kind of hole only a control finds.
//
// Run: node dev/scripts/test-check-proto-drift.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-proto-drift.py");

const failures = [];

function check(name, files, expect) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-protodrift-"));
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  // The gate finds its files with `git ls-files`, which is how it respects
  // .gitignore and stays off the vendored mkosi builddir. So the fixture has to
  // be a real repo or the gate dies in subprocess and every case "fails" for a
  // reason that has nothing to do with what it is checking. Found the moment this
  // file first ran - the tree argument was documented as fixture-drivable since
  // the gate was written, and had never actually been driven.
  const git = (...a) => spawnSync("git", ["-C", dir, ...a], { encoding: "utf8" });
  git("init", "-q");
  git("config", "user.email", "t@example.invalid");
  git("config", "user.name", "t");
  git("add", "-A");
  // The gate also reaches OUTSIDE the tree for the compositor's vendored copy,
  // which lives in its own repo - that is deliberate, and it is where the one real
  // divergence in the tree's history actually sat. It also means a fixture is not
  // hermetic unless that reach is pointed somewhere empty, or these cases quietly
  // compare two fixture files against the real compositor's 85 fields. The
  // assertion on output rather than exit code alone is what caught that.
  const env = { ...process.env, COMPOSITOR_PATH: join(dir, "no-compositor-here") };
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8", env });
  const got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  const ok = expect(got.code, got.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
  rmSync(dir, { recursive: true, force: true });
}

const proto = (body) => `syntax = "proto3";\npackage arlen.event;\n\n${body}`;

check(
  "two copies agreeing on every shared field pass",
  {
    "daemons/a/proto/event.proto": proto(
      "message Event {\n  string id = 1;\n  int64 timestamp = 2;\n}\n",
    ),
    "daemons/b/proto/event.proto": proto(
      "message Event {\n  string id = 1;\n  int64 timestamp = 2;\n}\n",
    ),
  },
  (code, out) => code === 0 && out.includes("no disagreement"),
);

check(
  "one field with two numbers is the mis-decode this exists to catch",
  {
    "daemons/a/proto/event.proto": proto("message Event {\n  string id = 1;\n}\n"),
    "daemons/b/proto/event.proto": proto("message Event {\n  string id = 3;\n}\n"),
  },
  (code, out) => code === 1 && out.includes("id"),
);

// The case the missing control was hiding. A `map<K, V>` type has angle brackets,
// a comma and a space in it, so the `[\w.]+` the pattern used for a type could not
// match one. Without this, the check reports "no disagreement" over a set that
// silently excludes every map field in the tree.
check(
  "a map field is read like any other, so a disagreement on one is caught",
  {
    "daemons/a/proto/event.proto": proto(
      "message Event {\n  string id = 1;\n  map<string, string> metadata = 4;\n}\n",
    ),
    "daemons/b/proto/event.proto": proto(
      "message Event {\n  string id = 1;\n  map<string, string> metadata = 9;\n}\n",
    ),
  },
  (code, out) => code === 1 && out.includes("metadata"),
);

check(
  "and agreeing map fields still pass, so the fix did not just make it loud",
  {
    "daemons/a/proto/event.proto": proto(
      "message Event {\n  map<string, string> metadata = 4;\n}\n",
    ),
    "daemons/b/proto/event.proto": proto(
      "message Event {\n  map<string, string> metadata = 4;\n}\n",
    ),
  },
  // One field, not two: the count is distinct fields per message across the
  // copies, which is the thing being compared, not the number of declarations.
  (code, out) => code === 0 && out.includes("1 field"),
);

// Divergence by absence is legal and has to stay legal: it is what lets one copy
// lag a field like `cgroup_id` without failing the tree.
check(
  "a copy missing a field the other has is not drift",
  {
    "daemons/a/proto/event.proto": proto(
      "message Event {\n  string id = 1;\n  uint64 cgroup_id = 4;\n}\n",
    ),
    "daemons/b/proto/event.proto": proto("message Event {\n  string id = 1;\n}\n"),
  },
  (code, out) => code === 0 && out.includes("no disagreement"),
);

check(
  "and a number reused for a different field name is drift",
  {
    "daemons/a/proto/event.proto": proto("message Event {\n  string id = 2;\n}\n"),
    "daemons/b/proto/event.proto": proto("message Event {\n  string source = 2;\n}\n"),
  },
  (code, out) => code === 1 && out.includes("source"),
);

// A gate that narrows its subject has to say so. This one prints NOT CHECKED when
// the compositor checkout is absent, and that line is load-bearing: "not compared"
// and "no disagreement" are different claims, and reading the second as the first
// is how the compositor's `origin`/`session_id` split survived nine agreeing copies.
check(
  "an absent compositor checkout is reported, not silently dropped",
  {
    "daemons/a/proto/event.proto": proto("message Event {\n  string id = 1;\n}\n"),
    "daemons/b/proto/event.proto": proto("message Event {\n  string id = 1;\n}\n"),
  },
  (code, out) => code === 0 && out.includes("NOT CHECKED"),
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("all proto-drift cases passed");
