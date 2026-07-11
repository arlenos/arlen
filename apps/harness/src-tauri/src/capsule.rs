//! The capsule mint flow's coder seams (context-capsule.md §8): the three commands
//! `apps/harness/src/lib/stores/mint.ts` invokes to drive the deliberate, human-only
//! "share a slice of my context" act.
//!
//! - `capsule_scope_options`: the named things a user can share. Today those are the
//!   live projects in the knowledge graph (a project + its member files is a coherent
//!   slice the materializer already understands via `FILE_PART_OF`).
//! - `capsule_preview`: the mandatory over-share preview for a chosen scope: the base
//!   node count and, per relation type the slice follows, how far it reaches. The
//!   `0x07` materializer follows `FILE_PART_OF` only, so the preview is truthful to
//!   what the mint actually exposes; the fixed field projection excludes sensitive
//!   fields, so `hasSensitive` is false by construction.
//! - `capsule_mint`: materialize the slice via the knowledge daemon's `0x07` op, then
//!   hand it to the `capsuled` control `Mint` op (the daemon signs + stores + registers
//!   it). Mint is a human act - the daemon's control socket admits only human-UI
//!   principals (harness/settings), never the agent, so this path is never
//!   agent-reachable.

use capsuled::control_client::CapsuleControlClient;
use capsuled::mint::MintParams;
use capsuled::scope::CapsuleScope;
use os_sdk::UnixGraphClient;
use serde::Serialize;

/// The largest number of shareable named things (projects) offered.
const MAX_SCOPE_OPTIONS: usize = 50;

/// One named thing the user can share, matching `mint.ts`'s `ScopeOption`.
#[derive(Serialize)]
pub struct ScopeOption {
    /// The scope id (a project id), used as the capsule scope root.
    id: String,
    /// The human name.
    label: String,
    /// A short plain description ("12 files").
    description: String,
}

/// One relation type the slice follows and its reach, matching `mint.ts`'s
/// `PreviewRelation`.
#[derive(Serialize)]
pub struct PreviewRelation {
    /// The relation type (`FILE_PART_OF`).
    #[serde(rename = "type")]
    kind: String,
    /// A plain description of what the relation pulls in.
    label: String,
    /// How many nodes the relation reaches from the scope.
    reach: i64,
}

/// The over-share preview for a scope, matching `mint.ts`'s `Preview`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    /// The base node count the slice carries.
    base_count: i64,
    /// The relations the slice follows and their reach.
    relations: Vec<PreviewRelation>,
    /// Whether the slice would carry sensitive fields (false: the projection
    /// excludes them).
    has_sensitive: bool,
}

/// The mint receipt returned to the caller (the frontend only needs success, but the
/// handle + hash are useful for a confirmation).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MintReceipt {
    /// The revocation handle (revoke it from Settings > Privacy > Shared context).
    handle: String,
    /// The slice content hash (hex), the capsule identity.
    slice_hash: String,
}

/// Resolve the knowledge graph socket the shared 3-tier way (env override, else
/// `$XDG_RUNTIME_DIR/arlen/knowledge.sock`, else `/run/arlen/knowledge.sock`).
fn graph_client() -> UnixGraphClient {
    let path = os_sdk::runtime::socket_path("ARLEN_DAEMON_SOCKET", "knowledge.sock");
    UnixGraphClient::new(path.to_string_lossy().into_owned())
}

