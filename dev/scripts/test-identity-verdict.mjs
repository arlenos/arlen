// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for the boot harness's identity verdict.
//
// The verdict fails a boot whose console shows the identity chain disagreeing with
// itself. It exists because on 13 Aug both of its cases happened for real, both
// passed the boot, and both were found by reading a log on a hunch - so what needs
// pinning is not that it can fire, but WHICH events fire it. The one that must NOT
// is `broker_unauthenticated`: that is a known open decision, and a gate that stays
// red for a reason nobody may act on is one people learn to ignore.

import { spawnSync } from "node:child_process";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

/** Run `identity_faults` over `text` and return the printable lines. */
function faults(text) {
  const py = `
import sys, json
sys.path.insert(0, ${JSON.stringify(join(ROOT, "dev/vm"))})
from verify import identity_faults
print(json.dumps(identity_faults(sys.stdin.read())))
`;
  const r = spawnSync("python3", ["-c", py], { input: text, encoding: "utf8" });
  if (r.status !== 0) {
    console.log(r.stderr);
    return null;
  }
  return JSON.parse(r.stdout);
}

const DIVERGENCE =
  'consent-broker[1]: WARN audit: stamped identity diverges from the legacy /proc ' +
  'resolution event="identity.divergence" pid=710 legacy="dev.arlen.desktop-shell" ' +
  'stamped="desktop-shell"\n';
const REFUSED =
  'consent-broker[1]: WARN audit: identity broker returned a reserved or malformed ' +
  'app_id event="identity.broker_returned_reserved_or_invalid" app_id=ai-agent\n';
const UNAUTHENTICATED =
  'undo-signer[1]: WARN audit: identity broker not authenticated ' +
  'event="identity.broker_unauthenticated" error=broker uid 65534 != expected 0\n';

console.log("identity verdict:");

{
  const f = faults("nothing interesting here\n");
  check("a clean console is clean", Array.isArray(f) && f.length === 0);
}
{
  const f = faults(DIVERGENCE);
  check("a divergence fails the boot", f?.some((l) => l.includes("identity.divergence")));
  check(
    "and the line is quoted so the reader sees which two ids",
    f?.some((l) => l.includes("dev.arlen.desktop-shell")),
  );
}
{
  const f = faults(REFUSED);
  check(
    "a refused stamp fails the boot",
    f?.some((l) => l.includes("broker_returned_reserved_or_invalid")),
  );
}
{
  // The deliberate exclusion, and the reason this file exists rather than a comment:
  // an open decision must not hold the gate red.
  const f = faults(UNAUTHENTICATED);
  check("an unauthenticated broker does NOT fail the boot", f?.length === 0);
}
{
  // Both at once still names both, so a second fault is not hidden by the first.
  const f = faults(DIVERGENCE + REFUSED);
  check("two faults are both reported", (f?.length ?? 0) >= 4);
}

console.log(failures ? `\n${failures} failure(s)` : "\nthe identity verdict holds");
process.exit(failures ? 1 : 0);
