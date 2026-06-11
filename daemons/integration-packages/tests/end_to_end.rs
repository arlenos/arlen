//! End-to-end: parse an adapter, confine and resolve its source, then read and
//! edit a setting through the same cap-std root.
//!
//! This is the composition the privileged Settings app drives. The per-module
//! unit tests cover the pieces in isolation; this proves they wire together over
//! a real fixture file: an adapter names a config under the user-config
//! allowlist, the source glob resolves to the concrete file under cap-std
//! confinement, and a verified format-preserving edit comes back. The display
//! read (`read_setting`) goes through the S18-B parse sandbox and so needs the
//! worker binary; the edit path (`read_text_confined` + `prepare_edit`) is
//! sandbox-free and is what this exercises end-to-end.

use arlen_integration_packages::{
    prepare_edit, read_text_confined, AdapterManifest, ConfigValue, Resolution,
};
use arlen_integration_packages::resolve::{confined_root, glob_under, resolve};

#[test]
fn adapter_to_resolve_to_edit_over_a_real_fixture() {
    // A temp HOME holding an app config under the allowlisted ~/.config.
    let home = tempfile::tempdir().unwrap();
    let cfg_dir = home.path().join(".config/app");
    std::fs::create_dir_all(&cfg_dir).unwrap();
    std::fs::write(
        cfg_dir.join("config.toml"),
        "# app config\nport = 8080\nname = \"app\"\n",
    )
    .unwrap();

    // An adapter naming that config and exposing the port as an editable int.
    let manifest = r#"
        [adapter]
        schema_version = "1.0"
        write_strategy = "anytime"
        [sources]
        cfg = { path = "~/.config/app/config.toml", format = "toml" }
        [[settings]]
        key = "port"
        source = "cfg"
        label = "Port"
        type = "int"
    "#;
    let m = AdapterManifest::parse(manifest, home.path()).unwrap();
    let source = &m.sources["cfg"];
    let setting = &m.settings[0];

    // Confine to the allowlist root, glob the source, resolve the instance.
    let (dir, relative) = confined_root(&source.path, home.path()).unwrap();
    let matches = glob_under(&dir, &relative);
    assert_eq!(matches.len(), 1, "the literal path matches exactly one file");
    let rel = match resolve(source.instance_strategy, &matches) {
        Resolution::One(rel) => rel,
        other => panic!("expected a single resolved file, got {other:?}"),
    };

    // Read the current text through the SAME confinement, prepare a verified edit.
    let text = read_text_confined(&dir, &rel).unwrap();
    assert!(text.contains("port = 8080"));
    let candidate = prepare_edit(&text, source, setting, &ConfigValue::Int(9090)).unwrap();
    assert!(candidate.contains("port = 9090"), "the edit applied");
    assert!(candidate.contains("# app config"), "the comment is preserved");
    assert!(candidate.contains("name = \"app\""), "the sibling key is untouched");
}

#[test]
fn an_adapter_source_outside_the_allowlist_never_resolves() {
    // A manifest naming a system file is refused at parse time (the declared-path
    // half of the containment), so the resolve/edit flow is never reached.
    let home = tempfile::tempdir().unwrap();
    let manifest = r#"
        [adapter]
        schema_version = "1.0"
        write_strategy = "anytime"
        [sources]
        evil = { path = "/etc/passwd", format = "flat" }
    "#;
    assert!(
        AdapterManifest::parse(manifest, home.path()).is_err(),
        "a source outside the user-config allowlist must be refused"
    );
}
