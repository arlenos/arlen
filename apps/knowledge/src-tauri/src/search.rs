// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Search (KA-R4): structured and name search over the graph, with the guided
//! facets.
//!
//! Text ranking rides the daemon's retrieval op rather than a `CONTAINS` scan:
//! the daemon already synthesises a fact text per node, indexes it in FTS5 and
//! fuses BM25 with a graph expansion, so asking it is both better and the read
//! that is already scoped to this caller. This command resolves the ids it
//! ranks, keeping that order, and applies the facets on top.
//!
//! Two of the model's six result types are answered: `file` and `project`. The
//! other four - paper, mail, note, session - name library and bridge entities
//! that are not in the graph as their own nodes, so asking for one returns
//! nothing rather than a File dressed up as a paper.

use crate::projects::escape_cypher_literal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The guided facets, as the store sends them. `null` means "any".
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchFacets {
    /// One of the model's result types.
    pub r#type: Option<String>,
    pub project: Option<String>,
    /// Days back from now.
    pub within_days: Option<i64>,
}

/// One hit, in the shape the results list renders.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SearchResult {
    pub id: String,
    pub r#type: String,
    pub title: String,
    /// The quiet context: the app or bridge it came from.
    pub sub: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

/// How many ids to ask the ranker for, and how many hits to return. The ranker
/// is asked for more than is shown because the facets filter afterwards.
const RANK_LIMIT: i64 = 200;
const RESULT_LIMIT: usize = 100;

/// The types this command can actually answer from the graph today.
const ANSWERABLE: [&str; 2] = ["file", "project"];

/// Search the graph. An empty query with facets set is a browse; a query with
/// no facets is a ranked text search; both together intersect.
#[tauri::command]
pub async fn knowledge_search(
    query: String,
    facets: SearchFacets,
) -> Result<Vec<SearchResult>, String> {
    // A facet naming a type the graph has no nodes for answers empty rather
    // than falling through to files: a "papers" filter that returns files is a
    // worse answer than none.
    if let Some(t) = facets.r#type.as_deref() {
        if !ANSWERABLE.contains(&t) {
            return Ok(Vec::new());
        }
    }
    let socket = os_sdk::runtime::socket_path("ARLEN_KNOWLEDGE_SOCKET", "knowledge.sock");
    let client = os_sdk::graph::UnixGraphClient::new(socket.to_string_lossy().into_owned());

    let trimmed = query.trim();
    let ranked = if trimmed.is_empty() {
        None
    } else {
        Some(client.retrieve(trimmed, RANK_LIMIT).await.map_err(|e| e.to_string())?)
    };

    let mut out = Vec::new();
    if facets.r#type.as_deref() != Some("project") {
        out.extend(files(&client, ranked.as_deref(), &facets).await?);
    }
    if facets.r#type.as_deref() != Some("file") {
        out.extend(projects(&client, ranked.as_deref(), &facets).await?);
    }
    if let Some(order) = ranked.as_deref() {
        // Keep the ranker's order; anything it did not rank sorts last.
        out.sort_by_key(|r| order.iter().position(|id| id == &r.id).unwrap_or(usize::MAX));
    } else {
        out.sort_by_key(|r| std::cmp::Reverse(r.at.unwrap_or(0)));
    }
    out.truncate(RESULT_LIMIT);
    Ok(out)
}

/// Matching files, with their live project.
async fn files(
    client: &os_sdk::graph::UnixGraphClient,
    ranked: Option<&[String]>,
    facets: &SearchFacets,
) -> Result<Vec<SearchResult>, String> {
    let mut wheres = vec!["f.last_accessed IS NOT NULL".to_string()];
    if let Some(ids) = ranked {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        wheres.push(format!("f.id IN [{}]", id_list(ids)));
    }
    if let Some(cut) = cutoff_micros(facets.within_days) {
        wheres.push(format!("f.last_accessed >= {cut}"));
    }
    let project_clause = match facets.project.as_deref() {
        // A project facet is a REQUIRED membership, so the match stops being
        // optional - an optional match with a filter on the project would keep
        // every unmatched file with a null project instead of dropping it.
        Some(name) => format!(
            "MATCH (f)-[r:FILE_PART_OF]->(p:Project {{name: '{}'}}) \
             WHERE r.invalid_at IS NULL AND r.expired_at IS NULL ",
            escape_cypher_literal(name)
        ),
        None => "OPTIONAL MATCH (f)-[r:FILE_PART_OF]->(p:Project) \
                 WHERE r.invalid_at IS NULL AND r.expired_at IS NULL "
            .to_string(),
    };
    let cypher = format!(
        "MATCH (f:File) WHERE {} {}\
         RETURN f.id AS id, f.path AS path, f.app_id AS app_id, \
                f.last_accessed AS at, p.name AS project \
         ORDER BY f.last_accessed DESC LIMIT {RANK_LIMIT}",
        wheres.join(" AND "),
        project_clause
    );
    let rows = client.query_rows(&cypher).await.map_err(|e| e.to_string())?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            let path = text(r, "path")?;
            Some(SearchResult {
                id: text(r, "id").unwrap_or_else(|| path.clone()),
                r#type: "file".to_string(),
                title: path.rsplit('/').next().unwrap_or(&path).to_string(),
                sub: text(r, "app_id").unwrap_or_default(),
                at: seconds(r, "at"),
                project: text(r, "project"),
            })
        })
        .collect())
}

