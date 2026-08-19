//! Speak to the REAL print portal, when a session bus offers one.
//!
//! The unit tests prove the request path is derived correctly and that the
//! outcomes serialise; neither proves the frontend accepts what we send, which
//! is the only thing that matters for a feature whose whole job is to reach
//! another process. This asks the actual portal.
//!
//! `#[ignore]`d: it hands a document to a print dialog on a machine that has a
//! desktop, which does not belong in a unit-test run.
//! `cargo test --test print_portal -- --ignored` drives it.

use tauri_plugin_arlen_shell::print::{print_file, PrintOutcome};

/// Is there a portal to talk to at all?
///
/// Reported as SKIPPED rather than passed when there is not: a test that
/// silently tests nothing is worse than an absent one.
async fn portal_present() -> bool {
    let Ok(c) = zbus::Connection::session().await else {
        return false;
    };
    let Ok(proxy) = zbus::fdo::DBusProxy::new(&c).await else {
        return false;
    };
    proxy
        .name_has_owner("org.freedesktop.portal.Desktop".try_into().unwrap())
        .await
        .unwrap_or(false)
}

#[tokio::test]
#[ignore = "talks to the real portal and opens a print dialog; run with --ignored"]
async fn the_portal_accepts_a_document_from_this_client() {
    if !portal_present().await {
        eprintln!("SKIPPED: no org.freedesktop.portal.Desktop on this bus, so nothing was verified");
        return;
    }

    let path = std::env::temp_dir().join("arlen-print-probe.txt");
    std::fs::write(&path, "Arlen print portal probe.\n").unwrap();

    // Whatever comes back, the call itself must have been ACCEPTED: an error
    // means the document never reached the portal, which is the failure this
    // test exists to catch. A dialog nobody answers is `NoAnswerYet`, and that
    // is a pass - the portal took the document.
    match print_file(path.display().to_string()).await {
        Ok(o) => {
            eprintln!("the portal answered: {o:?}");
            assert!(matches!(
                o,
                PrintOutcome::Sent
                    | PrintOutcome::Cancelled
                    | PrintOutcome::Refused
                    | PrintOutcome::NoAnswerYet
            ));
        }
        Err(e) => panic!("the portal refused the document: {e}"),
    }
}
