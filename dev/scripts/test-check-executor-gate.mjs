// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// Every executor that acts must read `executor_live`, and this check has to find
// the ones that do not WHEREVER they are written.
//
// The placement case is the reason this file exists. The gate used to cut each
// file at the first `#[cfg(test)]` and scan only what came before, so an executor
// added below the test module - where Rust convention puts it - was invisible.
// That is exactly the failure the gate is for: not a wrong gate, a missing one.
// Found on 11 August by appending an ungated `execute_quietly` and watching it
// pass. The fourth case keeps the exclusion honest in the other direction: a test
// double implementing the same trait must still be excused, or the check reports
// every mock as an ungated executor.
//
// Run: node dev/scripts/test-check-executor-gate.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-executor-gate.py");

const failures = [];

// The gate also asserts that every name in its ACKNOWLEDGED map still exists, so
// a stale excuse cannot outlive the file it was written for. A fixture tree has to
// satisfy that too, or every case fails on the staleness check instead of on what
// it is testing. These three are ungated on purpose - that is what being
// acknowledged means.
// Every file the gate's ACKNOWLEDGED table names, because it also reports an entry
// whose file is gone. `placeholder.rs` is in that list and is deliberately NOT
// named like an executor - discovery is by `impl Executor for` now, and this
// fixture is what proves the name stopped mattering.
const ACKNOWLEDGED = [
  "proxy_executor.rs",
  "read_executor.rs",
  "file_executor.rs",
  "placeholder.rs",
];
const ACK_BODY = `impl Executor for Ack {}

impl Ack {
    async fn execute(&self, _p: &Path) -> Result<()> {
        Ok(())
    }
}
`;

function check(name, body, expect, file = "probe_executor.rs") {
  const dir = mkdtempSync(join(tmpdir(), "arlen-execgate-"));
  const src = join(dir, "daemons/ai-engine-daemon/src");
  mkdirSync(src, { recursive: true });
  for (const f of ACKNOWLEDGED) writeFileSync(join(src, f), ACK_BODY);
  // The subject, deliberately not a name the map excuses.
  const abs = join(src, file);
  writeFileSync(abs, body);
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  const got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  const ok = expect(got.code, got.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
  rmSync(dir, { recursive: true, force: true });
}

const GATED = `impl Executor for ProbeExecutor {}

impl ProbeExecutor {
    async fn execute(&self, p: &Path) -> Result<()> {
        if !(self.executor_live)() {
            return Ok(());
        }
        std::fs::remove_file(p)?;
        Ok(())
    }
}
`;

const UNGATED = `impl Executor for ProbeExecutor {}

impl ProbeExecutor {
    async fn execute_quietly(&self, p: &Path) -> Result<()> {
        std::fs::remove_file(p)?;
        Ok(())
    }
}
`;

const TESTS = `#[cfg(test)]
mod tests {
    struct Fake;
    impl Fake {
        async fn execute(&self, _p: &Path) -> Result<()> {
            Ok(())
        }
    }
}
`;

console.log("check-executor-gate:");

check("an executor that reads the flag passes", GATED, (code) => code === 0);

check(
  "an executor that does not read the flag is caught",
  UNGATED,
  (code, out) => code === 1 && out.includes("execute_quietly"),
);

// The case the cut-at-the-first-marker version passed.
check(
  "an ungated executor BELOW the test module is caught",
  `${GATED}\n${TESTS}\n${UNGATED}`,
  (code, out) => code === 1 && out.includes("execute_quietly"),
);

check(
  "a test double inside the test module is still excused",
  `${GATED}\n${TESTS}`,
  (code) => code === 0,
);

// The reason discovery moved off the filename. `placeholder.rs` held a production
// `impl Executor for` that `*executor*.rs` never matched - inert, which is why it
// went unnoticed and why it proved the naming convention was enforced by nothing.
// An ungated executor in a plainly-named file must be caught on its trait alone.
check(
  "an ungated executor in a file not named like one is caught",
  UNGATED,
  (code, out) => code === 1 && out.includes("execute_quietly"),
  "helpers.rs",
);

// And the other half: mentioning the trait is not implementing it, or every file
// that imports the type would be scanned as an executor.
check(
  "a file that only mentions Executor is not scanned",
  "// see dispatch.rs for the Executor trait\nfn helper() {}\n",
  (code) => code === 0,
  "notes.rs",
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("all executor-gate cases passed");
