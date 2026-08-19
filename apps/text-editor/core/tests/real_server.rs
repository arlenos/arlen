//! Speak to a REAL language server, when one is installed.
//!
//! Everything else in this crate is tested against `cat` and hand-written
//! messages, which proves the framing and the rules but not that a real server
//! accepts what we send. `rust-analyzer` is on most machines that build this
//! tree; where it is not, this reports that it SKIPPED rather than passing
//! quietly - a test that silently tests nothing is worse than an absent one.
//!
//! `#[ignore]`d: it starts a process that indexes a project, which does not
//! belong in a unit-test run. `cargo test -- --ignored` drives it.

use std::path::Path;

use arlen_text_editor_core::host::Server;
use arlen_text_editor_core::session::{Event, Phase, Session};

/// A rust-analyzer that actually RUNS, not a path that exists.
///
/// `~/.cargo/bin/rust-analyzer` is a rustup proxy and is present whether or not
/// the component is installed; without it the proxy prints "Unknown binary
/// 'rust-analyzer' in official toolchain" and exits. The first version of this
/// checked `is_file`, found the proxy, and the test failed with "the server
/// keeps talking: Closed" - which reads as a bug in the client rather than as a
/// missing tool. Ask the binary whether it is what it claims.
fn rust_analyzer() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let candidates = [format!("{home}/.cargo/bin/rust-analyzer"), "rust-analyzer".to_string()];
    candidates.into_iter().find(|bin| {
        if !bin.contains('/') && !Path::new("/usr/bin/rust-analyzer").is_file() {
            return false;
        }
        std::process::Command::new(bin)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success() && o.stdout.starts_with(b"rust-analyzer"))
    })
}

#[test]
#[ignore = "starts a real language server; run with --ignored"]
fn rust_analyzer_completes_the_handshake_and_answers_about_a_file() {
    let Some(bin) = rust_analyzer() else {
        eprintln!("SKIPPED: no rust-analyzer at ~/.cargo/bin, so nothing was verified");
        return;
    };

    // A tiny crate of its own, so the server has a manifest to find and is not
    // asked to index whatever tree the test happens to run in.
    let dir = std::env::temp_dir().join("arlen-lsp-probe");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    // A deliberate error: `x` is never declared, so a working server has
    // something true to say about this file.
    let text = "fn main() {\n    let _ = x;\n}\n";
    std::fs::write(dir.join("src/main.rs"), text).unwrap();

    let mut server = Server::spawn(&bin, &[], &dir).expect("rust-analyzer starts");
    let uri = format!("file://{}", dir.display());
    let (mut session, initialize) = Session::start(&uri);
    server.send(&initialize).expect("send initialize");

    let mut ready = false;
    for _ in 0..50 {
        let msg = server.receive().expect("the server keeps talking");
        let (out, events) = session.receive(&msg);
        for o in &out {
            server.send(o).expect("send");
        }
        if events.contains(&Event::Ready) {
            ready = true;
            break;
        }
    }
    assert!(ready, "rust-analyzer never completed the handshake");
    assert_eq!(session.phase(), Phase::Ready);

    let file = format!("file://{}", dir.join("src/main.rs").display());
    let open = session.did_open(&file, "rust", text).expect("the session is ready");
    server.send(&open).expect("send didOpen");

    // Diagnostics arrive when the server has finished thinking. It emits plenty
    // of progress and log messages first, which is why this reads a stream
    // rather than expecting the next message to be the answer.
    let mut saw = None;
    for _ in 0..400 {
        let Ok(msg) = server.receive() else { break };
        let (out, events) = session.receive(&msg);
        for o in &out {
            let _ = server.send(o);
        }
        for e in events {
            if let Event::Diagnostics { uri, items } = e {
                if uri == file && !items.is_empty() {
                    saw = Some(items);
                }
            }
        }
        if saw.is_some() {
            break;
        }
    }

    let items = saw.expect("rust-analyzer published no diagnostics for a file with an undeclared name");
    assert!(
        items.iter().any(|d| d.message.contains('x')),
        "the diagnostics do not mention the undeclared name: {items:?}"
    );
    // Line 1, zero-based: the second line of the file, where `x` is.
    assert!(items.iter().any(|d| d.line == 1), "no diagnostic on the offending line: {items:?}");
}
