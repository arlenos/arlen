/// Tauri custom URI scheme handler for `module://`.
///
/// Every Tier 2 iframe loads through this handler. The flow per
/// asset request:
///
///   1. Parse the URL into `(module_id, asset_path, nonce)`.
///   2. Ask `lunaris-modulesd` for the iframe meta (`IframeLookup`).
///      If the daemon does not know the nonce, the iframe was either
///      revoked or never minted; return 404.
///   3. Load the asset from the daemon-supplied root path. Reject any
///      `..` segment so a malicious module cannot escape its bundle.
///   4. Attach the per-module CSP header derived from the manifest's
///      `[capabilities]` block.
///
/// Capability checks happen on host calls (in `lunaris-modulesd` over
/// the postMessage proxy). Asset serving here is purely about iframe
/// origin isolation, not about gating data access.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use modulesd_proto::{Request, Response};
use tauri::http::{Request as HttpRequest, Response as HttpResponse, StatusCode};

use crate::modulesd_client::ModulesdClient;

/// Strip the `?nonce=` query parameter and return `(asset_path, nonce)`.
fn parse_uri(uri: &str) -> Option<(String, String, String)> {
    // Expected: module://com.example.weather/dist/index.html?nonce=ABC
    let no_scheme = uri.strip_prefix("module://")?;
    let (host_and_path, query) = no_scheme.split_once('?').unwrap_or((no_scheme, ""));
    let (module_id, path) = host_and_path.split_once('/').unwrap_or((host_and_path, ""));
    let nonce = query
        .split('&')
        .find_map(|kv| kv.strip_prefix("nonce="))
        .unwrap_or("")
        .to_string();
    if module_id.is_empty() || nonce.is_empty() {
        return None;
    }
    Some((module_id.to_string(), path.to_string(), nonce))
}

/// Reject any path containing `..` after splitting on `/`.
fn safe_join(root: &Path, asset_path: &str) -> Option<PathBuf> {
    let mut joined = root.to_path_buf();
    for segment in asset_path.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            return None;
        }
        joined.push(segment);
    }
    Some(joined)
}

fn content_type_for(path: &Path) -> &'static str {
    match path.extension().and_then(|s| s.to_str()) {
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "application/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    }
}

fn error_response(status: StatusCode, body: &str) -> HttpResponse<Vec<u8>> {
    HttpResponse::builder()
        .status(status)
        .header("content-type", "text/plain; charset=utf-8")
        .body(body.as_bytes().to_vec())
        .unwrap_or_else(|_| HttpResponse::new(Vec::new()))
}

/// Async handler called by Tauri for every `module://` request.
pub async fn handle(
    client: Arc<ModulesdClient>,
    req: HttpRequest<Vec<u8>>,
) -> HttpResponse<Vec<u8>> {
    let uri = req.uri().to_string();
    let Some((module_id, asset_path, nonce)) = parse_uri(&uri) else {
        return error_response(StatusCode::BAD_REQUEST, "malformed module:// URI");
    };

    let resp = match client
        .call(Request::IframeLookup {
            id: String::new(),
            nonce: nonce.clone(),
        })
        .await
    {
        Ok(r) => r,
        Err(err) => {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                &format!("modulesd unreachable: {err}"),
            );
        }
    };

    let (bound_module, root_path, csp) = match resp {
        Response::IframeMeta {
            module_id,
            root_path,
            csp,
            ..
        } => (module_id, root_path, csp),
        Response::Error { message, .. } => {
            return error_response(StatusCode::NOT_FOUND, &message);
        }
        _ => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "unexpected reply"),
    };

    if bound_module != module_id {
        return error_response(
            StatusCode::FORBIDDEN,
            "iframe nonce bound to a different module",
        );
    }

    let asset_path = if asset_path.is_empty() {
        "index.html".to_string()
    } else {
        asset_path
    };

    let Some(full_path) = safe_join(Path::new(&root_path), &asset_path) else {
        return error_response(StatusCode::FORBIDDEN, "path traversal blocked");
    };

    let bytes = match tokio::fs::read(&full_path).await {
        Ok(b) => b,
        Err(_) => {
            return error_response(
                StatusCode::NOT_FOUND,
                &format!("asset not found: {asset_path}"),
            );
        }
    };

    HttpResponse::builder()
        .status(StatusCode::OK)
        .header("content-type", content_type_for(&full_path))
        .header("Content-Security-Policy", csp)
        .header("X-Module-Id", &module_id)
        .body(bytes)
        .unwrap_or_else(|_| HttpResponse::new(Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_uri_extracts_module_path_nonce() {
        let (m, p, n) = parse_uri("module://com.example.weather/dist/index.html?nonce=abc").unwrap();
        assert_eq!(m, "com.example.weather");
        assert_eq!(p, "dist/index.html");
        assert_eq!(n, "abc");
    }

    #[test]
    fn parse_uri_root_path() {
        let (m, p, n) = parse_uri("module://com.example.weather?nonce=abc").unwrap();
        assert_eq!(m, "com.example.weather");
        assert!(p.is_empty());
        assert_eq!(n, "abc");
    }

    #[test]
    fn parse_uri_rejects_missing_nonce() {
        assert!(parse_uri("module://com.example.weather/index.html").is_none());
    }

    #[test]
    fn safe_join_blocks_dotdot() {
        let root = Path::new("/usr/share/lunaris/modules/x");
        assert!(safe_join(root, "../escape.txt").is_none());
        assert!(safe_join(root, "ok/path.txt").is_some());
    }

    #[test]
    fn safe_join_strips_leading_slash() {
        let root = Path::new("/x");
        let joined = safe_join(root, "/a/b").unwrap();
        assert_eq!(joined, PathBuf::from("/x/a/b"));
    }

    #[test]
    fn content_type_picks_known_extensions() {
        assert!(content_type_for(Path::new("a.html")).starts_with("text/html"));
        assert!(content_type_for(Path::new("a.js")).starts_with("application/javascript"));
        assert!(content_type_for(Path::new("a.unknown")).starts_with("application/octet-stream"));
    }
}
