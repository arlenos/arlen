// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// Does `check-portal-interfaces.py` fail when the fault is put back?
//
// Every case here is a state the tree was actually in, or one file away from it:
// Print served and advertised nowhere (19 August, the reason the check exists),
// an interface advertised but not served (the dialog that opens and fails, which
// the `.portal` file warns about in its own comment), and the half-advertised
// state where the two files disagree about the same interface.

import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-portal-interfaces.py");
let failures = 0;

/** A tree with the given served types, portal line and preferred keys. */
function tree({ served, portal, preferred }) {
  const root = mkdtempSync(join(tmpdir(), "portal-iface-"));
  const src = join(root, "daemons/xdg-portal/daemon/src/interfaces");
  const dist = join(root, "daemons/xdg-portal/dist/xdg-desktop-portal/portals");
  mkdirSync(src, { recursive: true });
  mkdirSync(dist, { recursive: true });

  // One file per interface, in the shape the real ones have - including the
  // clippy allow between the attribute and the impl, which is what broke the
  // first version of the check.
  const all = ["FileChooser", "OpenUri", "Screenshot", "Print", "ScreenCast"];
  const iface = (t) =>
    t === "OpenUri" ? "OpenURI" : t;
  for (const t of all) {
    writeFileSync(
      join(src, `${t.toLowerCase()}.rs`),
      `#[interface(name = "org.freedesktop.impl.portal.${iface(t)}")]\n` +
        `#[allow(clippy::too_many_arguments)] // spec-mandated method signatures\n` +
        `impl ${t} {\n    async fn thing(&self) {}\n}\n`,
    );
  }
  writeFileSync(
    join(root, "daemons/xdg-portal/daemon/src/main.rs"),
    served.map((t) => `        .serve_at(OBJECT_PATH, ${t}::new())`).join("\n") + "\n",
  );
  writeFileSync(
    join(dist, "arlen.portal"),
    `[portal]\nDBusName=org.freedesktop.impl.portal.desktop.arlen\nInterfaces=${portal
      .map((n) => `org.freedesktop.impl.portal.${n}`)
      .join(";")};\nUseIn=arlen;\n`,
  );
  writeFileSync(
    join(root, "daemons/xdg-portal/dist/xdg-desktop-portal/arlen-portals.conf"),
    "# a comment\n[preferred]\n" +
      preferred.map((n) => `org.freedesktop.impl.portal.${n}=arlen`).join("\n") +
      "\n",
  );
  return root;
}

function run(root) {
  try {
    return { code: 0, out: execFileSync("python3", [check, root], { encoding: "utf8" }) };
  } catch (e) {
    return { code: e.status ?? 1, out: `${e.stdout ?? ""}${e.stderr ?? ""}` };
  }
}

function expect(name, root, wantFail, mustSay) {
  const { code, out } = run(root);
  const ok = wantFail ? code !== 0 : code === 0;
  const said = !mustSay || out.includes(mustSay);
  if (ok && said) {
    console.log(`  ok   ${name}`);
  } else {
    failures++;
    console.log(`  FAIL ${name}\n       exit ${code}\n${out}`);
  }
  rmSync(root, { recursive: true, force: true });
}

console.log("check-portal-interfaces:");

// The state the tree was in: Print on the bus, in neither file.
expect(
  "an interface served and advertised nowhere is refused",
  tree({
    served: ["FileChooser", "OpenUri", "Screenshot", "Print", "ScreenCast"],
    portal: ["FileChooser", "OpenURI"],
    preferred: ["FileChooser", "OpenURI"],
  }),
  true,
  "never route it here",
);

// The failure the `.portal` comment warns about, from the other side.
expect(
  "an interface advertised but not served is refused",
  tree({
    served: ["FileChooser", "OpenUri", "Print", "ScreenCast"],
    portal: ["FileChooser", "OpenURI", "Screenshot"],
    preferred: ["FileChooser", "OpenURI", "Screenshot"],
  }),
  true,
  "cannot answer",
);

// Half-advertised: routing would rest on the deprecated UseIn= key alone.
expect(
  "an interface in the .portal file but not the preferences is refused",
  tree({
    served: ["FileChooser", "OpenUri", "Screenshot", "Print", "ScreenCast"],
    portal: ["FileChooser", "OpenURI", "Screenshot"],
    preferred: ["FileChooser", "OpenURI"],
  }),
  true,
  "must agree",
);

// And the mirror of it.
expect(
  "an interface in the preferences but not the .portal file is refused",
  tree({
    served: ["FileChooser", "OpenUri", "Screenshot", "Print", "ScreenCast"],
    portal: ["FileChooser", "OpenURI"],
    preferred: ["FileChooser", "OpenURI", "Screenshot"],
  }),
  true,
  "must agree",
);

// The good state, so the check is not merely always-red.
expect(
  "a tree where the bus and both files agree passes, with the waiting two served",
  tree({
    served: ["FileChooser", "OpenUri", "Screenshot", "Print", "ScreenCast"],
    portal: ["FileChooser", "OpenURI", "Screenshot"],
    preferred: ["FileChooser", "OpenURI", "Screenshot"],
  }),
  false,
  null,
);

if (failures) {
  console.log(`${failures} case(s) failed`);
  process.exit(1);
}
console.log("the check refuses every shape of disagreement and passes the agreeing tree");