/// Escape a value interpolated into a single-quoted Cypher string literal.
fn escape_cypher(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

/// Read a string cell from a typed row, or an empty string.
fn cell_str(row: &std::collections::HashMap<String, serde_json::Value>, key: &str) -> String {
    row.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

/// Read an integer cell from a typed row, or zero.
fn cell_i64(row: &std::collections::HashMap<String, serde_json::Value>, key: &str) -> i64 {
    row.get(key).and_then(|v| v.as_i64()).unwrap_or(0)
}

/// The named things the user can share: the live projects and their member-file
/// counts. A project with no name is labelled by its id; a project with its own
/// description keeps it, else the description is derived from the file count.
#[tauri::command]
pub async fn capsule_scope_options() -> Result<Vec<ScopeOption>, String> {
    let client = graph_client();
    // One aggregation: each live project with the count of its live member files.
    // Non-aggregated return keys are the implicit group keys (Kuzu grouping).
    let cypher = format!(
        "MATCH (p:Project) WHERE p.expired_at IS NULL \
         OPTIONAL MATCH (f:File)-[r:FILE_PART_OF]->(p) \
         WHERE r.invalid_at IS NULL AND r.expired_at IS NULL \
         RETURN p.id AS id, p.name AS name, p.description AS description, \
         count(f) AS files ORDER BY files DESC LIMIT {MAX_SCOPE_OPTIONS}"
    );
    let rows = client.query_rows(&cypher).await.map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let id = cell_str(&row, "id");
            let name = cell_str(&row, "name");
            let own_desc = cell_str(&row, "description");
            let files = cell_i64(&row, "files");
            let label = if name.is_empty() { id.clone() } else { name };
            let description = if own_desc.is_empty() {
                format!("{files} {}", if files == 1 { "file" } else { "files" })
            } else {
                own_desc
            };
            ScopeOption { id, label, description }
        })
        .collect())
}

/// The over-share preview for a project scope: the live member files (the base + the
/// `FILE_PART_OF` reach). The materializer follows only `FILE_PART_OF` and applies a
/// fixed non-sensitive projection, so the preview names exactly one relation and
/// reports no sensitive fields.
#[tauri::command]
pub async fn capsule_preview(scope_id: String) -> Result<Preview, String> {
    let client = graph_client();
    let id = escape_cypher(&scope_id);
    let cypher = format!(
        "MATCH (f:File)-[r:FILE_PART_OF]->(p:Project {{id: '{id}'}}) \
         WHERE r.invalid_at IS NULL AND r.expired_at IS NULL \
         RETURN count(f) AS files"
    );
    let rows = client.query_rows(&cypher).await.map_err(|e| e.to_string())?;
    let files = rows.first().map(|r| cell_i64(r, "files")).unwrap_or(0);
    Ok(Preview {
        base_count: files,
        relations: vec![PreviewRelation {
            kind: "FILE_PART_OF".to_string(),
            label: "files in this project".to_string(),
            reach: files,
        }],
        // The capsule projection (`CAPSULE_LABELS`) excludes every sensitive field, so
        // a minted slice never carries one; `includeSensitive` is inert day-one.
        has_sensitive: false,
    })
}

