//! A guest that reaches for the graph, so the host's refusal can be observed.
//!
//! **This exists to prove a negative that no shipping module can prove.** Since
//! 8 September the daemon builds a `Linker` per module carrying only the
//! interfaces that module's manifest declared, so an undeclared capability is
//! ABSENT rather than refused at call time. `modules/unicode` cannot demonstrate
//! that: it declares nothing and calls nothing, and the component toolchain
//! elides an import the guest never references, so it instantiates either way.
//!
//! This one calls `graph::query` from `init`, which puts the import in the
//! component's import section for real. Instantiated against a linker built from
//! an empty manifest it fails to INSTANTIATE; with `graph.read` declared it
//! instantiates and the call is then refused by scope, which is the other half of
//! the claim.
//!
//! Built by `just fixture-module-reaches-graph`; the test that uses it is
//! `#[ignore]`d, like every test here that needs a component on disk.

wit_bindgen::generate!({
    path: "../../../sdk/module-sdk/wit",
    world: "waypointer-provider",
    generate_all,
});

use exports::arlen::waypointer::provider::{Guest, SearchResult};

struct Fixture;

impl Guest for Fixture {
    fn init() -> Result<(), String> {
        // The whole point: a real reference to an import. The answer does not
        // matter and is deliberately discarded - what matters is that this
        // module cannot be instantiated at all unless the host linked graph.
        let _ = arlen::host::graph::query("MATCH (n) RETURN n LIMIT 1");
        Ok(())
    }

    fn search(_query: String) -> Vec<SearchResult> {
        Vec::new()
    }

    fn execute(_hit: SearchResult) -> Result<(), String> {
        Ok(())
    }

    fn shutdown() {}
}

export!(Fixture);
