// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The Projects browser read (KA-R3): projects, then their members, as the
//! Miller columns drill down.
//!
//! The columns walk a VIRTUAL slash path rather than a filesystem one - `/` is
//! the set of projects, `/Thesis` is that project's members. Two levels is the
//! whole of it here, because a third column is a member's relationship hops and
//! that is a different read.

use serde::Serialize;
use std::collections::HashMap;

/// One browser row, matching the kit's `FileEntry` so the Miller columns render
/// a project exactly as they render a directory. Snake-case on the wire: the
/// shape is the kit's, not this app's, and renaming it here would fork it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BrowserEntry {
    /// What the column shows.
    pub name: String,
    /// `"directory"` for a project, `"file"` for a member.
    pub kind: String,
    /// Always null: a project has no byte size, and a member's size belongs to
    /// the filesystem read rather than the graph.
    pub size: Option<u64>,
    /// Seconds since the epoch, when the graph recorded one.
    pub modified_unix: Option<i64>,
    pub is_hidden: bool,
    pub readonly: bool,
    pub symlink_target: Option<String>,
    /// The member's real path, so "reveal in containing folder" can work from a
    /// virtual listing. Absent for a project, which has no single home.
    pub full_path: Option<String>,
}

/// The Projects columns: `/` lists projects, `/<project>` lists its members.
///
/// `as_of` is accepted and, when set, **refused** rather than answered with
/// present-day rows. The as-of read is the daemon's bitemporal `valid_as_of`
/// and this command does not perform it yet; returning today's members under a
/// past timestamp would be the one failure the scrubber cannot survive, since
/// the whole point of dragging the control is to trust that the view changed.
/// An error sends the frontend to its fixture, which says it is mocked.
#[tauri::command]
pub async fn knowledge_projects_list(
    path: String,
    as_of: Option<i64>,
) -> Result<Vec<BrowserEntry>, String> {
    if as_of.is_some() {
        return Err("as-of reads are not wired yet".to_string());
    }
    let socket = os_sdk::runtime::socket_path("ARLEN_KNOWLEDGE_SOCKET", "knowledge.sock");
    let client = os_sdk::graph::UnixGraphClient::new(socket.to_string_lossy().into_owned());

    match project_in(&path) {
        None => list_projects(&client).await,
        Some(project) => list_members(&client, project).await,
    }
}

/// The project a browser path names, or `None` for the root listing.
///
/// THE PLACE SEGMENT COMES OFF FIRST, and that is the whole of this function.
/// `ProjectsView` builds its columns with `{ initial: "/projects", root:
/// "/projects" }`, so the root listing arrives here as `/projects` and a project
/// as `/projects/<name>`. This used to be `path.trim_matches('/')` with empty
/// meaning root, which is the convention the doc above describes and NOT the one
/// the only caller speaks: the root arrived as `projects`, took the `else`, and
/// asked for the members of a project by that name. `list_projects` was
/// unreachable from the interface.
///
/// It failed in the way that hides: members are read over `FILE_PART_OF`, which
/// the read gate refuses for this caller, so the call errored, and under vite an
/// error sends the store to its fixture - whose listing for an unknown project is
/// empty. The pane rendered "Empty" against a graph holding 95 projects, and the
/// probe agreed there were 95, because both were answering honestly about
/// different questions (measured 16 August).
///
/// Stripping the leading segment is unambiguous rather than a guess about the
/// word "projects": every path the browser produces carries the root as its first
/// segment, so a project actually NAMED `projects` arrives as
/// `/projects/projects` and still resolves.
fn project_in(path: &str) -> Option<&str> {
    let trimmed = path.trim_matches('/');
    let rest = trimmed.strip_prefix("projects").unwrap_or(trimmed);
    let rest = rest.trim_matches('/');
    (!rest.is_empty()).then_some(rest)
}