/// Mint a capsule for the chosen scope: materialize the slice (knowledge `0x07`), then
/// sign + store + register it via the `capsuled` control `Mint` op. The materialize is
/// async (os-sdk), the control client is a synchronous one-shot, so the mint runs on a
/// blocking thread. Mint-requires-human is enforced daemon-side (the control socket
/// withholds the signing key from any non-human-UI caller).
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn capsule_mint(
    scope_id: String,
    audience: String,
    expiry: String,
    op_count: String,
    dropped: Vec<String>,
    include_sensitive: bool,
) -> Result<MintReceipt, String> {
    // Files are one hop from their project (File -[FILE_PART_OF]-> Project), so a
    // one-hop expansion from the project root carries the project and its files.
    let scope = CapsuleScope { roots: vec![scope_id.clone()], expand_hops: 1 };

    // Materialize the frozen slice as of now over the live graph.
    let client = graph_client();
    let slice = client
        .materialize_capsule(&scope)
        .await
        .map_err(|e| e.to_string())?;

    // Look up the project's name for the capsule label (the list surface shows it).
    let label = project_label(&client, &scope_id).await.unwrap_or_else(|| scope_id.clone());

    // A plain, relation-type-level summary for the active-capsules surface. Day-one the
    // slice follows FILE_PART_OF only; `dropped`/`includeSensitive` are recorded so the
    // summary is honest even though the 0x07 op carries no per-relation filter yet.
    let files = slice.nodes.iter().filter(|n| n.label == "File").count();
    let mut scope_summary = format!(
        "{files} {} in this project (FILE_PART_OF)",
        if files == 1 { "file" } else { "files" }
    );
    if !dropped.is_empty() {
        scope_summary.push_str(&format!("; {} relation(s) dropped", dropped.len()));
    }
    if include_sensitive {
        scope_summary.push_str("; sensitive fields requested (excluded by projection)");
    }

    let params = MintParams {
        scope,
        // The audience key is signed into the grant but not enforced on the
        // same-machine path (SO_PEERCRED is the gate); day-one every capsule's audience
        // is the local machine, so a placeholder key is recorded and the ledger stamps
        // "this machine". The form's `audience` value is kept in the summary context.
        audience_hex: audience_to_hex(&audience),
        expires_at_micros: expiry_to_micros(&expiry),
        max_ops: op_count.trim().parse::<u64>().unwrap_or(20),
        originating_user: current_user(),
        label,
        scope_summary,
    };

    let receipt = tokio::task::spawn_blocking(move || {
        let control = CapsuleControlClient::at_default_path().map_err(|e| e.to_string())?;
        control.mint(slice, params).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("capsule_mint task failed: {e}"))??;

    Ok(MintReceipt {
        handle: receipt.handle,
        slice_hash: receipt.slice_hash,
    })
}

/// The live project's name, for the capsule label.
async fn project_label(client: &UnixGraphClient, scope_id: &str) -> Option<String> {
    let id = escape_cypher(scope_id);
    let cypher = format!("MATCH (p:Project {{id: '{id}'}}) RETURN p.name AS name LIMIT 1");
    let rows = client.query_rows(&cypher).await.ok()?;
    let name = cell_str(rows.first()?, "name");
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Map the form's audience choice to an audience-key hex. Day-one the same-machine
/// path does not enforce the audience, so an unrecognized choice records a zero
/// placeholder key (a valid 32-byte hex); a caller may pass a real verifying-key hex.
fn audience_to_hex(audience: &str) -> String {
    let a = audience.trim();
    // Accept a real 64-hex verifying key verbatim; otherwise the local-machine
    // placeholder.
    if a.len() == 64 && a.bytes().all(|b| b.is_ascii_hexdigit()) {
        a.to_ascii_lowercase()
    } else {
        "00".repeat(32)
    }
}

/// Turn a form expiry token ("1h"/"1d"/"1w"/"2w"/"1m") into an absolute expiry in
/// epoch microseconds. Unknown tokens fall back to one week (mandatory expiry).
fn expiry_to_micros(expiry: &str) -> i64 {
    let now = now_micros();
    let day = 86_400i64 * 1_000_000;
    let delta = match expiry.trim() {
        "1h" => 3_600i64 * 1_000_000,
        "1d" => day,
        "3d" => 3 * day,
        "1w" => 7 * day,
        "2w" => 14 * day,
        "1m" => 30 * day,
        _ => 7 * day,
    };
    now.saturating_add(delta)
}

/// The minting user's name (best-effort), for the grant's `originating_user`.
fn current_user() -> String {
    std::env::var("USER")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "user".to_string())
}

/// Now, in microseconds since the epoch.
fn now_micros() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiry_tokens_map_to_a_future_stamp() {
        let now = now_micros();
        assert!(expiry_to_micros("1w") > now);
        assert!(expiry_to_micros("1h") > now);
        // Unknown falls back to a week, still in the future.
        assert!(expiry_to_micros("nonsense") > now);
        // Longer windows are further out.
        assert!(expiry_to_micros("2w") > expiry_to_micros("1w"));
    }

    #[test]
    fn audience_maps_real_hex_verbatim_else_placeholder() {
        let key = "ab".repeat(32);
        assert_eq!(audience_to_hex(&key), key);
        assert_eq!(audience_to_hex("this-machine"), "00".repeat(32));
        // A too-short or non-hex value is the placeholder, never passed through.
        assert_eq!(audience_to_hex("zz"), "00".repeat(32));
    }

    #[test]
    fn cypher_escaping_neutralizes_quotes() {
        assert_eq!(escape_cypher("a'b"), "a\\'b");
        assert_eq!(escape_cypher("a\\b"), "a\\\\b");
    }
}
