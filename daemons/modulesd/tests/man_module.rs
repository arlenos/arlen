// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The module that could not exist, hosted.
//!
//! On 8 September the first module's report said `man` could not be a module at
//! all: the host imports were graph, network, events and log, and a module whose
//! whole job is reading `/usr/share/man` had nothing to read it with. This is the
//! test of the answer - it reads real man pages through `files.read`, and it is
//! the first module whose results carry a key the host resolves.
//!
//! `#[ignore]`d because it needs the component built and a machine with man pages:
//!
//!   just module-man
//!   cargo test -p arlen-modulesd --test man_module -- --ignored

use std::path::{Path, PathBuf};
use std::sync::Arc;

use arlen_modules::{FilesCapability, ModuleCapabilities};
use arlen_modulesd::host::context::CapabilityContext;
use arlen_modulesd::runtime::tier1::Tier1Runtime;
use os_sdk::{UnixEventEmitter, UnixGraphClient};

const MODULE_ID: &str = "core.man";

fn repo_root() -> PathBuf {
    std::env::current_dir().unwrap().join("../..")
}

/// The capabilities the shipped manifest declares, read from it rather than
/// retyped: if the manifest and this test disagree, the test is wrong about the
/// module somebody actually installs.
fn declared_capabilities() -> ModuleCapabilities {
    let text = std::fs::read_to_string(repo_root().join("modules/man/manifest.toml"))
        .expect("the manifest is in the tree");
    let manifest: toml::Value = toml::from_str(&text).expect("the manifest parses");
    let read = manifest["capabilities"]["files"]["read"]
        .as_array()
        .expect("files.read is a list")
        .iter()
        .map(|v| v.as_str().expect("a path").to_string())
        .collect();
    ModuleCapabilities {
        files: Some(FilesCapability { read }),
        ..Default::default()
    }
}

/// Drive the module the way the daemon does, minus the consent broker.
///
/// Enabling a capability-bearing module dials the consent broker, and there is
/// none in a test process - the first version of this test discovered that by
/// skipping, silently, which is the shape of green that means nothing. Consent
/// has its own tests; this one is about whether the module can read the files it
/// declared, so it instantiates directly with those capabilities.
#[tokio::test]
#[ignore = "needs modules/man built and a machine with man pages; see the file header"]
async fn the_module_reads_real_man_pages_and_finds_one() {
    assert!(
        Path::new("/usr/share/man/man1").is_dir(),
        "this machine has no man pages, so there is nothing to find"
    );

    let runtime = Tier1Runtime::new().expect("runtime");
    let bytes = std::fs::read(repo_root().join("modules/man/module.wasm"))
        .expect("the component is built; see the file header");
    let component = wasmtime::component::Component::new(runtime.engine(), bytes)
        .expect("the component loads");

    let mut instance = runtime
        .instantiate(
            MODULE_ID,
            &component,
            CapabilityContext::new(MODULE_ID, declared_capabilities()),
            Arc::new(UnixGraphClient::new("/tmp/arlen-test-knowledge.sock")),
            Arc::new(UnixEventEmitter::new("/tmp/arlen-test-events.sock")),
        )
        .await
        .expect("a module declaring files.read instantiates and its init reads the directory");

    // The instance exposes its store and provider, so the call is the one the
    // daemon makes, minus the manager's clamping and wire mapping.
    let results = instance
        .provider
        .arlen_waypointer_provider()
        .call_search(&mut instance.store, "ls")
        .await
        .expect("search answers");

    assert!(!results.is_empty(), "no page matched `ls`");
    let exact = results.iter().find(|r| r.title == "ls").expect("the ls page");
    assert_eq!(exact.description.as_deref(), Some("Section 1"));
    assert_eq!(exact.description_key.as_deref(), Some("man.section"));
    assert!(
        matches!(&exact.action, a if format!("{a:?}").contains("man 1 ls")),
        "the action is the command that reads it: {:?}",
        exact.action
    );

    // What the search actually cost, measured rather than asserted loosely.
    //
    // The whole reason this module exists is the question of whether a per-call
    // budget of 1 M fuel can answer a real query, and three shapes could not: a
    // scoring pass over all 4600 names, building a result per match, and cutting
    // at twenty before ranking. The prefix range costs about 110000 - eleven
    // percent - because it looks at the answer rather than the corpus.
    //
    // The check is on the ORDER of magnitude, not the exact figure, so a
    // different machine's man pages do not fail it while a return to a
    // full-corpus scan does.
    let left = instance.store.get_fuel().expect("the store meters fuel");
    assert!(
        left > 1_000_000 / 2,
        "the search should leave most of the budget unspent, {left} left"
    );

    // The other half of the answer: the module shipped a key, and the catalogue
    // it shipped alongside resolves it.
    //
    // The module's own `search` never sees a locale, so nothing inside the guest
    // can prove this. Loading the shipped `i18n/de.json` the way the daemon does
    // is what catches a message whose placeholder is misspelled or whose German
    // was never written - both of which would otherwise ship as a silently
    // English description.
    let german = arlen_modulesd::host::strings::load(
        &repo_root().join("modules/man"),
        "de",
    )
    .expect("the module ships a German catalogue");
    assert_eq!(
        arlen_modulesd::host::strings::resolve(
            Some(&german),
            exact.description_key.as_deref(),
            exact.args.as_deref(),
            exact.description.as_deref().unwrap_or_default(),
        ),
        "Abschnitt 1",
        "the shipped catalogue resolves the key with the module's own argument"
    );
}