/// Matching projects. A project facet naming one project makes the project
/// results that project alone, which is what picking it in the facet bar means.
async fn projects(
    client: &os_sdk::graph::UnixGraphClient,
    ranked: Option<&[String]>,
    facets: &SearchFacets,
) -> Result<Vec<SearchResult>, String> {
    let mut wheres = vec!["p.expired_at IS NULL".to_string()];
    if let Some(ids) = ranked {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        wheres.push(format!("p.id IN [{}]", id_list(ids)));
    }
    if let Some(name) = facets.project.as_deref() {
        wheres.push(format!("p.name = '{}'", escape_cypher_literal(name)));
    }
    if let Some(cut) = cutoff_micros(facets.within_days) {
        wheres.push(format!("p.created_at >= {cut}"));
    }
    let cypher = format!(
        "MATCH (p:Project) WHERE {} \
         RETURN p.id AS id, p.name AS name, p.root_path AS root_path, \
                p.created_at AS at \
         ORDER BY p.created_at DESC LIMIT {RANK_LIMIT}",
        wheres.join(" AND ")
    );
    let rows = client.query_rows(&cypher).await.map_err(|e| e.to_string())?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            let name = text(r, "name")?;
            Some(SearchResult {
                id: text(r, "id").unwrap_or_else(|| name.clone()),
                r#type: "project".to_string(),
                title: name,
                sub: text(r, "root_path").unwrap_or_default(),
                at: seconds(r, "at"),
                project: None,
            })
        })
        .collect())
}

/// A quoted, escaped Cypher list of ids.
fn id_list(ids: &[String]) -> String {
    ids.iter()
        .map(|id| format!("'{}'", escape_cypher_literal(id)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The epoch-microsecond floor for "within N days", or `None` for any time.
///
/// A non-positive window is treated as no window rather than as a floor in the
/// future: a zero would otherwise return nothing and read as "you did nothing
/// today".
fn cutoff_micros(within_days: Option<i64>) -> Option<i64> {
    let days = within_days?;
    if days <= 0 {
        return None;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_micros() as i64;
    Some(now - days * 86_400 * 1_000_000)
}

fn text(row: &HashMap<String, serde_json::Value>, key: &str) -> Option<String> {
    row.get(key)?.as_str().map(str::to_string)
}

fn seconds(row: &HashMap<String, serde_json::Value>, key: &str) -> Option<i64> {
    row.get(key)?.as_i64().map(|micros| micros / 1_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_type_the_graph_has_no_nodes_for_answers_nothing_not_files() {
        // Picking "papers" and getting files back is a worse answer than an
        // empty list: it looks like the filter works and quietly does not.
        for t in ["paper", "mail", "note", "session"] {
            let facets = SearchFacets {
                r#type: Some(t.to_string()),
                ..Default::default()
            };
            let r = knowledge_search("anything".into(), facets).await;
            assert_eq!(r.unwrap(), Vec::new(), "{t} must answer empty");
        }
    }

    #[test]
    fn a_zero_or_negative_window_is_no_window_rather_than_an_empty_result() {
        assert!(cutoff_micros(None).is_none());
        assert!(cutoff_micros(Some(0)).is_none(), "zero days is not a floor in the future");
        assert!(cutoff_micros(Some(-3)).is_none());
        let seven = cutoff_micros(Some(7)).expect("a real window");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros() as i64;
        let week = 7 * 86_400 * 1_000_000;
        assert!((now - seven - week).abs() < 5_000_000, "seven days back, within a few seconds");
    }

    #[test]
    fn an_id_list_escapes_every_entry() {
        // Ids are paths, and a path can hold a quote.
        let list = id_list(&["/x/o'brien".to_string(), "/y".to_string()]);
        assert_eq!(list, "'/x/o\\'brien', '/y'");
    }
}
