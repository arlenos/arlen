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
//! **WHAT THIS FILE SAID BEFORE, AND WHY IT WAS WRONG.** It concluded that no
//! Rust guest the daemon can run could be written today: the `wasm32-wasip2`
//! build imports thirteen `wasi:*` interfaces our linker does not provide, and
//! the `wasm32-unknown-unknown` build "traps at an unnamed wasm function, which
//! is what Rust's std does on a target where most of it aborts". The first half
//! is true. The second was a guess, and it was the daemon's fault, not the
//! guest's: `tier1.rs` printed the trap with anyhow's plain Display, which drops
//! the source where wasmtime puts the reason. Printing the chain said
//! `wasm trap: interrupt` - the HOST interrupting the guest, because epoch
//! interruption was enabled while nothing ever set a store deadline or advanced
//! the epoch, so every module was already past its deadline on its first
//! instruction. Fixed on 8 September; this test is the thing that found it.
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

}

/// A name search, which is the open question rather than a passing check.
///
/// **It fails, and that is the finding.** The in-process plugin answers this
/// from an index of every named codepoint, built once and reused. A module
/// cannot: the fuel budget is per host call and has no allowance for one-time
/// setup, so the guest walks the codepoint space on every keystroke and 1 M fuel
/// does not reach `HEART`. Measured 8 September, with the daemon's error chain
/// finally printed: `wasm trap: all fuel consumed by WebAssembly`.
///
/// Left asserting what an extension author should get rather than the trap they
/// do get, on purpose. A test that asserted the trap would go green over a real
/// gap and defend it; this one goes green the day the gap closes.
///
/// The gap is a capability question rather than a plumbing one, so it is
/// recorded for the planner in `coder-reports.md`: a larger one-time budget for
/// `init`, a persistent index in the instance the daemon already keeps between
/// calls, or setup fuel paid once and refilled per call.
#[tokio::test]
#[ignore = "the open fuel finding; needs modules/unicode built, see the file header"]
async fn a_name_search_should_not_have_to_rescan_the_codepoint_space() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("ARLEN_USER_MODULES_DIR", tmp.path());
    stage(tmp.path());

    let (tx, _rx) = broadcast::channel(16);
    let manager: Arc<Manager> = Manager::new(tx).unwrap();
    manager.discover().await;
    let _ = manager
        .handle_request(Request::SetEnabled {
            id: "1".into(),
            module_id: MODULE_ID.into(),
            enabled: true,
        })
        .await;

    let resp = manager
        .handle_request(Request::WaypointerSearch {
            id: "2".into(),
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

/// An instance that has been idle longer than the epoch deadline still answers.
///
/// **This caught a bug an hour after the deadline was introduced, which is why it
/// is kept rather than deleted.** The epoch deadline counts from the CURRENT
/// epoch, so setting it once when the store is created gives the instance five
/// seconds of wall clock in total - not five seconds of running. A module worked
/// until the deadline passed and then trapped with `wasm trap: interrupt` on
/// every later call, on an instance that had done nothing in between. The daemon
/// now sets both budgets together at each host call.
///
/// The pause is longer than `EPOCH_DEADLINE_TICKS` on purpose: shorter and the
/// test passes whether or not the bug is back.
#[tokio::test]
#[ignore = "needs modules/unicode built (see the file header) and pauses past the deadline"]
async fn an_idle_instance_still_answers_after_the_deadline_would_have_passed() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("ARLEN_USER_MODULES_DIR", tmp.path());
    stage(tmp.path());

    let (tx, _rx) = broadcast::channel(16);
    let manager: Arc<Manager> = Manager::new(tx).unwrap();
    manager.discover().await;
    let _ = manager
        .handle_request(Request::SetEnabled {
            id: "1".into(),
            module_id: MODULE_ID.into(),
            enabled: true,
        })
        .await;

    // The first call is what instantiates, so the clock starts here.
    let _ = manager
        .handle_request(Request::WaypointerSearch {
            id: "2".into(),
            module_id: MODULE_ID.into(),
            query: "U+2764".into(),
        })
        .await;

    tokio::time::sleep(std::time::Duration::from_secs(6)).await;

    let resp = manager
        .handle_request(Request::WaypointerSearch {
            id: "3".into(),
            module_id: MODULE_ID.into(),
            query: "U+2764".into(),
        })
        .await;
    match resp {
        Response::WaypointerResults { results, .. } => {
            assert_eq!(results.len(), 1, "the idle instance still answers: {results:?}");
        }
        other => panic!("an instance idle past the deadline was killed: {other:?}"),
    }
}
