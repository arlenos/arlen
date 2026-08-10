// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// What the peer-identity gate must catch, and what it must leave alone.
//
// The rule it encodes was measured, not reasoned: every systemd directive that
// gives a unit its own mount namespace denies `/proc/<pid>/exe` on another
// process, and `RestrictNamespaces` - which does not create one - does not. That
// control is a case here rather than only a sentence in the docstring, because it
// is the difference between a rule and a superstition about sandboxing.
//
// Run: node dev/scripts/test-check-peer-identity-sandbox.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-peer-identity-sandbox.py");

const failures = [];

function tree(files) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-peerid-gate-"));
  for (const [rel, body] of Object.entries(files)) {
    const abs = join(dir, rel);
    mkdirSync(dirname(abs), { recursive: true });
    writeFileSync(abs, body);
  }
  return dir;
}

function run(dir) {
  // Both streams: a case asserting on stdout alone passes for the wrong reason
  // whenever the gate writes to stderr. That has bitten the sibling gate tests.
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  return { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
}

function check(name, dir, expect) {
  const { code, out } = run(dir);
  const ok = expect(code, out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, code, out });
  rmSync(dir, { recursive: true, force: true });
}

const U = "dev/mkosi/mkosi.extra/usr/lib/systemd/user";

const unit = (extra) => `[Unit]
Description=A daemon

[Service]
ExecStart=/usr/lib/arlen/libexec/arlen-thing
${extra}

[Install]
WantedBy=default.target
`;

const CARGO = `[package]
name = "arlen-thing"
version = "0.1.0"
`;

const RESOLVES = `
pub fn who(pid: u32) -> Option<String> {
    arlen_permissions::identity::app_id_from_pid(pid).ok()
}
`;

const PLAIN = `
pub fn add(a: u32, b: u32) -> u32 { a + b }
`;

console.log("check-peer-identity-sandbox:");

check(
  "a sandboxed unit whose daemon resolves peers is caught",
  tree({
    [`${U}/arlen-thing.service`]: unit("ProtectSystem=strict"),
    "daemons/thing/Cargo.toml": CARGO,
    "daemons/thing/src/lib.rs": RESOLVES,
  }),
  (code, out) => code === 1 && out.includes("mount namespace"),
);

check(
  "a sandboxed unit that does not resolve peers is left alone",
  tree({
    [`${U}/arlen-thing.service`]: unit("ProtectSystem=strict"),
    "daemons/thing/Cargo.toml": CARGO,
    "daemons/thing/src/lib.rs": PLAIN,
  }),
  (code) => code === 0,
);

check(
  "an unsandboxed unit that resolves peers is left alone",
  tree({
    [`${U}/arlen-thing.service`]: unit("Restart=on-failure"),
    "daemons/thing/Cargo.toml": CARGO,
    "daemons/thing/src/lib.rs": RESOLVES,
  }),
  (code) => code === 0,
);

check(
  "an unsandboxed peer-resolver is reported as paying for /proc identity",
  tree({
    [`${U}/arlen-thing.service`]: unit("Restart=on-failure"),
    "daemons/thing/Cargo.toml": CARGO,
    "daemons/thing/src/lib.rs": RESOLVES,
  }),
  (code, out) => code === 0 && out.includes("PAYING THE OTHER HALF"),
);

// The same unit once it is on the stamped resolver. It looks identical to the
// case above from the outside - no sandbox, resolves peers - but it is not
// paying for anything any more, and telling the two apart is the whole point:
// one is a trade being carried, the other is a sandbox nobody has collected.
check(
  "a peer-resolver already on enforce is reported as recoverable, not paying",
  tree({
    [`${U}/arlen-thing.service`]: unit("Restart=on-failure\nEnvironment=ARLEN_STAMPED_IDENTITY=enforce"),
    "daemons/thing/Cargo.toml": CARGO,
    "daemons/thing/src/lib.rs": RESOLVES,
  }),
  (code, out) =>
    code === 0 && out.includes("HARDENING RECOVERABLE") && !out.includes("PAYING THE OTHER HALF"),
);

// The measured control: RestrictNamespaces restricts what a unit may CREATE and
// does not put it in a mount namespace, so the exe read still works and flagging
// it would be a false positive that teaches people to ignore this gate.
check(
  "RestrictNamespaces alone is not treated as a sandbox",
  tree({
    [`${U}/arlen-thing.service`]: unit("RestrictNamespaces=yes"),
    "daemons/thing/Cargo.toml": CARGO,
    "daemons/thing/src/lib.rs": RESOLVES,
  }),
  (code) => code === 0,
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("all peer-identity gate cases passed");
