//! Living Capability Graph emission (living-capability-graph.md §4).
//!
//! When a token is minted, exactly one place holds the app identity, the fully
//! resolved declared scopes, and a fresh token id together. From there
//! [`emit_grant_node`] projects the capability into the graph: the `App`
//! principal, the `Grant` node (its declared ceiling as canonical JSON plus the
//! birth lifecycle state), the `EntityType` markers and `GRANTS` edges for each
//! reachable type, and the supersession of the app's prior non-revoked nodes.
//!
//! It runs raw Cypher `MERGE` on the serial graph thread (the same mechanism
//! `promotion.rs` uses), never the `0x02` write socket, so it has no dependency
//! on the bitemporal node-create engine change. Emission failure must be logged
//! and swallowed by the caller, never failing the mint: a graph hiccup must not
//! deny an app a validly-signed token, only degrade the browse projection.

use anyhow::Result;

use crate::graph::GraphHandle;
use crate::time;
use crate::token::CapabilityToken;
use crate::utils::escape_cypher;

/// The display label for a grantable type string: the segment after the last
/// `.` (`"system.File"` -> `"File"`), or the whole string if it has no dot.
fn type_label(entity_type: &str) -> &str {
    entity_type.rsplit('.').next().unwrap_or(entity_type)
}

/// The distinct entity types this token can reach (read and write scopes), the
/// queryable `GRANTS` projection of the declared ceiling. Sorted and deduped so
/// the projection is deterministic.
fn reachable_types(token: &CapabilityToken) -> Vec<String> {
    let mut types: Vec<String> = token
        .read_scopes
        .iter()
        .chain(token.write_scopes.iter())
        .map(|s| s.entity_type.clone())
        .collect();
    types.sort();
    types.dedup();
    types
}

/// The token's declared ceiling as canonical JSON of its four scope collections
/// (the faithful record of what the profile granted, the same serialization the
/// token already supports).
fn declared_ceiling_json(token: &CapabilityToken) -> String {
    serde_json::json!({
        "read": token.read_scopes,
        "write": token.write_scopes,
        "relations": token.relation_scopes,
        "instance": token.instance_scope,
    })
    .to_string()
}

/// Emit (MERGE) the Grant projection for a freshly-minted `token` (§4.1).
///
/// Idempotent on the token id, so a re-emit for the same token is a no-op
/// update. Marks the app's prior non-revoked Grant nodes superseded (a fresher
/// mint replaces them; a revoked node stays terminal). The new node is born
/// `live`; the restart sweep and `permission.changed` move it stale later.
pub async fn emit_grant_node(graph: &GraphHandle, token: &CapabilityToken) -> Result<()> {
    let id = token.id.to_string();
    let id_esc = escape_cypher(&id);
    let app_esc = escape_cypher(&token.app_id);
    let issued = time::dt_to_micros(&token.issued_at);
    let expires = token.expires_at.as_ref().map(time::dt_to_micros).unwrap_or(0);
    let ceiling_esc = escape_cypher(&declared_ceiling_json(token));

    // The whole projection runs as ONE transaction, so a mid-emit failure rolls
    // back rather than leaving a partial projection (a Grant with no reach edges,
    // or prior nodes left un-superseded so the app reads as two live grants).
    // Statements within the transaction see each other's writes, so the MATCH
    // after each MERGE resolves the just-created node.
    let mut stmts: Vec<String> = Vec::new();

    // The App principal (plain MERGE: do not clobber a name set by promotion).
    stmts.push(crate::cypher::merge_node("a", "App", &token.app_id));

    // The Grant node, born live. MERGE on the token id so a re-emit updates in
    // place rather than duplicating. The lifecycle flags and the use counters are
    // set ON CREATE only: a re-emit refreshes the declared data (ceiling, pid,
    // issue/expiry) but must NOT resurrect a revoked or superseded grant (revoked
    // is terminal, §3.1) nor zero an accrued use count, so those are never reset
    // on a match.
    stmts.push(format!(
        "MERGE (g:Grant {{id: '{id_esc}'}}) \
         ON CREATE SET g.app_id = '{app_esc}', g.source = 'capability-token', \
         g.pid = {pid}, g.issued_at = {issued}, \
         g.expires_at = {expires}, g.declared_ceiling = '{ceiling_esc}', \
         g.required = false, g.identity_verified = false, g.live = true, \
         g.revoked = false, g.superseded = false, g.last_exercised_at = 0, g.use_count = 0 \
         ON MATCH SET g.pid = {pid}, g.issued_at = {issued}, g.expires_at = {expires}, \
         g.declared_ceiling = '{ceiling_esc}'",
        pid = token.pid,
    ));

    // USED_BY: Grant -> App.
    stmts.push(format!(
        "{} MERGE (g)-[:USED_BY]->(a)",
        crate::cypher::match_two_nodes("g", "Grant", &id, "a", "App", &token.app_id)
    ));

    // The EntityType markers and GRANTS edges for each reachable type.
    for entity_type in reachable_types(token) {
        let t_esc = escape_cypher(&entity_type);
        let label_esc = escape_cypher(type_label(&entity_type));
        stmts.push(format!(
            "MERGE (t:EntityType {{id: '{t_esc}'}}) SET t.label = '{label_esc}'"
        ));
        stmts.push(format!(
            "{} MERGE (g)-[:GRANTS]->(t)",
            crate::cypher::match_two_nodes("g", "Grant", &id, "t", "EntityType", &entity_type)
        ));
    }

    // Supersede the app's prior non-revoked nodes (this fresher mint replaces
    // them); a revoked node stays terminal, and this token's own node is excluded.
    stmts.push(format!(
        "MATCH (g:Grant {{app_id: '{app_esc}'}}) \
         WHERE g.id <> '{id_esc}' AND NOT g.revoked \
         SET g.superseded = true, g.live = false"
    ));

    graph.transaction(stmts).await
}

