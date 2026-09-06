// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Both directions for `check-dbus-call-arity.py`.
//
// The positive case is the real one: the job server's `Register` grew a ninth
// argument and the shared proxy did not. The negatives are the three shapes that
// made the first run of this check wrong, each kept because each was a real bug
// in the reader rather than a hypothetical - a lifetime read as a quote, a comma
// inside an argument comment, and two interfaces serving the same member name.

import { mkdirSync, writeFileSync, readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const source = readFileSync(join(here, "check-dbus-call-arity.py"), "utf8");

let failed = 0;
function ok(name, cond, detail = "") {
  console.log(`  ${cond ? "ok  " : "FAIL"} ${name}`);
  if (!cond) {
    failed++;
    if (detail) console.log(`       ${detail.trim().split("\n").slice(0, 3).join(" | ")}`);
  }
}

// Every case mints its own tree; the check never reads the real one.
function run(files) {
  const root = mint("dbus-arity-");
  try {
    for (const [rel, body] of Object.entries(files)) {
      const p = join(root, rel);
      mkdirSync(dirname(p), { recursive: true });
      writeFileSync(p, body);
    }
    mkdirSync(join(root, "dev", "scripts"), { recursive: true });
    const copy = join(root, "dev", "scripts", "check-dbus-call-arity.py");
    writeFileSync(copy, source);
    const r = spawnSync("python3", [copy, root], { encoding: "utf8" });
    return { code: r.status, out: `${r.stdout}${r.stderr}` };
  } finally {
    cleanup(root);
  }
}

const SERVER = `
#[zbus::interface(name = "org.arlen.Thing1")]
impl Thing {
    async fn register(&self, app: String, title: String, items: Vec<String>) -> u64 { 1 }
}
`;

console.log("the gate compares a call against the member it dials");

// 1. The real one: a proxy one argument behind its server.
{
  const { code, out } = run({
    "daemons/thing/src/dbus.rs": SERVER,
    "contracts/proto/src/client.rs": `
#[zbus::proxy(interface = "org.arlen.Thing1", default_path = "/x")]
pub trait Thing {
    async fn register(&self, app: &str, title: &str) -> zbus::Result<u64>;
}
`,
  });
  ok("a proxy behind its server is caught", code === 1, out);
  ok("and the finding names both counts", out.includes("declares 2") && out.includes("takes 3"), out);
}

// 2. The same proxy, current.
{
  const { code } = run({
    "daemons/thing/src/dbus.rs": SERVER,
    "contracts/proto/src/client.rs": `
#[zbus::proxy(interface = "org.arlen.Thing1", default_path = "/x")]
pub trait Thing {
    async fn register(&self, app: &str, title: &str, items: &[&str]) -> zbus::Result<u64>;
}
`,
  });
  ok("a proxy that agrees is not a finding", code === 0);
}

// 3. A lifetime is not a quote. The first run read `Header<'_>` as opening a
//    string and reported that half the daemons serve argument-less methods.
{
  const { code, out } = run({
    "daemons/thing/src/dbus.rs": `
#[zbus::interface(name = "org.arlen.Thing1")]
impl Thing {
    async fn register(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        app: String,
        title: String,
        items: Vec<String>,
    ) -> u64 { 1 }
}
`,
    "contracts/proto/src/client.rs": `
#[zbus::proxy(interface = "org.arlen.Thing1", default_path = "/x")]
pub trait Thing {
    async fn register(&self, app: &str, title: &str, items: &[&str]) -> zbus::Result<u64>;
}
`,
  });
  ok("an injected header is not an argument the caller sends", code === 0, out);
}

// 4. A comma inside an argument's comment is not an argument. The file
//    manager's real call carries one per flag and two of them have commas.
{
  const { code, out } = run({
    "daemons/thing/src/dbus.rs": SERVER,
    "apps/x/src-tauri/src/jobs.rs": `
const INTERFACE: &str = "org.arlen.Thing1";
async fn go(proxy: &zbus::Proxy<'static>) {
    let _: Result<u64, _> = proxy
        .call(
            "Register",
            &(
                app, // the app, its id, and nothing else
                title,
                items, // the entries, in order, so the zone can expand them
            ),
        )
        .await;
}
`,
  });
  ok("a comma inside an argument comment is not an argument", code === 0, out);
}

// 5. Two interfaces, one member name: the portal frontend's three-argument
//    OpenFile and the impl backend's five-argument one are both correct.
{
  const { code, out } = run({
    "daemons/portal/src/file_chooser.rs": `
#[zbus::interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl Backend {
    async fn open_file(&self, handle: String, app_id: String, parent: String, title: String, opts: String) -> u32 { 0 }
}
`,
    "sdk/plugin/src/portal_proxy.rs": `
#[zbus::proxy(interface = "org.freedesktop.portal.FileChooser", default_path = "/x")]
pub trait FileChooser {
    async fn open_file(&self, parent: &str, title: &str, opts: &str) -> zbus::Result<u32>;
}
`,
  });
  ok("the same member on another interface is not compared", code === 0, out);
}

if (failed) {
  console.log(`\n${failed} failed`);
  process.exit(1);
}
console.log("both directions hold");