/// The browser's place listing (the `knowledge_list` intent in `adapter.ts`):
/// one virtual place per sidebar entry - timeline, projects, searches, library,
/// capsules.
///
/// **Only `projects` answers for real.** The other four are reads this app does
/// not have yet, and each one refuses so the store marks that place mocked and
/// serves its fixture. Refusing per place rather than per command is what lets
/// the Projects place go live while the rest stay honestly labelled: a command
/// that answered them all with empty lists would show four places as "your graph
/// has nothing" when the truth is "nobody asked it".
#[tauri::command]
pub async fn knowledge_list(location: String) -> Result<Vec<BrowserEntry>, String> {
    if location != "projects" {
        return Err(format!("the {location} place is not wired yet"));
    }
    let socket = os_sdk::runtime::socket_path("ARLEN_KNOWLEDGE_SOCKET", "knowledge.sock");
    let client = os_sdk::graph::UnixGraphClient::new(socket.to_string_lossy().into_owned());
    list_projects(&client).await
}

/// Every live project, newest first.
async fn list_projects(
    client: &os_sdk::graph::UnixGraphClient,
) -> Result<Vec<BrowserEntry>, String> {
    let rows = client
        .query_rows(
            "MATCH (p:Project) WHERE p.expired_at IS NULL \
             RETURN p.name AS name, p.root_path AS root_path, p.created_at AS created_at \
             ORDER BY p.created_at DESC LIMIT 500",
        )
        .await
        .map_err(|e| crate::report::graph_call_failed("list_projects", e))?;
    let entries = rows
        .iter()
        .filter_map(|r| {
            let name = text(r, "name")?;
            Some(BrowserEntry {
                name,
                kind: "directory".to_string(),
                size: None,
                modified_unix: seconds(r, "created_at"),
                is_hidden: false,
                readonly: true,
                symlink_target: None,
                // The project's own root, which is what makes two projects that
                // share a basename tellable apart - and what "reveal in
                // containing folder" needs anyway.
                full_path: text(r, "root_path"),
            })
        })
        .collect();
    Ok(disambiguate(entries))
}

/// Give every row a name no other row in the listing carries.
///
/// A PROJECT NAME IS A BASENAME, and basenames repeat. The graph's projects come
/// from detected directories anywhere in the tree, so two of them are called
/// `coffeeshop-repo-template` on this machine (under `source/` and `public/` of the
/// same site repo) and any tree with a `frontend/`, `docs/` or `build/` in two
/// places will do the same. A filesystem listing cannot produce that - names are
/// unique within one directory - so the browser kit keys its rows on
/// `entry.name` (MillerColumns.svelte:132), which is sound for the listing it was
/// built for and not for this one.
///
/// Feeding it duplicates does not render two rows badly, it renders NONE: Svelte
/// aborts a keyed `{#each}` on a duplicate key, so the pane came up "Empty" while
/// 95 projects sat behind it (measured 16 August, found in the webview's own
/// console rather than in any log).
///
/// Disambiguating only where it is needed keeps the common case clean: a unique
/// name stays exactly as detected, and a colliding one gains the directory that
/// contains it, which is the part that actually differs. If even that repeats,
/// the whole root path goes in - long, and still true.
fn disambiguate(entries: Vec<BrowserEntry>) -> Vec<BrowserEntry> {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for e in &entries {
        *seen.entry(e.name.as_str()).or_default() += 1;
    }
    let duplicated: Vec<String> = seen
        .iter()
        .filter(|(_, n)| **n > 1)
        .map(|(k, _)| (*k).to_string())
        .collect();

    let mut out = entries;
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    for e in &mut out {
        if !duplicated.contains(&e.name) {
            used.insert(e.name.clone());
            continue;
        }
        let root = e.full_path.clone().unwrap_or_default();
        let parent = std::path::Path::new(&root)
            .parent()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned());
        let mut candidate = match parent {
            Some(p) if !p.is_empty() => format!("{} ({p})", e.name),
            _ => e.name.clone(),
        };
        if used.contains(&candidate) || candidate == e.name {
            // Still not unique, or there was no parent to name it by: the full
            // root is the last thing that cannot collide, since two projects
            // cannot share one.
            candidate = if root.is_empty() {
                format!("{} ({})", e.name, used.len())
            } else {
                format!("{} ({root})", e.name)
            };
        }
        used.insert(candidate.clone());
        e.name = candidate;
    }
    out
}

