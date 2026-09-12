//! The enumeration half of the read verb: handles, and nothing else.
//!
//! `0x08` requires an anchoring filter because an unanchored read of a sensitive
//! label is the harvest shape. That reason is about CONTENT, so this op is built
//! to have none (`pi-gate-class-registry.md`, "The enumeration half, measured and
//! ruled"): it answers an id and a display name per row and stops there. "List
//! everything" becomes "list the handles", and anything beyond a handle is asked
//! for by id through the anchored path.
//!
//! **What may be enumerated is a list, not a rule with an exception.** A label is
//! here when the set of its NAMES is not itself the private content. A project is:
//! knowing you have one called "Thesis" is not knowing what is in it. A message, a
//! document or a contact is not, and never becomes one - for those the names ARE
//! the content, and identity-only enumeration is the harvest again with one field.
//! Adding a row to this table is that judgement, made once, in the open.
//!
//! The caller's own read scope still applies on top (RS-R1): being enumerable is
//! permission for the label to be listed at all, not permission for this caller.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::graph::GraphHandle;

/// The labels a caller may enumerate, and the field that names each one.
///
/// Both halves are identifiers this daemon chose, never caller input, so they are
/// interpolated into Cypher directly; the caller supplies no part of the query.
pub const ENUMERABLE: &[(&str, &str)] = &[
    // A project's name is a handle the person chose for their own work. The
    // membership behind it is not readable here, and asking for it is the
    // anchored path's job.
    ("Project", "name"),
];

/// The most handles one call may take. Clamped here as well as at the client, so
/// a caller that skips the client cannot ask for the whole table.
pub const MAX_LIST_LIMIT: i64 = 200;

/// The default when a call names no limit.
pub const DEFAULT_LIST_LIMIT: i64 = 50;

/// What a caller asks for.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListRequest {
    /// The label to enumerate, in its namespaced form (`system.Project`) or bare.
    pub label: String,
    /// How many handles at most.
    #[serde(default)]
    pub limit: Option<i64>,
}

/// One handle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Identity {
    /// The node id, which is what the anchored read takes.
    pub id: String,
    /// What to call it on a screen. Empty when the node has no name set.
    pub name: String,
}

/// The display field for `label`, or `None` when the label is not enumerable.
pub fn display_field(label: &str) -> Option<&'static str> {
    let bare = label.strip_prefix("system.").unwrap_or(label);
    ENUMERABLE.iter().find(|(l, _)| *l == bare).map(|(_, f)| *f)
}

/// The bare label an enumerable request names, or `None`.
pub fn enumerable_label(label: &str) -> Option<&'static str> {
    let bare = label.strip_prefix("system.").unwrap_or(label);
    ENUMERABLE.iter().find(|(l, _)| *l == bare).map(|(l, _)| *l)
}

/// Clamp a requested limit into the bounds this op will serve.
pub fn clamp_limit(requested: Option<i64>) -> i64 {
    requested.unwrap_or(DEFAULT_LIST_LIMIT).clamp(1, MAX_LIST_LIMIT)
}

/// The handles of one label, ordered by name so two calls agree.
///
/// Live rows only: a Project closed by `expired_at` is a record of a period, not
/// something to hand back as a thing that exists.
pub async fn list_identities(
    graph: &GraphHandle,
    label: &'static str,
    field: &'static str,
    limit: i64,
) -> Result<Vec<Identity>> {
    let rows = graph
        .query_rows(format!(
            "MATCH (n:{label}) WHERE n.expired_at IS NULL \
             RETURN n.id AS id, n.{field} AS name ORDER BY name LIMIT {limit}"
        ))
        .await?;
    Ok(rows
        .rows
        .iter()
        .filter_map(|r| {
            let id = r.first()?.as_str().to_string();
            if id.is_empty() {
                return None;
            }
            Some(Identity {
                id,
                name: r.get(1).map(|c| c.as_str().to_string()).unwrap_or_default(),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_is_enumerable_and_a_message_is_not() {
        assert_eq!(enumerable_label("Project"), Some("Project"));
        assert_eq!(enumerable_label("system.Project"), Some("Project"));
        assert_eq!(display_field("system.Project"), Some("name"));
        // The ones the ruling names: for these the names ARE the content.
        for label in ["Message", "Document", "Contact", "File", "Event"] {
            assert_eq!(enumerable_label(label), None, "{label} must not be enumerable");
        }
    }

    #[test]
    fn a_limit_is_clamped_in_both_directions() {
        assert_eq!(clamp_limit(None), DEFAULT_LIST_LIMIT);
        assert_eq!(clamp_limit(Some(5)), 5);
        assert_eq!(clamp_limit(Some(0)), 1);
        assert_eq!(clamp_limit(Some(-9)), 1);
        assert_eq!(clamp_limit(Some(10_000)), MAX_LIST_LIMIT);
    }

    /// A request carrying anything but the two fields does not parse, so a field
    /// nobody designed cannot arrive and be ignored.
    #[test]
    fn a_request_with_an_extra_field_is_refused() {
        assert!(serde_json::from_str::<ListRequest>(r#"{"label":"Project"}"#).is_ok());
        assert!(
            serde_json::from_str::<ListRequest>(r#"{"label":"Project","fields":["secret"]}"#)
                .is_err()
        );
    }

    #[tokio::test]
    async fn it_answers_handles_and_leaves_closed_ones_out() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        graph
            .transaction(vec![
                "CREATE (:Project {id:'p1', name:'Thesis'})".into(),
                "CREATE (:Project {id:'p2', name:'Arlen'})".into(),
                "CREATE (:Project {id:'p3', name:'Gone', expired_at: 5})".into(),
            ])
            .await
            .unwrap();

        let out = list_identities(&graph, "Project", "name", 50).await.unwrap();
        assert_eq!(
            out,
            vec![
                Identity { id: "p2".into(), name: "Arlen".into() },
                Identity { id: "p1".into(), name: "Thesis".into() },
            ],
            "the live handles, by name, and nothing else"
        );
    }

    #[tokio::test]
    async fn the_limit_is_what_comes_back() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        graph
            .transaction(
                (0..10)
                    .map(|i| format!("CREATE (:Project {{id:'p{i}', name:'n{i}'}})"))
                    .collect(),
            )
            .await
            .unwrap();
        let out = list_identities(&graph, "Project", "name", 3).await.unwrap();
        assert_eq!(out.len(), 3);
    }
}
