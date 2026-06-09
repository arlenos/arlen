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

    // The App principal (plain MERGE: do not clobber a name set by promotion).
    graph.write(format!("MERGE (a:App {{id: '{app_esc}'}})")).await?;

    // The Grant node, born live. MERGE on the token id so a re-emit updates in
    // place rather than duplicating. The lifecycle flags and the use counters are
    // set ON CREATE only: a re-emit refreshes the declared data (ceiling, pid,
    // issue/expiry) but must NOT resurrect a revoked or superseded grant (revoked
    // is terminal, §3.1) nor zero an accrued use count, so those are never reset
    // on a match.
    graph
        .write(format!(
            "MERGE (g:Grant {{id: '{id_esc}'}}) \
             ON CREATE SET g.app_id = '{app_esc}', g.pid = {pid}, g.issued_at = {issued}, \
             g.expires_at = {expires}, g.declared_ceiling = '{ceiling_esc}', \
             g.required = false, g.identity_verified = false, g.live = true, \
             g.revoked = false, g.superseded = false, g.last_exercised_at = 0, g.use_count = 0 \
             ON MATCH SET g.pid = {pid}, g.issued_at = {issued}, g.expires_at = {expires}, \
             g.declared_ceiling = '{ceiling_esc}'",
            pid = token.pid,
        ))
        .await?;

    // USED_BY: Grant -> App.
    graph
        .write(format!(
            "MATCH (g:Grant {{id: '{id_esc}'}}), (a:App {{id: '{app_esc}'}}) \
             MERGE (g)-[:USED_BY]->(a)"
        ))
        .await?;

    // The EntityType markers and GRANTS edges for each reachable type.
    for entity_type in reachable_types(token) {
        let t_esc = escape_cypher(&entity_type);
        let label_esc = escape_cypher(type_label(&entity_type));
        graph
            .write(format!(
                "MERGE (t:EntityType {{id: '{t_esc}'}}) SET t.label = '{label_esc}'"
            ))
            .await?;
        graph
            .write(format!(
                "MATCH (g:Grant {{id: '{id_esc}'}}), (t:EntityType {{id: '{t_esc}'}}) \
                 MERGE (g)-[:GRANTS]->(t)"
            ))
            .await?;
    }

    // Supersede the app's prior non-revoked nodes (this fresher mint replaces
    // them); a revoked node stays terminal, and this token's own node is excluded.
    graph
        .write(format!(
            "MATCH (g:Grant {{app_id: '{app_esc}'}}) \
             WHERE g.id <> '{id_esc}' AND NOT g.revoked \
             SET g.superseded = true, g.live = false"
        ))
        .await?;

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
}
