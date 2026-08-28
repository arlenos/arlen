// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for the inert-IP-firewall check.
//
// The red case is what the boot found: a user unit with `IPAddressDeny=any`,
// which systemd cannot apply without privileges and does not fail on. The green
// cases are the two honest alternatives - a seccomp address-family restriction,
// which does work unprivileged, and an explicit KNOWN entry naming what enforces
// the restriction instead.

import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const GATE = join(ROOT, "dev/scripts/check-user-unit-firewall.py");

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

function run(body, name = "arlen-thing.service") {
  const dir = mint("userfw-");
  const u = join(dir, "dev/mkosi/mkosi.extra/usr/lib/systemd/user");
  mkdirSync(u, { recursive: true });
  writeFileSync(join(u, name), body);
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  cleanup(dir);
  return { code: r.status, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
}

const BASE = "[Unit]\nDescription=x\n\n[Service]\nExecStart=/bin/true\n";

console.log("user unit IP firewall:");

{
  const r = run(`${BASE}IPAddressDeny=any\n`);
  check("IPAddressDeny in a user unit is caught", r.code === 1);
  check("and the message says why it cannot apply", r.out.includes("BPF"));
  check("and points at the working alternative", r.out.includes("RestrictAddressFamilies"));
}
{
  const r = run(`${BASE}IPAddressAllow=localhost\n`);
  check("IPAddressAllow is caught too", r.code === 1);
}
{
  const r = run(`${BASE}RestrictAddressFamilies=AF_UNIX\n`);
  check("the seccomp restriction passes", r.code === 0);
}
{
  // A commented-out directive is documentation, not configuration.
  const r = run(`${BASE}# IPAddressDeny=any is inert here, see the egress enforcer\n`);
  check("a commented directive is not the defect", r.code === 0);
}
{
  // The one unit that keeps it deliberately must pass under its real name.
  const r = run(`${BASE}IPAddressDeny=any\n`, "arlen-ai-proxy.service");
  check("a KNOWN unit is excused", r.code === 0);
}
{
  const dir = mint("userfw-empty-");
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  cleanup(dir);
  check("a tree with no user units refuses rather than passing", r.status === 2);
}

console.log(failures ? `\n${failures} failure(s)` : "\nboth directions hold");
process.exit(failures ? 1 : 0);