/// Persist (MERGE) a consent grant into the SHARED LCG Grant node (system-dialog-
/// plan.md, decided Option A): the durable half of the consent grant lifecycle,
/// surfaced by the LCG-R4 `access_grants` read in the same see+revoke place as
/// capability-token grants.
///
/// Keyed by the consent grant's `revocation_handle` (deterministic over its
/// recipient + class + scope), so re-consenting the same scope strengthens the
/// existing node rather than duplicating. Born live, `source = "consent"`,
/// carrying the class + concrete scope; the token-shaped fields (pid / expiry /
/// declared_ceiling) are null/0, `issued_at` is the consent time. A `USED_BY`
/// edge ties it to its App so the read joins it like any grant. Unlike a
/// capability mint there is NO superseding (a user may hold several distinct
/// consent grants for one app at different scopes), and a re-consent RE-ACTIVATES
/// a previously revoked grant (the user explicitly re-allowed it). Atomic +
/// idempotent.
pub async fn persist_consent_grant(
    graph: &GraphHandle,
    recipient: &str,
    consent_class: &str,
    consent_scope: Option<&str>,
    revocation_handle: &str,
) -> Result<()> {
    let app_esc = escape_cypher(recipient);
    let id_esc = escape_cypher(revocation_handle);
    let class_esc = escape_cypher(consent_class);
    let scope_esc = escape_cypher(consent_scope.unwrap_or(""));
    let now = time::now().0;

    let stmts = vec![
        // The App principal (do not clobber a name set by promotion).
        crate::cypher::merge_node("a", "App", recipient),
        // The consent Grant node, born live. ON CREATE seeds every field (token
        // fields null/0); ON MATCH refreshes the consent data AND re-activates
        // (a re-consent overrides a prior revoke, the user's explicit intent).
        format!(
            "MERGE (g:Grant {{id: '{id_esc}'}}) \
             ON CREATE SET g.app_id = '{app_esc}', g.source = 'consent', \
             g.consent_class = '{class_esc}', g.consent_scope = '{scope_esc}', \
             g.pid = 0, g.issued_at = {now}, g.expires_at = 0, g.declared_ceiling = '', \
             g.required = false, g.identity_verified = false, g.live = true, \
             g.revoked = false, g.superseded = false, g.last_exercised_at = 0, g.use_count = 0 \
             ON MATCH SET g.source = 'consent', g.consent_class = '{class_esc}', \
             g.consent_scope = '{scope_esc}', g.live = true, g.revoked = false, g.superseded = false"
        ),
        // USED_BY: Grant -> App.
        format!(
            "{} MERGE (g)-[:USED_BY]->(a)",
            crate::cypher::match_two_nodes("g", "Grant", revocation_handle, "a", "App", recipient)
        ),
    ];
    graph.transaction(stmts).await
}

