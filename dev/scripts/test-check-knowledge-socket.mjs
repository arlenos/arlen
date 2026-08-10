// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// What the knowledge-socket gate must catch, and what it must leave alone.
//
// The gate is two greps ANDed together, which is exactly the kind of check that
// looks obviously right and quietly matches nothing. So each shape it claims to
// catch is built here and shown to fail it, and each shape it must stay quiet on
// is built too - the second half is the one that decides whether a gate is
// usable, because a check that flags the daemon binding its own socket teaches
// people to add exclusions.
//
// Run: node dev/scripts/test-check-knowledge-socket.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { execFileSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-knowledge-socket.py");

const failures = [];

function tree(files) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-ksock-gate-"));
  for (const [rel, body] of Object.entries(files)) {
    const abs = join(dir, rel);
    mkdirSync(dirname(abs), { recursive: true });
    writeFileSync(abs, body);
  }
  return dir;
}

function run(dir) {
  try {
    return { code: 0, out: execFileSync("python3", [GATE, dir], { encoding: "utf8" }) };
  } catch (e) {
    return { code: e.status ?? 1, out: `${e.stdout ?? ""}${e.stderr ?? ""}` };
  }
}

function check(name, dir, expect) {
  const { code, out } = run(dir);
  const ok = expect(code, out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, code, out });
  rmSync(dir, { recursive: true, force: true });
}

// The shape that shipped: bind variable only, then XDG.
const BROKEN = `
fn knowledge_socket() -> String {
    if let Ok(p) = std::env::var("ARLEN_DAEMON_SOCKET") { return p; }
    if let Ok(x) = std::env::var("XDG_RUNTIME_DIR") { return format!("{x}/arlen/knowledge.sock"); }
    "/run/arlen/knowledge.sock".into()
}
`;

const FIXED = BROKEN.replace(
  'std::env::var("ARLEN_DAEMON_SOCKET")',
  'std::env::var("ARLEN_KNOWLEDGE_SOCKET")',
);

const VIA_SDK = `
fn knowledge_socket() -> String {
    // XDG named only in the doc of the helper we call.
    // XDG_RUNTIME_DIR
    os_sdk::runtime::knowledge_socket_path().to_string_lossy().into_owned()
}
`;

check(
  "a resolver that reads only the bind variable fails",
  tree({ "apps/thing/src/main.rs": BROKEN }),
  (code, out) => code !== 0 && out.includes("apps/thing/src/main.rs"),
);

check(
  "reading the client variable passes",
  tree({ "apps/thing/src/main.rs": FIXED }),
  (code) => code === 0,
);

check(
  "calling the SDK helper passes",
  tree({ "apps/thing/src/main.rs": VIA_SDK }),
  (code) => code === 0,
);

check(
  "the daemon that binds the socket is not a client",
  // The reason the gate needs a scope at all: the daemon legitimately resolves
  // its own bind path from ARLEN_DAEMON_SOCKET, and flagging it would make the
  // gate wrong on its first run.
  tree({ "daemons/knowledge/src/main.rs": BROKEN }),
  (code) => code === 0,
);

check(
  "a file that only names the path in prose is not a resolver",
  // Without the XDG half of the rule this would be flagged, and then every doc
  // comment mentioning the socket would need an exclusion.
  tree({
    "apps/thing/src/doc.rs":
      '//! Connects to /run/arlen/knowledge.sock when the daemon is up.\n',
  }),
  (code) => code === 0,
);

check(
  "a resolver for some other socket is not this gate's business",
  // XDG plus a socket, but not the knowledge one: out of scope by construction
  // rather than by exclusion.
  tree({
    "apps/thing/src/other.rs":
      'fn s() -> String { let x = std::env::var("XDG_RUNTIME_DIR").unwrap(); format!("{x}/arlen/notification.sock") }\n',
  }),
  (code) => code === 0,
);

check(
  "asking the SDK under the bind name is the same bug, spelled differently",
  // The shape the first version of this gate could not see: no XDG_RUNTIME_DIR
  // in the file at all, because the fallback lives inside the helper. Two of the
  // seven resolvers were written this way.
  tree({
    "apps/thing/src/sdk_call.rs":
      'fn s() -> String { os_sdk::runtime::socket_path("ARLEN_DAEMON_SOCKET", "knowledge.sock").to_string_lossy().into_owned() }\n',
  }),
  (code, out) => code !== 0 && out.includes("apps/thing/src/sdk_call.rs"),
);

console.log(failures.length ? "\nsome cases regressed" : "\nevery shape holds");
process.exit(failures.length ? 1 : 0);