/// One project's live members, by the bitemporal FILE_PART_OF edge.
///
/// **Denied today for this caller, and worth knowing before debugging it.** The
/// read gate requires every relationship type in a query to be in the caller's
/// readable set, and that set keeps only entirely-alphanumeric names, so
/// `FILE_PART_OF` cannot be in it. Measured against the gate, this query answers
/// "read denied: label outside the caller's read scope" for any caller that is
/// not system-anchored - which this app is not. The column falls back to its
/// fixture and says it is mocked, which is the honest outcome; making it real
/// needs the scope model to admit declared relations, or this app to be
/// first-party. Recorded for a decision rather than worked around here.
///
/// Liveness comes from the EDGE stamps alone, and that is not a shortcut: a
/// `File` node has no `expired_at` column - only `Project` does - and this
/// engine refuses a labelled match that names a column the table lacks
/// ("Binder exception: Cannot find property expired_at for f"). An earlier cut
/// of this query filtered on it and would have failed every members listing,
/// silently, with the frontend showing its fixture instead. Membership is what
/// the question is about anyway, and the edge carries it.
async fn list_members(
    client: &os_sdk::graph::UnixGraphClient,
    project: &str,
) -> Result<Vec<BrowserEntry>, String> {
    // RESOLVE THE DISPLAY NAME BACK THROUGH THE LISTING THAT PRODUCED IT.
    //
    // The columns compose a child path from `entry.name` (MillerColumns.svelte:78),
    // and `disambiguate` may have made that name `foundation (Repositories)` so the
    // listing could render at all. Matching `p.name` against it would then find
    // nothing, so opening a disambiguated project would answer "no members" - the
    // exact class of quiet wrongness this file keeps producing.
    //
    // Parsing the suffix back off would be a guess (a project may genuinely be
    // called `foo (bar)`). Asking the listing instead is not a guess: the same
    // function decided both strings, so the mapping is right by construction. It
    // costs one extra read per drill-in, on a click.
    //
    // The members then hang off the project's ROOT PATH rather than its name,
    // which is the identity anyway - two projects can share a name, and no two
    // share a root.
    let root = list_projects(client)
        .await?
        .into_iter()
        .find(|e| e.name == project)
        .and_then(|e| e.full_path);
    let cypher = match root {
        Some(root) => format!(
            "MATCH (f:File)-[r:FILE_PART_OF]->(p:Project {{root_path: '{}'}}) \
             WHERE r.invalid_at IS NULL AND r.expired_at IS NULL \
             RETURN f.path AS path, f.last_accessed AS last_accessed \
             ORDER BY f.path LIMIT 2000",
            escape_cypher_literal(&root)
        ),
        // No root recorded for it, or a name the listing does not carry: fall back
        // to the name, which is what this always did.
        None => format!(
            "MATCH (f:File)-[r:FILE_PART_OF]->(p:Project {{name: '{}'}}) \
             WHERE r.invalid_at IS NULL AND r.expired_at IS NULL \
             RETURN f.path AS path, f.last_accessed AS last_accessed \
             ORDER BY f.path LIMIT 2000",
            escape_cypher_literal(project)
        ),
    };
    let rows = client.query_rows(&cypher).await.map_err(|e| crate::report::graph_call_failed("list_members", e))?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            let path = text(r, "path")?;
            let name = path.rsplit('/').next().unwrap_or(&path).to_string();
            Some(BrowserEntry {
                name,
                kind: "file".to_string(),
                size: None,
                modified_unix: seconds(r, "last_accessed"),
                is_hidden: false,
                readonly: true,
                symlink_target: None,
                full_path: Some(path),
            })
        })
        .collect())
}

/// A string cell, or `None` when the column is absent or not a string. A row
/// missing its name is dropped rather than shown as an empty entry.
fn text(row: &HashMap<String, serde_json::Value>, key: &str) -> Option<String> {
    row.get(key)?.as_str().map(str::to_string)
}

/// A timestamp cell as Unix SECONDS. The graph stores microseconds since the
/// epoch and the kit's `modified_unix` is seconds, so this converts rather than
/// passing a number that would render as a date fifty thousand years out.
fn seconds(row: &HashMap<String, serde_json::Value>, key: &str) -> Option<i64> {
    row.get(key)?.as_i64().map(|micros| micros / 1_000_000)
}

