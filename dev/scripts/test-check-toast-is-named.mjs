// Control for check-toast-is-named.py. The tree is at zero, which is the state
// where a broken checker and a working one look the same, so every case here is
// written for it.
import { mkdtempSync, mkdirSync, writeFileSync, rmSync, copyFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const script = join(here, "check-toast-is-named.py");

function run(files) {
  const root = mkdtempSync(join(tmpdir(), "toast-named-"));
  mkdirSync(join(root, "dev", "scripts"), { recursive: true });
  copyFileSync(script, join(root, "dev", "scripts", "check.py"));
  for (const [rel, text] of Object.entries(files)) {
    const p = join(root, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, text);
  }
  const r = spawnSync("python3", [join(root, "dev", "scripts", "check.py")], { encoding: "utf8" });
  rmSync(root, { recursive: true, force: true });
  return r;
}

let failed = 0;
const check = (name, ok, detail) => {
  if (ok) console.log(`  ok   ${name}`);
  else { console.log(`  FAIL ${name}${detail ? `: ${detail}` : ""}`); failed++; }
};

// The defect as it shipped: a sentence built here.
let r = run({
  "apps/x/src/a.rs": `fn f() { emit_toast(&app, ToastKind::Success, "Night Light is now on".into()); }`,
});
check("a written toast is seen", r.status === 1, r.stdout);
check("and it quotes the sentence", r.stdout.includes("Night Light is now on"), r.stdout);
check("and it points at the named call", r.stdout.includes("emit_toast_key"), r.stdout);

// The same defect through a format!, over more than one line.
r = run({
  "apps/x/src/a.rs": `
fn f() {
    emit_toast(
        &app,
        ToastKind::Error,
        format!("The command did not run: {e}"),
    );
}`,
});
check("a format! sentence is seen", r.status === 1, r.stdout);

// The named call is the point, not a defect.
r = run({
  "apps/x/src/a.rs": `fn f() { emit_toast_key(&app, ToastKind::Error, "sh.toast.commandDidNotRun", &[("why", e)]); }`,
});
check("a named toast passes", r.status === 0, r.stdout);

// A variable carries the line: nothing literal to judge.
r = run({ "apps/x/src/a.rs": `fn f() { emit_toast(&app, kind, message); }` });
check("a toast from a variable passes", r.status === 0, r.stdout);

// A format string that is only placeholders and punctuation says nothing in any
// language, and reporting it was the checker's own first mistake.
r = run({ "apps/x/src/a.rs": `fn f() { emit_toast(&app, kind, format!("{id}: {e}")); }` });
check("placeholders are not prose", r.status === 0, r.stdout);

// The build caches hold older copies of this tree.
r = run({
  "dev/mkosi/mkosi.builddir/x/cargo-home/git/checkouts/arlen-1/2/src/a.rs":
    `fn f() { emit_toast(&app, kind, "Night Light is now on".into()); }`,
});
check("a cached checkout of an old tree is skipped", r.status === 0, r.stdout);

if (failed) { console.log(`\n    ${failed} case(s) failed`); process.exit(1); }
console.log("check-toast-is-named: control cases pass.");
