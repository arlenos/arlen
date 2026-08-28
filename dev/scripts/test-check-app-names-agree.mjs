// Control for check-app-names-agree.py. The gate is green on the tree, so every
// case here plants a disagreement and confirms it is seen - and plants the two
// shapes that are NOT defects and confirms they are not.
import { mkdirSync, writeFileSync, copyFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const script = join(here, "check-app-names-agree.py");

function app(root, name, { deskEn, deskDe, catEn, catDe, confTitle, noDesktop }) {
  const base = join(root, "apps", name);
  mkdirSync(join(base, "src/lib/i18n"), { recursive: true });
  mkdirSync(join(base, "src-tauri"), { recursive: true });
  writeFileSync(
    join(base, "src/lib/i18n/messages.ts"),
    `const messages = {\n  en: {\n    "x.app.title": "${catEn}",\n  },\n  de: {\n    "x.app.title": "${catDe}",\n  },\n};\n`,
  );
  if (!noDesktop) {
    mkdirSync(join(base, "dist"), { recursive: true });
    writeFileSync(
      join(base, `dist/arlen-${name}.desktop`),
      `[Desktop Entry]\nType=Application\nName=${deskEn}\nName[de]=${deskDe}\n`,
    );
  }
  if (confTitle !== undefined) {
    writeFileSync(
      join(base, "src-tauri/tauri.conf.json"),
      JSON.stringify({ app: { windows: [{ title: confTitle }] } }),
    );
  }
}

function run(build) {
  const root = mint("app-names-");
  mkdirSync(join(root, "dev", "scripts"), { recursive: true });
  copyFileSync(script, join(root, "dev", "scripts", "check.py"));
  build(root);
  const r = spawnSync("python3", [join(root, "dev", "scripts", "check.py")], { encoding: "utf8" });
  cleanup(root);
  return r;
}

let failed = 0;
const check = (name, ok, detail) => {
  if (ok) console.log(`  ok   ${name}`);
  else { console.log(`  FAIL ${name}${detail ? `: ${detail}` : ""}`); failed++; }
};

let r = run((root) =>
  app(root, "clock", { deskEn: "Clock", deskDe: "Uhr", catEn: "Clock", catDe: "Uhr", confTitle: "Clock" }),
);
check("an app that agrees everywhere passes", r.status === 0, r.stdout + r.stderr);

r = run((root) =>
  app(root, "monitor", { deskEn: "Systemmonitor", deskDe: "Systemmonitor", catEn: "Task manager", catDe: "Task-Manager", confTitle: "Task manager" }),
);
check("a launcher name that differs is seen", r.status === 1, r.stdout);
check("and it prints both names", r.stdout.includes("'Systemmonitor'") && r.stdout.includes("'Task manager'"), r.stdout);

r = run((root) =>
  app(root, "clock", { deskEn: "Clock", deskDe: "Clock", catEn: "Clock", catDe: "Uhr", confTitle: "Clock" }),
);
check("a German name that differs is seen", r.status === 1, r.stdout);

r = run((root) =>
  app(root, "clock", { deskEn: "Clock", deskDe: "Uhr", catEn: "Clock", catDe: "Uhr", confTitle: "Arlen Clock" }),
);
check("a config title that differs is seen", r.status === 1, r.stdout);

// The two shapes that are not defects.
r = run((root) =>
  app(root, "shell", { catEn: "Arlen", catDe: "Arlen", confTitle: "arlen-desktop-shell", noDesktop: true }),
);
check("a window nobody launches is not judged", r.status === 0, r.stdout);

r = run((root) => {
  mkdirSync(join(root, "apps", "nocatalog", "dist"), { recursive: true });
  writeFileSync(join(root, "apps", "nocatalog", "dist/x.desktop"), "[Desktop Entry]\nName=Whatever\n");
});
check("an app with no catalog is not judged", r.status === 0, r.stdout);

if (failed) {
  console.log(`\n    ${failed} case(s) failed`);
  process.exit(1);
}
console.log("check-app-names-agree: control cases pass.");
