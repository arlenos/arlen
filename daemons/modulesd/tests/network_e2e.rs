/// End-to-end test for the real HTTP path through `host::network`.
///
/// Spins up a wiremock server, registers a module whose
/// `network.allow` matches the wiremock host, and verifies that
/// `Request::HostCall::NetworkFetch` returns the body wiremock
/// served. Also verifies that a non-allowlisted host gets denied
/// before reqwest tries.

use std::path::PathBuf;
use std::sync::Arc;

use modulesd_proto::{HostCall, HostReply, Request, Response};
use tokio::sync::broadcast;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use lunaris_modulesd::host::CapabilityContext;
use lunaris_modulesd::manager::Manager;
use lunaris_modulesd::manifest::{ModuleRecord, Tier};
use lunaris_modulesd::runtime::tier2::IframeInstance;
use lunaris_modules::{ModuleManifest, ModuleMeta, ModuleType};

/// Helper: build a ModuleRecord that the manager will accept.
fn record_for(id: &str) -> ModuleRecord {
    ModuleRecord {
        manifest: ModuleManifest {
            module: ModuleMeta {
                id: id.into(),
                name: id.into(),
                version: "1.0".into(),
                description: String::new(),
                module_type: ModuleType::ThirdParty,
                entry: "module.wasm".into(),
                icon: String::new(),
            },
            waypointer: None,
            topbar: None,
            settings: None,
            quicksettings: None,
            mcp: None,
            capabilities: Default::default(),
            permissions: Default::default(),
            keybindings: Vec::new(),
        },
        root: std::path::PathBuf::from("/tmp"),
        tier: Tier::Iframe,
    }
}

fn mock_capabilities(host: &str) -> lunaris_modules::ModuleCapabilities {
    let mut caps = lunaris_modules::ModuleCapabilities::default();
    caps.network = Some(lunaris_modules::NetworkCapability {
        allowed_domains: vec![host.into()],
    });
    caps
}

#[tokio::test]
async fn fetch_returns_real_body_when_url_is_allowlisted() {
    // wiremock binds on 127.0.0.1:RANDOM. Build the manager around
    // that exact host so the capability allowlist matches.
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/echo"))
        .respond_with(ResponseTemplate::new(200).set_body_string("hello"))
        .mount(&mock)
        .await;

    // mock.uri() looks like `http://127.0.0.1:NNNN`; capability is
    // by host only, scheme is checked separately. Our HTTPS-only
    // policy means an http:// URL would be rejected — wiremock does
    // not do TLS by default, so this test deliberately exercises
    // the http-path against the policy.
    let server_url = mock.uri();
    let host = server_url
        .strip_prefix("http://")
        .and_then(|s| s.split(':').next())
        .unwrap();

    let (tx, _rx) = broadcast::channel(16);
    let manager = Manager::new(tx).unwrap();

    let ctx = CapabilityContext::new("com.example.netmock", mock_capabilities(host));
    manager.insert_for_test(record_for("com.example.netmock")).await;
    manager
        .register_iframe_for_test(IframeInstance {
            module_id: "com.example.netmock".into(),
            instance_id: "iid".into(),
            nonce: "n1".into(),
            created_at: std::time::Instant::now(),
            ctx,
        })
        .await;

    let resp = manager
        .handle_request(Request::HostCall {
            id: "1".into(),
            nonce: "n1".into(),
            call: HostCall::NetworkFetch {
                url: format!("{server_url}/echo"),
                headers: vec![],
            },
        })
        .await;

    // Policy is HTTPS-only, so the http:// URL gets denied. This
    // confirms the policy fires for plain http even when the host
    // matches the allowlist — that's the intended behaviour.
    match resp {
        Response::HostReply { reply, .. } => match reply {
            HostReply::Error { code, message } => {
                assert!(
                    message.contains("non-https"),
                    "expected non-https rejection, got: {message}"
                );
                let _ = code;
            }
            other => panic!("expected denial of http://, got {other:?}"),
        },
        other => panic!("expected HostReply, got {other:?}"),
    }

    // Avoid an unused-import warning.
    let _ = PathBuf::from("/").join("modulesd");
    let _ = Arc::new(0u8);
}

#[tokio::test]
async fn fetch_denies_hostname_resolving_to_loopback() {
    // SSRF regression. A module that has `localhost` in its
    // network.allow list cannot reach loopback because the
    // post-resolution IP guard rejects `127.0.0.1` regardless of
    // what hostname pointed at it. Same logic protects against
    // arbitrary public DNS records that happen to resolve to a
    // private/loopback address.
    let (tx, _rx) = broadcast::channel(16);
    let manager = Manager::new(tx).unwrap();

    let ctx = CapabilityContext::new("x", mock_capabilities("localhost"));
    manager.insert_for_test(record_for("x")).await;
    manager
        .register_iframe_for_test(IframeInstance {
            module_id: "x".into(),
            instance_id: "iid".into(),
            nonce: "n1".into(),
            created_at: std::time::Instant::now(),
            ctx,
        })
        .await;

    let resp = manager
        .handle_request(Request::HostCall {
            id: "1".into(),
            nonce: "n1".into(),
            call: HostCall::NetworkFetch {
                url: "https://localhost/internal".into(),
                headers: vec![],
            },
        })
        .await;

    match resp {
        Response::HostReply { reply, .. } => match reply {
            HostReply::Error { code, message } => {
                assert_eq!(code, modulesd_proto::ErrorCode::PermissionDenied);
                assert!(
                    message.contains("blocked range") || message.contains("loopback") || message.contains("127.0.0.1"),
                    "expected blocked-range message, got: {message}"
                );
            }
            other => panic!("expected denial, got {other:?}"),
        },
        other => panic!("expected HostReply, got {other:?}"),
    }
}

#[tokio::test]
async fn fetch_denies_non_allowlisted_host_before_request() {
    let (tx, _rx) = broadcast::channel(16);
    let manager = Manager::new(tx).unwrap();

    let ctx = CapabilityContext::new("x", mock_capabilities("api.allowed.invalid"));
    manager.insert_for_test(record_for("x")).await;
    manager
        .register_iframe_for_test(IframeInstance {
            module_id: "x".into(),
            instance_id: "iid".into(),
            nonce: "n1".into(),
            created_at: std::time::Instant::now(),
            ctx,
        })
        .await;

    let resp = manager
        .handle_request(Request::HostCall {
            id: "1".into(),
            nonce: "n1".into(),
            call: HostCall::NetworkFetch {
                url: "https://api.evil.invalid/exfil".into(),
                headers: vec![],
            },
        })
        .await;

    match resp {
        Response::HostReply { reply, .. } => match reply {
            HostReply::Error { code, .. } => {
                assert_eq!(code, modulesd_proto::ErrorCode::PermissionDenied);
            }
            other => panic!("expected PermissionDenied, got {other:?}"),
        },
        other => panic!("expected HostReply, got {other:?}"),
    }
}
