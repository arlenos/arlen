//! End-to-end test for the Tier 1 WASM dogfood module
//! `examples/modules/com.example.unit-converter`.
//!
//! Verifies the full chain: manifest discovery → tier classification →
//! lazy instantiation → `Guest::init` → `Guest::search` → WIT→proto
//! translation → response back on the socket protocol.
//!
//! The network round-trip inside `Guest::search` is **not** exercised
//! here for the same reasons documented in `currency_dogfood.rs`:
//! firing a real reqwest against `api.exchangerate.host` would gate
//! the suite on outbound HTTPS and a 30 s timeout, and wiremock binds
//! on `127.0.0.1` which the SSRF defense rejects. Real network
//! coverage lives in `network_e2e.rs` against an allowlisted local
//! host. This test feeds the module a query the parser rejects, so
//! `search()` returns an empty `Vec` without touching the network —
//! still proving the WIT call works end-to-end.

use std::path::Path;
use std::sync::Arc;

use modulesd_proto::{ModuleTier, Request, Response};
use tokio::sync::broadcast;

use arlen_modulesd::manager::Manager;

fn copy_dir_recursive(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to);
        } else {
            std::fs::copy(&from, &to).unwrap();
        }
    }
}

fn example_module_path() -> std::path::PathBuf {
    let mut p = std::env::current_dir().unwrap();
    p.push("../examples/modules/com.example.unit-converter");
    p
}

#[tokio::test]
async fn tier1_unit_converter_discovers_runs_init_and_search() {
    let example = example_module_path();
    let wasm = example.join("module.wasm");

    if !example.exists() {
        eprintln!(
            "skipping: example module dir not present at {}",
            example.display()
        );
        return;
    }
    if !wasm.exists() {
        eprintln!(
            "skipping: module.wasm absent at {}. Build with `cargo install cargo-component` + `cargo component build --release` in the example dir.",
            wasm.display()
        );
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("ARLEN_USER_MODULES_DIR", tmp.path());
    copy_dir_recursive(&example, &tmp.path().join("com.example.unit-converter"));

    let (tx, _rx) = broadcast::channel(16);
    let manager: Arc<Manager> = Manager::new(tx).unwrap();
    manager.discover().await;

    // 1. Discovery classifies the module as Tier 1 WASM and surfaces
    //    it in the module list.
    let resp = manager
        .handle_request(Request::ListModules { id: "1".into() })
        .await;
    let mut found = false;
    if let Response::ModuleList { modules, .. } = resp {
        for m in modules {
            if m.id == "com.example.unit-converter" {
                assert_eq!(
                    m.tier,
                    ModuleTier::Wasm,
                    "module.wasm is on disk; expected Tier 1 classification",
                );
                assert!(m.enabled);
                assert!(!m.failed);
                found = true;
            }
        }
    }
    assert!(found, "unit-converter not discovered after copy");

    // 2. WaypointerSearch with an unparseable query exercises the full
    //    instantiate → init → search WIT call chain. The module's
    //    `parse_query` returns `None`, `search` returns an empty
    //    `Vec`, and modulesd round-trips that as a typed response.
    //    Crucially: `Guest::init` ran, the linker bound all four host
    //    imports, and the search export was reachable.
    let resp = manager
        .handle_request(Request::WaypointerSearch {
            id: "2".into(),
            module_id: "com.example.unit-converter".into(),
            query: "this is not a currency conversion".into(),
        })
        .await;
    match resp {
        Response::WaypointerResults { results, module_id, .. } => {
            assert_eq!(module_id, "com.example.unit-converter");
            assert!(
                results.is_empty(),
                "unparseable query must produce zero results without touching the network"
            );
        }
        Response::Error { code, message, .. } => {
            panic!(
                "search returned error: code={code:?} message={message} — module probably failed to instantiate or init"
            );
        }
        other => panic!("unexpected response: {other:?}"),
    }

    // 3. A second search call exercises the cached `Tier1Instance`
    //    path: `ensure_tier1_instance` returns the existing entry
    //    rather than recompiling. This is the hot path for normal
    //    per-keystroke search.
    let resp = manager
        .handle_request(Request::WaypointerSearch {
            id: "3".into(),
            module_id: "com.example.unit-converter".into(),
            query: "also not a query".into(),
        })
        .await;
    assert!(
        matches!(resp, Response::WaypointerResults { .. }),
        "second call (cached instance) must succeed: {resp:?}"
    );

    // 4. Clean shutdown: SIGTERM handler in main calls
    //    shutdown_all_tier1 which iterates instances and calls
    //    Guest::shutdown. We exercise the same path explicitly here
    //    to verify it completes against a real loaded instance.
    manager.shutdown_all_tier1().await;

    std::env::remove_var("ARLEN_USER_MODULES_DIR");
}
