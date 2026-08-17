// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-announced-refusal.
//
// The first case is the defect this was written from, reduced: the sound
// panel's refusal strip, drawn on a failed mute and announced to nobody.
//
// The rest are the lines that were hard to draw. A load-time flag must not be
// dragged in, a message handed to a child component must be followed into that
// child, an `{:else}` branch must not be read as part of the failure branch, and
// field validation tied to its input must pass without an assertive alert. Each
// of those was wrong in a first version, and each one made the gate either shout
// at a page that was fine or stay quiet about one that was not.
//
// Run: node dev/scripts/test-check-announced-refusal.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-announced-refusal.py");

const failures = [];

function check(name, files, expect) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-announce-"));
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
  rmSync(dir, { recursive: true, force: true });
}

const panel = (attrs) =>
  '<script lang="ts">\n' +
  '  let error = $state<string | null>(null);\n' +
  "  async function toggleMute() {\n" +
  "    try {\n" +
  '      await invoke("toggle_audio_mute");\n' +
  "    } catch {\n" +
  '      error = "sh.audio.writeFailed";\n' +
  "    }\n" +
  "  }\n" +
  "</script>\n\n" +
  "{#if error}\n" +
  `  <div class="pop-error"${attrs}>{$t(error)}</div>\n` +
  "{/if}\n" +
  '<button onclick={toggleMute}>mute</button>\n';

console.log("check-announced-refusal:");

check(
  "a refusal drawn after a press with no live region is a finding",
  { "apps/shell/src/lib/AudioPopover.svelte": panel("") },
  (code, out) => code === 1 && out.includes("`error`"),
);

check(
  "the same panel passes once the message announces",
  { "apps/shell/src/lib/AudioPopover.svelte": panel(' role="alert"') },
  (code) => code === 0,
);

check(
  "aria-live counts as well as role=alert",
  { "apps/shell/src/lib/AudioPopover.svelte": panel(' aria-live="assertive"') },
  (code) => code === 0,
);

// A read that failed while the page drew is prose a reader meets in order.
// Marking it assertive would interrupt for something that was already there, so
// a flag arriving through a store must not be pulled into scope.
check(
  "a store-backed load failure is not this check's business",
  {
    "apps/shell/src/lib/Panel.svelte":
      '<script lang="ts">\n' +
      '  import { placesUnavailable } from "./store";\n' +
      "</script>\n\n" +
      "{#if $placesUnavailable}\n" +
      "  <p>We could not read your places.</p>\n" +
      "{/if}\n",
  },
  (code) => code === 0,
);

// Six panels share one banner component. Looking only at the caller's block
// would report every one of them as silent while the role sits one file over.
check(
  "a message handed to a child that announces is followed into the child",
  {
    "apps/shell/src/lib/Banner.svelte":
      '<script lang="ts">\n  let { message } = $props();\n</script>\n' +
      '<div class="pop-error" role="alert">{message}</div>\n',
    "apps/shell/src/lib/AudioPopover.svelte":
      '<script lang="ts">\n' +
      '  import PopoverErrorBanner from "./Banner.svelte";\n' +
      "  let error = $state<string | null>(null);\n" +
      '  function press() { error = "no"; }\n' +
      "</script>\n\n" +
      "{#if error}\n  <PopoverErrorBanner message={$t(error)} />\n{/if}\n",
  },
  (code) => code === 0,
);

check(
  "a child that does not announce is still a finding",
  {
    "apps/shell/src/lib/Plain.svelte":
      '<script lang="ts">\n  let { message } = $props();\n</script>\n' +
      "<div>{message}</div>\n",
    "apps/shell/src/lib/AudioPopover.svelte":
      '<script lang="ts">\n' +
      '  import Plain from "./Plain.svelte";\n' +
      "  let error = $state<string | null>(null);\n" +
      '  function press() { error = "no"; }\n' +
      "</script>\n\n" +
      "{#if error}\n  <Plain message={$t(error)} />\n{/if}\n",
  },
  (code, out) => code === 1 && out.includes("`error`"),
);

// Bluetooth draws its banner in the truthy branch and "Connecting..." in the
// else. Reading past the `{:else}` found an icon component there, could not
// resolve it, and called the whole block silent.
check(
  "an else branch is not read as part of the failure branch",
  {
    "apps/shell/src/lib/Banner.svelte":
      '<script lang="ts">\n  let { message } = $props();\n</script>\n' +
      '<div role="alert">{message}</div>\n',
    "apps/shell/src/lib/BluetoothPopover.svelte":
      '<script lang="ts">\n' +
      '  import PopoverErrorBanner from "./Banner.svelte";\n' +
      '  import { Bluetooth } from "lucide-svelte";\n' +
      "  let error = $state<string | null>(null);\n" +
      '  function press() { error = "no"; }\n' +
      "</script>\n\n" +
      "{#if error}\n" +
      "  <PopoverErrorBanner message={$t(error)} />\n" +
      "{:else}\n" +
      "  <div><Bluetooth size={32} /><span>Connecting...</span></div>\n" +
      "{/if}\n",
  },
  (code) => code === 0,
);

// Validation re-runs on every keystroke. An assertive alert there interrupts on
// each one, so the message is tied to its field instead of shouted.
check(
  "field validation tied to its input passes without an alert",
  {
    "apps/settings/src/lib/RuleDialog.svelte":
      '<script lang="ts">\n' +
      "  let appIdError = $state<string | null>(null);\n" +
      "  function validate(v) { appIdError = v ? null : \"bad\"; }\n" +
      "</script>\n\n" +
      '<input aria-invalid={appIdError !== null}\n' +
      '  aria-describedby={appIdError !== null ? "e" : undefined} />\n' +
      '{#if appIdError}<div class="field-error" id="e">{appIdError}</div>{/if}\n',
  },
  (code) => code === 0,
);

// A checker that reads nothing must say so. Silence and a clean tree look the
// same from the exit code otherwise, which is the failure this whole file is
// about, one level up.
check(
  "an empty tree refuses rather than passing",
  { "README.md": "no apps here\n" },
  (code, out) => code === 2 && out.includes("NOTHING WAS READ"),
);

if (failures.length) {
  console.log("");
  for (const f of failures) {
    console.log(`--- ${f.name} (exit ${f.code})`);
    console.log(f.out.trim());
  }
  process.exit(1);
}
console.log("a refusal that only the sighted can notice is a finding, and every line around that is drawn where it was meant to be");