/// The deterministic id for an app's declared grant in dimension `dim_key`: one
/// grant per (dimension, app), so a re-emit strengthens the single node.
/// Namespaced (`<dim>:<app>`) so it can never collide with a token-derived grant
/// id (a UUID) nor another dimension's grant.
fn declared_grant_id(dim_key: &str, app_id: &str) -> String {
    format!("{dim_key}:{app_id}")
}

/// The deterministic id for an app's declared `NetworkAccess` grant.
fn network_grant_id(app_id: &str) -> String {
    declared_grant_id("network", app_id)
}

/// Project one DECLARED profile dimension as a `declared`-source LCG Grant node
/// (living-capability-graph.md §11b), so the App-access page shows + revokes the
/// app's reach in that dimension. `consent_class` is the family label (e.g.
/// `NetworkAccess`, `FilesystemAccess`), `grant_id` its deterministic per-app id
/// (see [`declared_grant_id`]), `scope` the [`reach_summary`] consent string.
///
/// `source = 'declared'` (the App-access provenance "Declared at install", vs a
/// runtime `consent` grant - decided 1 July). Keyed so a re-emit at the next
/// connect refreshes the scope WITHOUT resurrecting a user-revoked grant: ON MATCH
/// updates only `consent_scope`, never the lifecycle flags
/// (`live`/`revoked`/`superseded`) - the same discipline as [`emit_grant_node`] and
/// the deliberate OPPOSITE of [`persist_consent_grant`] (a runtime re-consent, which
/// DOES re-activate). Enforcement stays external (net-guard, the SDK, the brokers);
/// this only makes the reach visible + revocable. Atomic + idempotent.
pub async fn emit_declared_grant(
    graph: &GraphHandle,
    app_id: &str,
    consent_class: &str,
    grant_id: &str,
    scope: &str,
) -> Result<()> {
    let app_esc = escape_cypher(app_id);
    let id_esc = escape_cypher(grant_id);
    let class_esc = escape_cypher(consent_class);
    let scope_esc = escape_cypher(scope);
    let now = time::now().0;

    let stmts = vec![
        crate::cypher::merge_node("a", "App", app_id),
        // ON CREATE seeds the full grant (token fields null/0, born live); ON MATCH
        // refreshes ONLY the scope, never the lifecycle - so a re-emit after the
        // user revoked leaves the revoke intact (a declared grant is re-granted only
        // by the user re-adding it, not by the app reconnecting).
        format!(
            "MERGE (g:Grant {{id: '{id_esc}'}}) \
             ON CREATE SET g.app_id = '{app_esc}', g.source = 'declared', \
             g.consent_class = '{class_esc}', g.consent_scope = '{scope_esc}', \
             g.pid = 0, g.issued_at = {now}, g.expires_at = 0, g.declared_ceiling = '', \
             g.required = false, g.identity_verified = false, g.live = true, \
             g.revoked = false, g.superseded = false, g.last_exercised_at = 0, g.use_count = 0 \
             ON MATCH SET g.consent_scope = '{scope_esc}'"
        ),
        format!(
            "{} MERGE (g)-[:USED_BY]->(a)",
            crate::cypher::match_two_nodes("g", "Grant", grant_id, "a", "App", app_id)
        ),
    ];
    graph.transaction(stmts).await
}

/// Project an app's DECLARED network reach as a `NetworkAccess` grant. Thin wrapper
/// over [`emit_declared_grant`]; `None` reach is a no-op.
pub async fn emit_declared_network_grant(
    graph: &GraphHandle,
    app_id: &str,
    reach: Option<&str>,
) -> Result<()> {
    let Some(scope) = reach else {
        return Ok(());
    };
    emit_declared_grant(graph, app_id, "NetworkAccess", &network_grant_id(app_id), scope).await
}