/// Escape a string for a single-quoted Cypher literal: backslash first, so an
/// escaped quote is not double-escaped, then the quote. A project name is a
/// user-chosen directory name and can contain either.
pub fn escape_cypher_literal(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str, root: &str) -> BrowserEntry {
        BrowserEntry {
            name: name.to_string(),
            kind: "directory".to_string(),
            size: None,
            modified_unix: None,
            is_hidden: false,
            readonly: true,
            symlink_target: None,
            full_path: Some(root.to_string()),
        }
    }

    #[test]
    fn two_projects_with_one_basename_do_not_collapse_the_whole_listing() {
        // The real pair from this machine. Keyed on `name`, these two rendered
        // zero rows rather than two.
        let out = disambiguate(vec![
            project("coffeeshop-repo-template", "/home/t/site/source/coffeeshop-repo-template"),
            project("coffeeshop-repo-template", "/home/t/site/public/coffeeshop-repo-template"),
            project("arlen", "/home/t/Repositories/arlen"),
        ]);
        let names: Vec<&str> = out.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "coffeeshop-repo-template (source)",
                "coffeeshop-repo-template (public)",
                // A name nobody else carries is left exactly as detected.
                "arlen",
            ]
        );
        let unique: std::collections::HashSet<&str> = names.iter().copied().collect();
        assert_eq!(unique.len(), names.len());
    }

    #[test]
    fn a_collision_the_parent_cannot_settle_falls_back_to_the_root() {
        // Same basename AND same parent name, two different trees.
        let out = disambiguate(vec![
            project("build", "/a/pkg/build"),
            project("build", "/b/pkg/build"),
        ]);
        let names: Vec<&str> = out.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names[0], "build (pkg)");
        assert_eq!(names[1], "build (/b/pkg/build)");
        assert_ne!(names[0], names[1]);
    }

    #[test]
    fn the_browsers_root_lists_projects_rather_than_one_projects_members() {
        // The exact strings `ProjectsView` builds its columns with. The first of
        // these used to resolve to Some("projects"), which asked for the members
        // of a project by that name and left the root listing unreachable.
        assert_eq!(project_in("/projects"), None);
        assert_eq!(project_in("/projects/"), None);
        assert_eq!(project_in("/projects/arlen"), Some("arlen"));
        // The documented convention still works, so the command is honest about
        // both callers rather than swapping one for the other.
        assert_eq!(project_in("/"), None);
        assert_eq!(project_in(""), None);
        assert_eq!(project_in("/arlen"), Some("arlen"));
        // A project actually named `projects` arrives under the root segment and
        // is still reachable - the reason this strips a position, not a word.
        assert_eq!(project_in("/projects/projects"), Some("projects"));
    }

    #[test]
    fn a_project_name_cannot_break_out_of_its_literal() {
        // The name comes from a directory on disk, so a quote in it is ordinary
        // rather than hostile - but it would end the literal all the same.
        assert_eq!(escape_cypher_literal("Tim's thesis"), "Tim\\'s thesis");
        assert_eq!(escape_cypher_literal(r"back\slash"), r"back\\slash");
        // Backslash first: an already-escaped quote must not double-escape.
        assert_eq!(escape_cypher_literal(r"a\'b"), r"a\\\'b");
    }

    #[test]
    fn a_microsecond_stamp_reads_back_as_seconds() {
        let mut row = HashMap::new();
        row.insert("t".to_string(), serde_json::json!(1_700_000_000_000_000i64));
        assert_eq!(seconds(&row, "t"), Some(1_700_000_000));
        // An absent or non-numeric cell is absent, never zero - a zero would
        // render as 1970 and look like a fact.
        assert_eq!(seconds(&row, "missing"), None);
        row.insert("s".to_string(), serde_json::json!("nope"));
        assert_eq!(seconds(&row, "s"), None);
    }

    #[tokio::test]
    async fn an_unwired_place_refuses_so_it_is_marked_mocked_not_empty() {
        // The store flips `mocked` per call, so a refusal is what keeps the four
        // unwired places labelled. Answering them with an empty list would read
        // as "the graph knows nothing about your library".
        for place in ["timeline", "searches", "library", "capsules"] {
            assert!(
                knowledge_list(place.to_string()).await.is_err(),
                "{place} must refuse rather than answer empty"
            );
        }
    }

    #[tokio::test]
    async fn an_as_of_read_is_refused_rather_than_answered_with_today() {
        // The scrubber's whole promise is that the view changed. Answering a
        // past timestamp with present rows is the one lie it cannot survive.
        let r = knowledge_projects_list("/".to_string(), Some(1_700_000_000)).await;
        assert!(r.is_err(), "an as-of read must refuse until it is wired");
    }
}
