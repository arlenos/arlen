// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-knowledge-socket-resolver.
//
// The case it was written from: nineteen clients resolving the knowledge socket
// from one env name, six months after the shared resolver was added for exactly
// that and applied to six of them.
//
// Run: node dev/scripts/test-check-knowledge-socket-resolver.mjs

import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-knowledge-socket-resolver.py");

const failures = [];

function check(name, files, expect) {
  const dir = mint("arlen-ksock-");
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  const got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  const ok = expect(got.code, got.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
  cleanup(dir);
}

console.log("check-knowledge-socket-resolver:");

check(
  "the shared resolver passes",
  {
    "apps/thing/src/lib.rs":
      "fn s() -> PathBuf { os_sdk::runtime::knowledge_socket_path() }\n",
  },
  (c) => c === 0,
);

check(
  "a client reading only the client name is a finding",
  {
    "apps/thing/src/lib.rs":
      'fn s() { os_sdk::runtime::socket_path("ARLEN_KNOWLEDGE_SOCKET", "knowledge.sock"); }\n',
  },
  (c, out) => c === 1 && out.includes("thing/src/lib.rs"),
);

// Both names, because the pair is the whole point: a client pinned to either one
// is broken under the launcher that sets the other.
check(
  "a client reading only the daemon name is a finding",
  {
    "apps/thing/src/lib.rs":
      'fn s() { runtime::socket_path("ARLEN_DAEMON_SOCKET", "knowledge.sock"); }\n',
  },
  (c, out) => c === 1 && out.includes("thing/src/lib.rs"),
);

// The daemon that owns the socket resolves its own bind, in its own crate.
check(
  "the knowledge daemon's own bind is exempt",
  {
    "daemons/knowledge/src/main.rs":
      'fn s() { crate::utils::socket_path("ARLEN_DAEMON_SOCKET", "knowledge.sock"); }\n',
  },
  (c) => c === 0,
);

// A different socket entirely is not this check's business.
check(
  "another socket's single-name resolver is not a finding",
  {
    "apps/thing/src/lib.rs":
      'fn s() { os_sdk::runtime::socket_path("ARLEN_MODULESD_SOCKET", "modulesd.sock"); }\n',
  },
  (c) => c === 0,
);

check(
  "a tree with no Rust sources refuses rather than passing",
  { "README.md": "nothing here\n" },
  (c, out) => c === 2 && out.includes("NOTHING WAS READ"),
);

if (failures.length) {
  console.log("");
  for (const f of failures) {
    console.log(`--- ${f.name} (exit ${f.code})`);
    console.log(f.out.trim());
  }
  process.exit(1);
}
console.log("one socket, two names, one resolver");
