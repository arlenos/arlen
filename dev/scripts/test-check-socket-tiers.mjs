// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for the socket-tier check: watch it fail on the shape it claims to
// catch, and pass on the shapes it must not.
//
// Fixture trees, not the repo, so it keeps working as resolvers come and go. The
// repo is asked one thing at the end: that the scan reads it at all.

import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const CHECK = join(ROOT, "dev/scripts/check-socket-tiers.py");

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

function tree(source, at = "sdk/thing/src/lib.rs") {
  const root = mint("socket-tiers-");
  mkdirSync(dirname(join(root, at)), { recursive: true });
  writeFileSync(join(root, at), source);
  return root;
}

function run(root) {
  const r = spawnSync("python3", [CHECK, root], { encoding: "utf8" });
  return { code: r.status, out: (r.stdout || "") + (r.stderr || "") };
}

// The shape the shell plugin had: a pin, then straight to /run.
{
  const root = tree(`fn producer() -> String {
    std::env::var("ARLEN_PRODUCER_SOCKET")
        .unwrap_or_else(|_| "/run/arlen/event-bus-producer.sock".to_string())
}
`);
  const { code, out } = run(root);
  check(
    "a pin falling straight to /run fails, and the line is named",
    code === 1 && out.includes("sdk/thing/src/lib.rs:1"),
  );
  cleanup(root);
}

// The same resolver with the tier it was missing.
{
  const root = tree(`fn producer() -> String {
    if let Ok(p) = std::env::var("ARLEN_PRODUCER_SOCKET") {
        return p;
    }
    if let Some(d) = std::env::var("XDG_RUNTIME_DIR").ok() {
        return format!("{d}/arlen/event-bus-producer.sock");
    }
    "/run/arlen/event-bus-producer.sock".to_string()
}
`);
  check("the three-tier resolver passes", run(root).code === 0);
  cleanup(root);
}

// A system daemon naming its own socket is not a resolver and is left alone:
// no env pin, so nothing here is falling through anything.
{
  const root = tree(`const SOCKET: &str = "/run/arlen/ai-engine.sock";

fn bind() -> &'static str {
    SOCKET
}
`);
  check("a system path with no pin is not a resolver", run(root).code === 0);
  cleanup(root);
}

// Delegating to the shared resolver carries no literal, so it never trips.
{
  const root = tree(`fn producer() -> String {
    os_sdk::runtime::socket_path("ARLEN_PRODUCER_SOCKET", "event-bus-producer.sock")
        .to_string_lossy()
        .into_owned()
}
`);
  check("delegating to the shared resolver passes", run(root).code === 0);
  cleanup(root);
}

// A comment ABOUT the fallback is prose, not the fallback. Several of the correct
// resolvers explain themselves that way and must not read as the thing they warn
// about.
{
  const root = tree(`fn producer() -> String {
    // Falls back to "/run/arlen/event-bus-producer.sock" when there is no
    // per-user runtime dir, which is the system case.
    std::env::var("ARLEN_PRODUCER_SOCKET").unwrap_or_default()
}
`);
  check("a comment naming the path is not the path", run(root).code === 0);
  cleanup(root);
}

// Tests pin literal paths on purpose: they are asserting the fallback.
{
  const root = tree(`fn producer() -> String {
    String::new()
}

#[cfg(test)]
mod tests {
    #[test]
    fn falls_back() {
        let _ = std::env::var("ARLEN_PRODUCER_SOCKET");
        assert_eq!(super::producer(), "/run/arlen/event-bus-producer.sock");
    }
}
`);
  check("a test asserting the fallback is not a finding", run(root).code === 0);
  cleanup(root);
}

{
  const r = run(ROOT);
  check(
    "and the repo itself passes",
    r.code === 0 && /socket resolver\(s\), each with the per-user tier/.test(r.out),
  );
}

console.log(failures === 0 ? "all ok" : `${failures} failure(s)`);
process.exit(failures === 0 ? 0 : 1);
