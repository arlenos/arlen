// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! A module that did not declare a capability cannot be instantiated at all.
//!
//! **The test the ruling named**, and the one no shipping module can stand in
//! for: `modules/unicode` declares nothing and calls nothing, and the component
//! toolchain elides an import the guest never references, so it instantiates
//! whatever the linker carries. `dev/fixtures/module-reaches-graph` calls
//! `graph::query` in `init`, which puts the import in the component for real.
//!
//! Both directions, because the point is structural isolation rather than a
//! refusal: with an empty manifest the module fails to INSTANTIATE, and with
//! `graph.read` declared the same component instantiates. If the second half
//! ever fails, the per-module linker has quietly stopped granting what a
//! manifest asks for, which is the same defect wearing the other face.
//!
//! `#[ignore]`d because it needs the fixture built:
//!
//!   just fixture-module-reaches-graph
//!   cargo test -p arlen-modulesd --test undeclared_is_absent -- --ignored

use std::sync::Arc;

use arlen_modules::{GraphCapability, ModuleCapabilities};
use arlen_modulesd::host::context::CapabilityContext;
use arlen_modulesd::runtime::tier1::Tier1Runtime;
use os_sdk::{UnixEventEmitter, UnixGraphClient};

const FIXTURE: &str = "dev/fixtures/module-reaches-graph/module.wasm";

fn component_bytes() -> Vec<u8> {
    let root = std::env::current_dir().unwrap().join("../..");
    let path = root.join(FIXTURE);
    std::fs::read(&path).unwrap_or_else(|e| {
        panic!("{}: {e}. Build it with `just fixture-module-reaches-graph`.", path.display())
    })
}

async fn instantiate_with(caps: ModuleCapabilities) -> Result<(), String> {
    let runtime = Tier1Runtime::new().expect("runtime");
    let component = wasmtime::component::Component::new(runtime.engine(), component_bytes())
        .expect("the fixture compiles");
    let graph = Arc::new(UnixGraphClient::new("/tmp/arlen-test-knowledge.sock"));
    let events = Arc::new(UnixEventEmitter::new("/tmp/arlen-test-events.sock"));
    runtime
        .instantiate(
            "com.example.reaches-graph",
            &component,
            CapabilityContext::new("com.example.reaches-graph", caps),
            graph,
            events,
        )
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tokio::test]
#[ignore = "needs dev/fixtures/module-reaches-graph built; see the file header"]
async fn a_module_that_declared_nothing_cannot_be_instantiated() {
    let err = instantiate_with(ModuleCapabilities::default())
        .await
        .expect_err("a module reaching for an undeclared import must not instantiate");
    assert!(
        err.contains("instantiate"),
        "it fails at instantiation rather than at the call: {err}"
    );
    // Named, so this cannot pass on an unrelated failure - a missing fixture or
    // a broken engine would also "fail to instantiate".
    assert!(
        err.contains("graph"),
        "and it fails for the import it reached for: {err}"
    );
}

#[tokio::test]
#[ignore = "needs dev/fixtures/module-reaches-graph built; see the file header"]
async fn the_same_module_instantiates_once_it_declares_the_capability() {
    let caps = ModuleCapabilities {
        graph: Some(GraphCapability {
            read: vec!["system.File".into()],
            write: Vec::new(),
        }),
        ..Default::default()
    };
    instantiate_with(caps)
        .await
        .expect("a declared capability is linked, so the same component starts");
}
