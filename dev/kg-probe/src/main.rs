//! What does the knowledge graph actually hold, as a session caller sees it?
//!
//! Written because three rounds of clicking a booted surface could not answer it.
//! The Knowledge app's Timeline and Projects panes render empty on a fresh boot
//! while the daemon's own log reports promoted nodes and a detected project, and
//! from outside the VM there is no way to tell an empty result from a stale read
//! from a refused one - every one of them looks like an empty pane.
//!
//! So this asks the daemon the SAME questions the app asks, over the same socket,
//! and prints what came back. It resolves nothing by itself; it turns a question
//! about a screenshot into a question about rows.
//!
//! Deliberately a VERIFY-image tool and not a release one. It reads the user's
//! graph and prints counts, which is exactly what a probe should do and exactly
//! what has no business being installed on a system nobody is debugging.
//!
//! Prints one line per question, always, including on failure - a probe that goes
//! quiet when the thing it probes is broken is worse than no probe, because the
//! silence reads as absence.

use os_sdk::graph::UnixGraphClient;

/// The app's own queries, by the name of the surface each one feeds.
///
/// Copied deliberately rather than imported: the app builds them inside Tauri
/// commands that need its whole host. If a query here drifts from the app's, the
/// probe answers a question nobody asked - so each carries the file it mirrors.
const QUESTIONS: &[(&str, &str)] = &[
    // apps/knowledge/src-tauri/src/timeline.rs, read_file_accesses
    (
        "timeline: file accesses",
        "MATCH (f:File) WHERE f.last_accessed IS NOT NULL RETURN f.id LIMIT 500",
    ),
    // apps/knowledge/src-tauri/src/timeline.rs, read_window_focus
    (
        "timeline: window focus",
        "MATCH (e:Event) WHERE e.type = 'window.focused' RETURN e.id LIMIT 500",
    ),
    // apps/knowledge/src-tauri/src/projects.rs, list_projects
    (
        "projects: live",
        "MATCH (p:Project) WHERE p.expired_at IS NULL RETURN p.name LIMIT 500",
    ),
    // The same nodes without the liveness filter, which separates "nothing was
    // recorded" from "everything recorded is closed".
    ("projects: any", "MATCH (p:Project) RETURN p.name LIMIT 500"),
    ("files: any", "MATCH (f:File) RETURN f.id LIMIT 500"),
    ("events: any", "MATCH (e:Event) RETURN e.id LIMIT 500"),
];

/// How many times to ask, and how long to wait between rounds.
///
/// One round is not enough, and the reason is a mistake I made with this tool
/// rather than a theory. The unit starts right after the graph daemon, so a single
/// round measures the graph about five seconds into the boot - before the first
/// promotion pass, which runs on a 30 second interval, and before most of the
/// session has emitted anything. Every "0 row(s)" it printed was true and told me
/// almost nothing, and I read those zeroes for an hour as evidence about the
/// steady state.
///
/// Two rounds: one at boot, one after the promotion interval has come round twice.
/// The difference between them is the interesting number - it separates "nothing
/// is ever recorded" from "nothing had been recorded yet".
const ROUNDS: usize = 2;
const ROUND_GAP: std::time::Duration = std::time::Duration::from_secs(75);

#[tokio::main]
async fn main() {
    let socket = os_sdk::runtime::socket_path("ARLEN_KNOWLEDGE_SOCKET", "knowledge.sock");
    println!("kg-probe: socket {}", socket.display());
    let client = UnixGraphClient::new(socket.to_string_lossy().into_owned());

    let mut failures = 0;
    for round in 1..=ROUNDS {
        if round > 1 {
            tokio::time::sleep(ROUND_GAP).await;
        }
        println!("kg-probe: round {round} of {ROUNDS}");
        failures += ask_all(&client).await;
    }
    println!("kg-probe: done, {failures} question(s) failed");
}

/// Ask every question once; answer how many failed.
async fn ask_all(client: &UnixGraphClient) -> usize {
    let mut failures = 0;
    for (name, cypher) in QUESTIONS {
        match client.query_rows(cypher).await {
            // The count is the answer; the rows themselves are the user's content
            // and this prints none of them.
            Ok(rows) => println!("kg-probe: {name}: {} row(s)", rows.len()),
            Err(e) => {
                failures += 1;
                // The error verbatim, because "denied" and "no such file" and
                // "invalid query" are three different findings that an empty pane
                // renders identically.
                println!("kg-probe: {name}: FAILED: {e}");
            }
        }
    }
    failures
}