/// Project EVERY declared profile dimension as a `declared`-source LCG grant, so the
/// App-access page shows + revokes the app's full reach across the capability
/// families (living-capability-graph.md §11b, generalized from network-first). Graph
/// is projected separately by [`emit_grant_node`] (the token-based capability grant);
/// this covers the profile-declared dimensions (network, event_bus, filesystem,
/// notifications, clipboard, system, input, search, intents, mcp). Each dimension
/// whose [`reach_summary`] is `Some` becomes a grant keyed `<dim>:<app>`, revoke-
/// preserving on re-emit. Returns the first transaction error.
pub async fn emit_all_declared_grants(
    graph: &GraphHandle,
    app_id: &str,
    profile: &arlen_permissions::PermissionProfile,
) -> Result<()> {
    // (reach, consent_class, dimension key). event_bus (an app that hears the bus
    // sees activity) and mcp (an app exposing tools the AI then uses) are real reach.
    // The `consent_class` strings must normalise (lowercase, strip non-alpha) to the
    // App-access page's `DIMENSION_FAMILY` keys (grants.ts): networkaccess /
    // filesystem / clipboard / notifications / system / eventbus / mcp / input /
    // search / intents. Network is the one "…Access" form; the rest are the bare
    // dimension name so they land in the right family instead of falling through.
    let dims: [(Option<String>, &str, &str); 10] = [
        (profile.network.reach_summary(), "NetworkAccess", "network"),
        (profile.event_bus.reach_summary(), "EventBus", "event_bus"),
        (profile.filesystem.reach_summary(), "Filesystem", "filesystem"),
        (
            profile.notifications.reach_summary(),
            "Notifications",
            "notifications",
        ),
        (profile.clipboard.reach_summary(), "Clipboard", "clipboard"),
        (profile.system.reach_summary(), "System", "system"),
        (profile.input.reach_summary(), "Input", "input"),
        (profile.search.reach_summary(), "Search", "search"),
        (profile.intents.reach_summary(), "Intents", "intents"),
        (profile.mcp.reach_summary(), "Mcp", "mcp"),
    ];
    for (reach, consent_class, dim_key) in dims {
        if let Some(scope) = reach {
            emit_declared_grant(graph, app_id, consent_class, &declared_grant_id(dim_key, app_id), &scope)
                .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_label_takes_the_last_segment() {
        assert_eq!(type_label("system.File"), "File");
        assert_eq!(type_label("Project"), "Project");
        assert_eq!(type_label("a.b.c"), "c");
    }

    #[test]
    fn reachable_types_unions_read_and_write_deduped() {
        use crate::token::{EntityScope, InstanceScope};
        let scope = |t: &str| EntityScope {
            entity_type: t.to_string(),
            fields: None,
            exclude_fields: vec![],
        };
        let token = CapabilityToken::new(
            "com.x".into(),
            1,
            vec![scope("system.File"), scope("system.Project")],
            vec![scope("system.File")], // overlaps read
            vec![],
            InstanceScope::Own,
        );
        assert_eq!(
            reachable_types(&token),
            vec!["system.File".to_string(), "system.Project".to_string()]
        );
    }

    #[test]
    fn declared_ceiling_is_canonical_json_of_the_four_collections() {
        use crate::token::{EntityScope, InstanceScope};
        let token = CapabilityToken::new(
            "com.x".into(),
            1,
            vec![EntityScope {
                entity_type: "system.File".into(),
                fields: Some(vec!["path".into()]),
                exclude_fields: vec![],
            }],
            vec![],
            vec![],
            InstanceScope::Own,
        );
        let json = declared_ceiling_json(&token);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["read"][0]["entity_type"], "system.File");
        assert!(parsed.get("write").is_some());
        assert!(parsed.get("relations").is_some());
        assert!(parsed.get("instance").is_some());
    }

    fn token_for(app: &str, types: &[&str]) -> CapabilityToken {
        use crate::token::{EntityScope, InstanceScope};
        let scopes = types
            .iter()
            .map(|t| EntityScope {
                entity_type: t.to_string(),
                fields: None,
                exclude_fields: vec![],
            })
            .collect();
        CapabilityToken::new(app.into(), 7, scopes, vec![], vec![], InstanceScope::Own)
    }

    #[tokio::test]
    async fn emit_creates_a_live_grant_and_a_re_mint_supersedes_the_prior() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();

        let token1 = token_for("com.x", &["system.File", "system.Project"]);
        let id1 = token1.id.to_string();
        emit_grant_node(&graph, &token1).await.unwrap();

        // The fresh grant is live and projects its reach one hop.
        let reach = graph
            .query_rows_json(format!(
                "MATCH (g:Grant {{id:'{id1}'}})-[:GRANTS]->(t:EntityType) WHERE g.live \
                 RETURN t.label ORDER BY t.label"
            ))
            .await
            .unwrap();
        assert!(reach.contains("File") && reach.contains("Project"), "reach projected: {reach}");

        // USED_BY ties it to the App.
        let used = graph
            .query_rows_json(format!(
                "MATCH (g:Grant {{id:'{id1}'}})-[:USED_BY]->(a:App) RETURN a.id"
            ))
            .await
            .unwrap();
        assert!(used.contains("com.x"), "USED_BY edge: {used}");

        // A second mint for the same app supersedes the first (and unsets live).
        let token2 = token_for("com.x", &["system.File"]);
        emit_grant_node(&graph, &token2).await.unwrap();
        let state = graph
            .query_rows_json(format!(
                "MATCH (g:Grant {{id:'{id1}'}}) RETURN g.superseded, g.live"
            ))
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&state).unwrap();
        let row = &parsed["rows"][0];
        assert_eq!(row[0], true, "prior grant superseded: {state}");
        assert_eq!(row[1], false, "prior grant no longer live: {state}");
    }

    #[tokio::test]
    async fn a_re_emit_does_not_resurrect_a_revoked_grant() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();

        let token = token_for("com.x", &["system.File"]);
        let id = token.id.to_string();
        emit_grant_node(&graph, &token).await.unwrap();

        // Simulate a revoke having marked this grant terminal.
        graph
            .write(format!(
                "MATCH (g:Grant {{id:'{id}'}}) SET g.revoked = true, g.live = false"
            ))
            .await
            .unwrap();

        // Re-emitting the SAME token must refresh data but preserve the terminal
        // revoked state (ON MATCH never resets the lifecycle flags), so a stale
        // re-emit can never un-revoke a grant.
        emit_grant_node(&graph, &token).await.unwrap();
        let state = graph
            .query_rows_json(format!("MATCH (g:Grant {{id:'{id}'}}) RETURN g.revoked, g.live"))
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&state).unwrap();
        let row = &parsed["rows"][0];
        assert_eq!(row[0], true, "revoked stays terminal across a re-emit: {state}");
        assert_eq!(row[1], false, "a revoked grant is not made live again: {state}");
    }

    #[tokio::test]
    async fn persist_consent_grant_creates_a_live_consent_grant_then_re_consent_reactivates() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        persist_consent_grant(&graph, "org.arlen.files", "Destructive", Some("/home/x"), "rh-1")
            .await
            .unwrap();
        // The consent grant exists, live, source=consent, with class+scope, joined
        // to its App via USED_BY (so the access_grants read surfaces it).
        let rs = graph
            .query_rows(
                "MATCH (g:Grant {id:'rh-1'})-[:USED_BY]->(:App {id:'org.arlen.files'}) \
                 RETURN g.source, g.consent_class, g.consent_scope, g.live, g.revoked"
                    .into(),
            )
            .await
            .unwrap();
        let r = &rs.rows[0];
        assert_eq!(r[0].as_str(), "consent", "source discriminator");
        assert_eq!(r[1].as_str(), "Destructive", "consent class");
        assert_eq!(r[2].as_str(), "/home/x", "consent scope");
        assert!(r[3].as_bool(), "born live");
        assert!(!r[4].as_bool(), "not revoked");

        // Revoke it, then re-consent the same handle: a re-consent re-activates.
        graph
            .write("MATCH (g:Grant {id:'rh-1'}) SET g.revoked = true, g.live = false".into())
            .await
            .unwrap();
        persist_consent_grant(&graph, "org.arlen.files", "Destructive", Some("/home/x"), "rh-1")
            .await
            .unwrap();
        let rs2 = graph
            .query_rows(
                "MATCH (g:Grant {id:'rh-1'}) RETURN g.live, g.revoked, count(*) AS c".into(),
            )
            .await
            .unwrap();
        assert!(rs2.rows[0][0].as_bool(), "re-consent re-activates live");
        assert!(!rs2.rows[0][1].as_bool(), "re-consent clears revoked");
        assert_eq!(rs2.rows[0][2].as_i64(), 1, "one node, not duplicated");
    }

    #[tokio::test]
    async fn declared_network_grant_projects_and_a_re_emit_never_un_revokes() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();

        // No declared reach -> no grant node (a no-op).
        emit_declared_network_grant(&graph, "com.x", None).await.unwrap();
        let none = graph
            .query_rows("MATCH (g:Grant {id:'network:com.x'}) RETURN count(*) AS c".into())
            .await
            .unwrap();
        assert_eq!(none.rows[0][0].as_i64(), 0, "no grant for no reach");

        // A declared reach projects a live NetworkAccess grant, source 'declared'.
        emit_declared_network_grant(&graph, "com.x", Some("all")).await.unwrap();
        let g = graph
            .query_rows(
                "MATCH (g:Grant {id:'network:com.x'}) \
                 RETURN g.source, g.consent_class, g.consent_scope, g.live, g.revoked"
                    .into(),
            )
            .await
            .unwrap();
        assert_eq!(g.rows[0][0].as_str(), "declared");
        assert_eq!(g.rows[0][1].as_str(), "NetworkAccess");
        assert_eq!(g.rows[0][2].as_str(), "all");
        assert!(g.rows[0][3].as_bool(), "born live");
        assert!(!g.rows[0][4].as_bool(), "not revoked");
        let used = graph
            .query_rows(
                "MATCH (g:Grant {id:'network:com.x'})-[:USED_BY]->(a:App) RETURN a.id".into(),
            )
            .await
            .unwrap();
        assert_eq!(used.rows[0][0].as_str(), "com.x");

        // The user revokes it; then the app reconnects (a re-emit). The re-emit must
        // refresh the scope but NEVER resurrect the revoke - a declared grant is
        // re-granted only by the user re-adding it, not by the app reconnecting.
        graph
            .write("MATCH (g:Grant {id:'network:com.x'}) SET g.revoked = true, g.live = false".into())
            .await
            .unwrap();
        emit_declared_network_grant(&graph, "com.x", Some("api.openai.com")).await.unwrap();
        let after = graph
            .query_rows(
                "MATCH (g:Grant {id:'network:com.x'}) RETURN g.revoked, g.live, g.consent_scope"
                    .into(),
            )
            .await
            .unwrap();
        assert!(after.rows[0][0].as_bool(), "re-emit must NOT un-revoke");
        assert!(!after.rows[0][1].as_bool(), "stays not-live after revoke");
        assert_eq!(after.rows[0][2].as_str(), "api.openai.com", "scope refreshed");
    }

    #[tokio::test]
    async fn emit_all_declared_grants_projects_each_declared_dimension() {
        let tmp = tempfile::tempdir().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();

        // A profile declaring three dimensions (filesystem, clipboard, mcp) and
        // leaving the rest at their empty defaults.
        let profile: arlen_permissions::PermissionProfile = toml::from_str(
            r#"
[info]
app_id = "com.y"
[filesystem]
documents = true
[clipboard]
read = true
write = true
[mcp]
tools_default_permit = ["search"]
"#,
        )
        .unwrap();

        emit_all_declared_grants(&graph, "com.y", &profile).await.unwrap();

        // Each declared dimension lands as a `declared` grant keyed `<dim>:<app>`,
        // with its consent_class + reach_summary scope.
        // consent_class normalises (lowercase, strip non-alpha) to the frontend's
        // DIMENSION_FAMILY keys: filesystem / clipboard / mcp.
        for (id, class, scope) in [
            ("filesystem:com.y", "Filesystem", "documents"),
            ("clipboard:com.y", "Clipboard", "read, write"),
            ("mcp:com.y", "Mcp", "search"),
        ] {
            let rows = graph
                .query_rows(format!(
                    "MATCH (g:Grant {{id:'{id}'}}) RETURN g.source, g.consent_class, g.consent_scope"
                ))
                .await
                .unwrap();
            assert_eq!(rows.rows.len(), 1, "{id} must be projected");
            assert_eq!(rows.rows[0][0].as_str(), "declared");
            assert_eq!(rows.rows[0][1].as_str(), class);
            assert_eq!(rows.rows[0][2].as_str(), scope);
        }

        // An undeclared dimension (notifications default-off) projects no grant.
        let none = graph
            .query_rows("MATCH (g:Grant {id:'notifications:com.y'}) RETURN count(*) AS c".into())
            .await
            .unwrap();
        assert_eq!(none.rows[0][0].as_i64(), 0, "undeclared dimension = no grant");
    }
}
