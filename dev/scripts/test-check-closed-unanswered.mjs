// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Both directions for `check-closed-unanswered.py`.
//
// A gate that has never been red is a gate nobody has tested, and this one was
// born green - the six defects that motivated it were fixed the same afternoon.
// So the control matters more than usual: it puts each shape back and requires a
// refusal, and it puts the honest versions in and requires a pass.

import { mkdirSync, writeFileSync, readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const source = readFileSync(join(here, "check-closed-unanswered.py"), "utf8");

let failed = 0;
function ok(name, cond) {
  console.log(`  ${cond ? "ok  " : "FAIL"} ${name}`);
  if (!cond) failed++;
}

/// Run the gate against a throwaway tree holding one file.
///
/// Minted and removed through `lib/fixture.mjs`, which refuses a path it has no
/// record of creating. The first cut of this file called `rmSync` on its own temp
/// dir and `check-fixture-deletes` refused the commit - correctly: the rule is not
/// that the path looks safe, it is that the delete cannot be handed one it did not
/// make. That check exists because a control here once deleted the working tree.
function run(body, name = "Thing.svelte") {
  const root = mint("closed-unanswered-");
  try {
    const dir = join(root, "apps", "demo", "src", "lib");
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, name), body);
    // The gate resolves its tree from its own location, so it is copied beside a
    // fixture `apps/` rather than pointed at one.
    mkdirSync(join(root, "dev", "scripts"), { recursive: true });
    const copy = join(root, "dev", "scripts", "check-closed-unanswered.py");
    writeFileSync(copy, source);
    const r = spawnSync("python3", [copy], { encoding: "utf8" });
    return { code: r.status, out: `${r.stdout}${r.stderr}` };
  } finally {
    cleanup(root);
  }
}

console.log("the gate catches a surface closing on a discarded answer, and passes the honest forms");

// 1. The direct shape: swallow, then close, in one body.
{
  const { code, out } = run(`<script>
  function pick() {
    invoke("set_query_and_show", { query: "p:" }).catch(() => {});
    close();
  }
</script>`);
  ok("a discarded rejection beside a close is caught", code === 1);
  ok("and the finding names the file", out.includes("apps/demo/src/lib/Thing.svelte"));
}

// 2. The one-hop shape: a helper swallows, its caller closes.
{
  const { code } = run(`<script>
  function copyText(t) {
    navigator.clipboard.writeText(t).catch(() => {});
  }
  function onPick(v) {
    copyText(v);
    closePopover();
  }
</script>`);
  ok("a swallowing helper called from a closing function is caught", code === 1);
}

// 3. Honest: the answer is read, and the close is on success only.
{
  const { code } = run(`<script>
  function pick() {
    void shellAction("set_query_and_show", { query: "p:" }, "sh.toast.launcherClosed")
      .then(() => close());
  }
</script>`);
  ok("reading the answer before closing passes", code === 0);
}

// 4. Honest: a log line that could not be written is not a claim to anybody.
{
  const { code } = run(`<script>
  function pick() {
    invoke("frontend_log", { msg: "picked" }).catch(() => {});
    close();
  }
</script>`);
  ok("a discarded log line beside a close passes", code === 0);
}

// 5. Honest: a discarded rejection with no surface closing under it.
{
  const { code } = run(`<script>
  function next() {
    invoke("mpris_next", { id }).catch(() => {});
  }
</script>`);
  ok("a discarded rejection with nothing closing passes", code === 0);
}

// 6. The brace-scoping the sibling check learned the hard way: a close in the
//    function ABOVE is not this function's close.
{
  const { code } = run(`<script>
  function shut() {
    close();
  }
  function next() {
    invoke("mpris_next", { id }).catch(() => {});
  }
</script>`);
  ok("a close in a neighbouring function does not count", code === 0);
}

// 7. A brace inside a string must not move the function boundary.
{
  const { code } = run(`<script>
  function shut() {
    close();
    const s = "}";
  }
  function next() {
    invoke("mpris_next", { id }).catch(() => {});
  }
</script>`);
  ok("a brace inside a string does not move the boundary", code === 0);
}

// 8. Honest: a window-control button, where closing IS the action. The PDF
//    reader's `winClose` is this shape, and an earlier cut of the gate matched it
//    against itself because only the `.catch(...)` text was removed rather than
//    the whole statement holding it.
{
  const { code } = run(`<script>
  function winClose() {
    getCurrentWindow().close().catch(() => {});
  }
</script>`);
  ok("a close that IS the discarded call does not match itself", code === 0);
}

// 9. And the same file must still be caught when a second statement closes.
{
  const { code } = run(`<script>
  function save() {
    getCurrentWindow().close().catch(() => {});
    closePopover();
  }
</script>`);
  ok("but a real close in another statement still is", code === 1);
}

console.log(failed === 0 ? "\nboth directions hold" : `\n${failed} failed`);
process.exit(failed === 0 ? 0 : 1);
