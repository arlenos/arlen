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

/// Stage the module the way an install would: manifest, component, catalogue.
///
/// The `i18n/` directory is not decoration here. The manager resolves a result's
/// key against `<module dir>/i18n/<locale>.json`, so a staging that copied only
/// the manifest and the wasm would leave the German assertion below testing an
/// absent catalogue's fallback rather than the catalogue.
fn stage(into: &Path) {
    let root = repo_root();
    let src = root.join("modules/man");
    let wasm = src.join("module.wasm");
    assert!(
        wasm.is_file(),
        "the component is not built. See this file's header for the command.",
    );
    let dir = into.join(MODULE_ID);
    std::fs::create_dir_all(dir.join("i18n")).unwrap();
    std::fs::copy(src.join("manifest.toml"), dir.join("manifest.toml")).unwrap();
    std::fs::copy(&wasm, dir.join("module.wasm")).unwrap();
    for locale in ["en", "de"] {
        let name = format!("{locale}.json");
        std::fs::copy(src.join("i18n").join(&name), dir.join("i18n").join(&name)).unwrap();
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
            // Both point at nothing on purpose. This module declares neither
            // graph nor events, so `populate_linker` never puts either import
            // on the linker and the guest has no way to reach them; a path that
            // resolves would only make that harder to see.
            Arc::new(UnixGraphClient::new("/tmp/arlen-man-test-unreachable.sock")),
            Arc::new(UnixEventEmitter::new("/tmp/arlen-man-test-unreachable.sock")),
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

/// The same module, driven the way the shell drives it: discovery, consent,
/// search, and the description resolved into German on the way out.
///
/// **The direct-instantiation test above deliberately skips two things, and they
/// are the two that only exist in the daemon.** A capability-bearing module has
/// to pass the consent broker before it goes active, and the manager - not the
/// guest - resolves a result's key against the module's catalogue. Neither is
/// reachable without a broker on the socket, so this test brings one: a listener
/// that reads the intake frame and answers `allowed_once`.
///
/// It also asserts what the broker was TOLD, because that is the half of ruling 1
/// that a passing search cannot show: the grant has to name the prefix the module
/// declared, not the word "filesystem". A dialog saying "this extension wants to
/// read your files" would be true and useless.
#[tokio::test]
#[ignore = "needs modules/man built; see the file header"]
async fn the_daemon_hosts_it_end_to_end_and_answers_in_german() {
    use std::sync::Mutex;

    assert!(
        Path::new("/usr/share/man/man1").is_dir(),
        "this machine has no man pages, so there is nothing to find"
    );

    // A German session. `chosen_locale` reads `~/.config/arlen/locale.toml`
    // through `dirs`, so pointing XDG_CONFIG_HOME at a temp tree is the whole
    // configuration - deliberately not LANG, which the reader ignores.
    let config = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(config.path().join("arlen")).unwrap();
    std::fs::write(
        config.path().join("arlen/locale.toml"),
        "[locale]\nui = \"de\"\n",
    )
    .unwrap();
    std::env::set_var("XDG_CONFIG_HOME", config.path());

    // The broker. It records what it was asked so the assertion below is about
    // the real intake frame rather than a re-derived summary.
    let runtime = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(runtime.path().join("arlen")).unwrap();
    std::env::set_var("XDG_RUNTIME_DIR", runtime.path());
    let asked: Arc<Mutex<Option<serde_json::Value>>> = Arc::new(Mutex::new(None));
    let listener =
        tokio::net::UnixListener::bind(arlen_modulesd::consent::intake_socket_path()).unwrap();
    let recorded = Arc::clone(&asked);
    tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut header = [0u8; 4];
            if stream.read_exact(&mut header).await.is_err() {
                continue;
            }
            let mut body = vec![0u8; u32::from_le_bytes(header) as usize];
            if stream.read_exact(&mut body).await.is_err() {
                continue;
            }
            *recorded.lock().unwrap() = serde_json::from_slice(&body).ok();
            let reply = serde_json::to_vec(&arlen_consent_contract::IntakeResult::Decided {
                outcome: arlen_consent_contract::ConsentOutcome::AllowedOnce,
            })
            .unwrap();
            let _ = stream
                .write_all(&(reply.len() as u32).to_le_bytes())
                .await;
            let _ = stream.write_all(&reply).await;
        }
    });

    let mods = tempfile::tempdir().unwrap();
    std::env::set_var("ARLEN_USER_MODULES_DIR", mods.path());
    stage(mods.path());

    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let manager: Arc<arlen_modulesd::Manager> = arlen_modulesd::Manager::new(tx).unwrap();
    manager.discover().await;

    let resp = manager
        .handle_request(modulesd_proto::Request::SetEnabled {
            id: "1".into(),
            module_id: MODULE_ID.into(),
            enabled: true,
        })
        .await;
    assert!(
        !matches!(resp, modulesd_proto::Response::Error { .. }),
        "the broker granted, so enabling succeeds: {resp:?}"
    );

    let request = asked.lock().unwrap().clone().expect("the broker was asked");
    let scope = request["scope"].as_str().unwrap_or_default();
    assert!(
        scope.contains("/usr/share/man"),
        "the grant names the prefix the manifest declared: {request:?}"
    );
    assert_eq!(
        request["on_behalf_of"].as_str(),
        Some(MODULE_ID),
        "the grant is attributed to the module, not to modulesd"
    );

    let resp = manager
        .handle_request(modulesd_proto::Request::WaypointerSearch {
            id: "2".into(),
            module_id: MODULE_ID.into(),
            query: "ls".into(),
        })
        .await;
    let modulesd_proto::Response::WaypointerResults { results, .. } = resp else {
        panic!("expected results, got {resp:?}");
    };
    let exact = results.iter().find(|r| r.title == "ls").expect("the ls page");
    assert_eq!(
        exact.description.as_deref(),
        Some("Abschnitt 1"),
        "the manager resolved the module's key against its own catalogue"
    );
}
