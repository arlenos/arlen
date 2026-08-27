// Control for check-menu-labels-translated.py: plant the defect it exists for
// and confirm it is seen, and plant the shapes it must NOT flag.
//
// The gate is green on the tree as it stands (nothing names a menu label in
// Rust any more), which is exactly the state where a broken checker looks
// identical to a working one. So every case here is a file written for it.
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const script = join(here, "check-menu-labels-translated.py");

function run(files) {
  const root = mkdtempSync(join(tmpdir(), "menu-label-"));
  mkdirSync(join(root, "dev", "scripts"), { recursive: true });
  // The gate locates the tree from its own path, so it has to live in the fixture.
  const body = spawnSync("cat", [script], { encoding: "utf8" }).stdout;
  writeFileSync(join(root, "dev", "scripts", "check.py"), body);
  for (const [rel, text] of Object.entries(files)) {
    const p = join(root, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, text);
  }
  const r = spawnSync("python3", [join(root, "dev", "scripts", "check.py")], {
    encoding: "utf8",
  });
  rmSync(root, { recursive: true, force: true });
  return r;
}

let failed = 0;
function check(name, ok, detail) {
  if (ok) console.log(`  ok   ${name}`);
  else {
    console.log(`  FAIL ${name}${detail ? `: ${detail}` : ""}`);
    failed++;
  }
}

// The defect as it actually shipped.
let r = run({
  "apps/x/src-tauri/src/lib.rs": `
fn menu() {
    let g = MenuGroup::new("File", vec![MenuItem::item("New Folder", "file.new_folder")]);
}
`,
});
check("the shipped defect is seen", r.status === 1, `exit ${r.status}`);
check("it names the group label", r.stdout.includes("'File'"), r.stdout);
check("it names the item label", r.stdout.includes("'New Folder'"), r.stdout);
check("it points at the frontend fix", r.stdout.includes("menu.ts"), r.stdout);

// A submenu label is a label too.
r = run({
  "apps/x/src-tauri/src/lib.rs": `let s = MenuItem::submenu("Sort By", vec![]);`,
});
check("a submenu label is seen", r.status === 1, r.stdout);

// Labels from variables are the point of the fix, not a defect.
r = run({
  "apps/x/src-tauri/src/lib.rs": `
fn menu(t: &Tr) {
    let g = MenuGroup::new(t.get("f.gm.file"), vec![MenuItem::item(label, "file.new")]);
}
`,
});
check("a label from a call is allowed", r.status === 0, r.stdout);

// The action id sits in the second position, but an id-shaped first argument
// (a label built later, a placeholder) must not read as prose.
r = run({
  "apps/x/src-tauri/src/lib.rs": `let i = MenuItem::item("view.sort.name", "view.sort.name");`,
});
check("an id-shaped literal is not prose", r.status === 0, r.stdout);

// The surface's own file must stay writable with literal fixtures.
r = run({
  "sdk/os-sdk/src/menu.rs": `let g = MenuGroup::new("File", vec![]);`,
  // An ordinary file beside it: the exemption is what this case is about, and
  // the empty-read guard below would otherwise refuse a fixture whose only Rust
  // is the excluded one, so the case would stop failing for its own reason.
  "apps/x/src-tauri/src/lib.rs": `let i = MenuItem::item("view.sort.name", "view.sort.name");`,
});
check("the surface's own tests are exempt", r.status === 0, r.stdout);

// And a tree with no menus at all passes without claiming it checked any.
r = run({ "apps/x/src-tauri/src/lib.rs": `fn main() {}` });
check("a tree with no menu passes", r.status === 0, r.stdout);
check("and says it saw none", r.stdout.includes("0 file(s)"), r.stdout);

// A tree with no Rust in it is a walk that reached nothing, and this used to
// answer "0 file(s) name a menu label" and exit 0 - which is what a renamed
// directory would have earned as well.
r = run({});
check(
  "a tree with no Rust source refuses rather than passing",
  r.status === 2 && `${r.stdout}${r.stderr}`.includes("NOTHING WAS READ"),
  `exit ${r.status}: ${r.stdout}${r.stderr}`,
);

if (failed) {
  console.log(`\n    ${failed} case(s) failed`);
  process.exit(1);
}
console.log("check-menu-labels-translated: control cases pass.");
