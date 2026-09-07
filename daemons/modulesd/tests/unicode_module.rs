// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The first real module, hosted: discovery, tier, instantiate, init, search.
//!
//! `modules/unicode` is the first Tier 1 guest in the tree. Everything else in
//! this crate's integration suite either builds its own fixture component or is
//! `#[ignore]`d for want of one, so until this existed the Tier 1 chain had
//! never run against a module somebody would actually ship.
//!
//! It stages the shipped `manifest.toml` and the built component into a temp
//! module directory and drives the manager the way the shell does.
//!
//! **It fails today, and that is the finding rather than a broken test.**
//! Discovery, tier classification and enable all pass. Hosting does not, and
//! BOTH ways of building a Rust guest fail, differently:
//!
//! * `wasm32-wasip2` - the target rustup gives you - links the standard
//!   library, and the standard library imports WASI. Thirteen interfaces in
//!   that build, all `wasi:cli/*` and `wasi:io/*` (stdio, exit, environment),
//!   none of them filesystem or sockets. `modulesd`'s linker provides the four
//!   `arlen:host/*` interfaces and nothing else, so instantiation is refused:
//!   "component imports instance `wasi:io/poll@0.2.6`, but a matching
//!   implementation was not found in the linker".
//!
//! * `wasm32-unknown-unknown` plus `wasm-tools component new` - what
//!   `just module-unicode` builds - imports nothing at all and instantiates
//!   cleanly. Then `init()` traps, at an unnamed wasm function, which is what
//!   Rust's std does on a target where most of it aborts.
//!
//! So there is no way to write this module in Rust today that the daemon can
//! run. Leave the test here and failing: it is the shortest statement of why
//! the runtime has never hosted a guest.
//!
//! `#[ignore]`d because it needs the component built first:
//!
//!   just module-unicode
//!   cargo test -p arlen-modulesd --test unicode_module -- --ignored

use std::path::{Path, PathBuf};
use std::sync::Arc;

use modulesd_proto::{ModuleTier, Request, Response};
use tokio::sync::broadcast;

use arlen_modulesd::manager::Manager;

const MODULE_ID: &str = "core.unicode";

fn repo_root() -> PathBuf {
    // The test runs with the crate as CWD.
    std::env::current_dir().unwrap().join("../..")
}

/// Stage the module the way an install would: manifest plus `module.wasm`.
fn stage(into: &Path) {
    let root = repo_root();
    let manifest = root.join("modules/unicode/manifest.toml");
    let wasm = root.join("modules/unicode/module.wasm");
    assert!(
        wasm.is_file(),
        "the component is not built. See this file's header for the command.",
    );
    let dir = into.join(MODULE_ID);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(&manifest, dir.join("manifest.toml")).unwrap();
    std::fs::copy(&wasm, dir.join("module.wasm")).unwrap();
}

#[tokio::test]
#[ignore = "needs modules/unicode built for wasm32-wasip2; see the file header"]
async fn the_first_module_is_discovered_hosted_and_answers_a_search() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("ARLEN_USER_MODULES_DIR", tmp.path());
    stage(tmp.path());

    let (tx, _rx) = broadcast::channel(16);
    let manager: Arc<Manager> = Manager::new(tx).unwrap();
    manager.discover().await;

    // Discovery, and the tier that follows from `module.wasm` being there.
    let resp = manager
        .handle_request(Request::ListModules { id: "1".into() })
        .await;
    let Response::ModuleList { modules, .. } = resp else {
        panic!("expected a module list");
    };
    let m = modules
        .iter()
        .find(|m| m.id == MODULE_ID)
        .unwrap_or_else(|| panic!("{MODULE_ID} was not discovered: {modules:?}"));
    assert_eq!(m.tier, ModuleTier::Wasm, "a module.wasm on disk is Tier 1");
    assert!(!m.failed, "the module is not in a failed state before it has run");

    // Enabling it. The module asks for nothing, so `describe(caps).needs_consent()`
    // is false and the broker is never dialled - which means this must succeed
    // with no consent broker running at all. That is the property worth pinning:
    // a module that wants no authority is not gated behind a service that would
    // have nothing to ask about.
    let resp = manager
        .handle_request(Request::SetEnabled {
            id: "1b".into(),
            module_id: MODULE_ID.into(),
            enabled: true,
        })
        .await;
    assert!(
        !matches!(resp, Response::Error { .. }),
        "a capability-less module enables without a broker: {resp:?}"
    );

    // A codepoint query: one result, no scan, and the character itself in the
    // title. This is the call that instantiates the component, runs `init` and
    // crosses the WIT boundary in both directions.
    let resp = manager
        .handle_request(Request::WaypointerSearch {
            id: "2".into(),
            module_id: MODULE_ID.into(),
            query: "U+2764".into(),
        })
        .await;
    match resp {
        Response::WaypointerResults { results, module_id, .. } => {
            assert_eq!(module_id, MODULE_ID);
            assert_eq!(results.len(), 1, "one codepoint, one answer: {results:?}");
            assert!(
                results[0].title.contains('\u{2764}'),
                "the title carries the character: {:?}",
                results[0].title
            );
        }
        other => panic!("expected results, got {other:?}"),
    }

    // A name query. The in-process plugin answered this from a prebuilt index;
    // the module scans, because the fuel budget is per call and has no
    // allowance for one-time setup. Whether that fits in 1 M instructions is
    // exactly what this asserts - a trap here is the finding, not a flake.
    let resp = manager
        .handle_request(Request::WaypointerSearch {
            id: "3".into(),
            module_id: MODULE_ID.into(),
            query: "HEART".into(),
        })
        .await;
    match resp {
        Response::WaypointerResults { results, .. } => {
            assert!(!results.is_empty(), "a name search finds something");
        }
        other => panic!("expected results, got {other:?}"),
    }
}
