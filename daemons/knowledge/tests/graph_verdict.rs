// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Controls for `arlen-graph-verdict`.
//!
//! The case that earns this check is the second one: a File node present and no
//! FILE_PART_OF edge. That is precisely the boot the current assertion cannot
//! tell from a good one, because the dogfood prints `DOGFOOD WRITE ok` on the
//! AGENT's word that it wrote and `ai_verdict` greps for that line. Here the
//! store is asked instead, and it disagrees.
//!
//! Reading nothing must not read as a clean answer either, so a store that will
//! not open exits 2 rather than 1 - a different fact, kept a different fact.

use knowledge::graph;
use std::process::Command;

const VERDICT: &str = env!("CARGO_BIN_EXE_arlen-graph-verdict");
const FILE: &str = "/w/dogfood.rs";

/// (exit code, combined output) of the verdict run against `store`.
fn verdict(store: &str) -> (i32, String) {
    let out = Command::new(VERDICT)
        .args([store, "--file", FILE])
        .output()
        .expect("the verdict binary runs");
    (
        out.status.code().unwrap_or(-1),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

/// A store with the schema, plus whatever `writes` puts in it.
async fn store_with(dir: &std::path::Path, writes: &[String]) -> String {
    let path = dir.join("g").to_str().unwrap().to_string();
    let g = graph::spawn(&path).expect("a store opens");
    for w in writes {
        g.write(w.clone()).await.expect("the fixture write lands");
    }
    g.shutdown().await;
    path
}

#[tokio::test]
async fn the_store_confirms_a_file_that_was_promoted_and_linked() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = store_with(
        tmp.path(),
        &[
            format!("CREATE (:File {{id: '{FILE}', path: '{FILE}'}})"),
            "CREATE (:Project {id: 'p1', name: 'p1'})".to_string(),
            format!(
                "MATCH (f:File {{id: '{FILE}'}}), (p:Project {{id: 'p1'}}) \
                 CREATE (f)-[:FILE_PART_OF {{op_id: 'op-1'}}]->(p)"
            ),
        ],
    )
    .await;

    let (code, out) = verdict(&path);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("GRAPH OK"), "{out}");
}

/// The sensor question is the one that says the machine-wide eBPF layer reaches
/// the graph and not merely the event store. It is asked in Cypher, so this test
/// is also what settles that the predicate RUNS: `ask` folds any query error into
/// "the table is not in this store", so a mistyped function would report a missing
/// table on a store that has one, and the line would read as an honest no.
#[tokio::test]
async fn the_store_reports_an_app_node_minted_by_the_kernel_sensor() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_with(
        dir.path(),
        &[
            "CREATE (:App {id: 'ebpf:cgroup:4242', name: 'ebpf:cgroup:4242'})".to_string(),
        ],
    )
    .await;
    let (_, out) = verdict(&store);
    assert!(
        out.contains("GRAPH sensor: yes"),
        "an App node minted by the sensor is seen: {out}"
    );
    assert!(
        !out.contains("not measured"),
        "the predicate ran rather than erroring into the not-measured branch: {out}"
    );
}

/// The other half, and the one that matters more: a store carrying only desktop
/// nodes must say no. Without this the line above could be printed by a predicate
/// that matches everything.
#[tokio::test]
async fn a_store_with_only_desktop_apps_reports_no_sensor() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_with(
        dir.path(),
        &["CREATE (:App {id: 'org.arlen.Files', name: 'Files'})".to_string()],
    )
    .await;
    let (_, out) = verdict(&store);
    assert!(
        out.contains("GRAPH sensor: no"),
        "a desktop-only store does not pass for a sensor-carrying one: {out}"
    );
}

#[tokio::test]
async fn a_promoted_file_with_no_edge_is_refused() {
    // The self-report case: the agent would have said it wrote. The store says
    // otherwise, and the store is the one that has to be right.
    let tmp = tempfile::TempDir::new().unwrap();
    let path = store_with(
        tmp.path(),
        &[format!("CREATE (:File {{id: '{FILE}', path: '{FILE}'}})")],
    )
    .await;

    let (code, out) = verdict(&path);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("GRAPH promoted: yes"), "{out}");
    assert!(out.contains("GRAPH linked: no"), "{out}");
    assert!(out.contains("the graph does not have"), "{out}");
}

#[tokio::test]
async fn a_closed_edge_does_not_count_as_linked() {
    // Retraction closes an edge rather than deleting it (close-never-delete), so
    // a verdict that ignored the stamps would call a retracted link a live one.
    let tmp = tempfile::TempDir::new().unwrap();
    let path = store_with(
        tmp.path(),
        &[
            format!("CREATE (:File {{id: '{FILE}', path: '{FILE}'}})"),
            "CREATE (:Project {id: 'p1', name: 'p1'})".to_string(),
            format!(
                "MATCH (f:File {{id: '{FILE}'}}), (p:Project {{id: 'p1'}}) \
                 CREATE (f)-[:FILE_PART_OF {{op_id: 'op-1', invalid_at: 5, expired_at: 5}}]->(p)"
            ),
        ],
    )
    .await;

    let (code, out) = verdict(&path);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("GRAPH linked: no"), "{out}");
}

#[tokio::test]
async fn an_empty_store_is_refused_rather_than_passed() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = store_with(tmp.path(), &[]).await;

    let (code, out) = verdict(&path);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("promotion never reached the graph"), "{out}");
}

#[test]
fn a_store_that_will_not_open_is_not_a_clean_no() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("not-a-store");
    std::fs::write(&path, b"this is not a graph").unwrap();

    let (code, out) = verdict(path.to_str().unwrap());
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("GRAPH UNREADABLE"), "{out}");
}

#[test]
fn a_missing_store_is_reported_as_missing() {
    let (code, out) = verdict("/nonexistent/arlen/graph");
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("no store at"), "{out}");
}
