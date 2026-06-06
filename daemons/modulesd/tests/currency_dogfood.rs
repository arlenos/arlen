/// End-to-end test against the ds#77 dogfood module:
/// `examples/modules/com.example.currency`.
///
/// Runs the full lifecycle on a temp module dir:
///   1. Copy the example bundle into a temp `LUNARIS_USER_MODULES_DIR`.
///   2. Boot the manager and run discovery.
///   3. Mint a Tier 2 iframe URL.
///   4. Look up the resulting nonce; confirm CSP and root path.
///   5. Issue an allowed and a denied network host call; confirm the
///      gate matches the manifest.
///
/// This is the contract test that ties manifest, discovery,
/// permission profile, iframe broker, and host gate together.

use std::path::Path;
use std::sync::Arc;

use modulesd_proto::{ErrorCode, HostCall, HostReply, Request, Response};
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
    // Tests run from the crate root, so the example module is two
    // levels up + into the examples tree.
    let mut p = std::env::current_dir().unwrap();
    p.push("../examples/modules/com.example.currency");
    p
}

#[tokio::test]
async fn currency_module_discoverable_after_copy_and_capabilities_enforced() {
    let example = example_module_path();
    if !example.exists() {
        // CI may run with the example tree absent; treat as skip.
        eprintln!("skipping: example module not present at {}", example.display());
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("LUNARIS_USER_MODULES_DIR", tmp.path());
    copy_dir_recursive(&example, &tmp.path().join("com.example.currency"));

    let (tx, _rx) = broadcast::channel(16);
    let manager: Arc<Manager> = Manager::new(tx).unwrap();
    manager.discover().await;

    // 1. Module appears in the list, classified Tier 2 (no module.wasm).
    let resp = manager
        .handle_request(Request::ListModules { id: "1".into() })
        .await;
    let mut found_id = None;
    if let Response::ModuleList { modules, .. } = resp {
        for m in modules {
            if m.id == "com.example.currency" {
                found_id = Some(m.id);
                assert_eq!(m.tier, modulesd_proto::ModuleTier::Iframe);
                assert!(m.enabled);
                assert!(!m.failed);
            }
        }
    }
    assert!(found_id.is_some(), "currency module not discovered");

    // 2. Mint an iframe URL and verify the bound nonce resolves.
    let resp = manager
        .handle_request(Request::IframeMint {
            id: "2".into(),
            module_id: "com.example.currency".into(),
            slot: "topbar".into(),
        })
        .await;
    let nonce = match resp {
        Response::IframeIssued { nonce, url, .. } => {
            assert!(url.starts_with("module://com.example.currency/dist/"));
            nonce
        }
        other => panic!("expected IframeIssued, got {other:?}"),
    };

    // 3. Lookup roundtrip carries CSP and the dist root.
    let resp = manager
        .handle_request(Request::IframeLookup {
            id: "3".into(),
            nonce: nonce.clone(),
        })
        .await;
    match resp {
        Response::IframeMeta {
            module_id,
            csp,
            root_path,
            ..
        } => {
            assert_eq!(module_id, "com.example.currency");
            assert!(csp.contains("api.exchangerate.host"));
            assert!(csp.contains("frame-ancestors 'self'"));
            assert!(root_path.ends_with("dist"));
        }
        other => panic!("expected IframeMeta, got {other:?}"),
    }

    // 4. Allowed network call passes the capability gate at the
    //    SDK level (`check_fetch`). We deliberately do NOT invoke
    //    the full `HostCall::NetworkFetch` round-trip here because
    //    that would launch a real reqwest call against
    //    `api.exchangerate.host`; on offline / firewalled CI the
    //    30 s timeout would block the suite. Wiremock-driven
    //    end-to-end coverage lives in `network_e2e.rs`.
    use arlen_modulesd::host::{network::check_fetch, CapabilityContext};
    use arlen_modules::{ModuleCapabilities, NetworkCapability};
    let mut caps = ModuleCapabilities::default();
    caps.network = Some(NetworkCapability {
        allowed_domains: vec!["api.exchangerate.host".into()],
    });
    let ctx = CapabilityContext::new("com.example.currency", caps);
    check_fetch(
        &ctx,
        "https://api.exchangerate.host/latest?base=USD&symbols=EUR",
    )
    .expect("allowlisted https URL must pass the capability check");
    let _ = nonce;

    // 5. Denied network call (host outside the manifest allowlist).
    let resp = manager
        .handle_request(Request::HostCall {
            id: "5".into(),
            nonce: nonce.clone(),
            call: HostCall::NetworkFetch {
                url: "https://api.evil.com/exfil".into(),
                headers: vec![],
            },
        })
        .await;
    match resp {
        Response::HostReply { reply, .. } => match reply {
            HostReply::Error { code, .. } => assert_eq!(code, ErrorCode::PermissionDenied),
            other => panic!("expected PermissionDenied, got {other:?}"),
        },
        other => panic!("expected HostReply, got {other:?}"),
    }

    std::env::remove_var("LUNARIS_USER_MODULES_DIR");
}
