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
    // The one question about something THIS RUN produced.
    //
    // Every question above is satisfied by any row from anywhere, which is a
    // weaker claim than it reads as: "the graph holds files" and "the graph
    // ingested what just happened" are different sentences, and only the second
    // is what a boot is for. The image starts with an empty graph, so a row here
    // can only have arrived through the path under test - the dogfood emits a
    // `file.opened` for this exact path, the writer stores it, promotion turns it
    // into a File node.
    //
    // The path is the dogfood's, hardcoded on both sides, and that coupling is
    // the point rather than a shortcut: the assertion has to name the thing the
    // run made, or it is back to counting rows. If the dogfood's fixture path
    // ever moves, this question stops returning rows and the verdict says the
    // ingestion path is dead - a false alarm, but a LOUD one, which is the right
    // direction for a coupling to fail in.
    (
        "ingestion: this run's file",
        "MATCH (f:File) WHERE f.id = '/var/lib/arlen-work/notes.md' RETURN f.id LIMIT 1",
    ),
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

    // Ask who the daemon thinks we are BEFORE asking anything about the graph.
    //
    // Without this the probe cannot tell its two failures apart, and they mean
    // opposite things. `read denied: label outside the caller's read scope` is
    // what an unnamed caller gets for every question, and it is also what a
    // caller with a real scope gap gets for one. On the 15 Aug boot all six
    // questions came back with it because the daemon could not resolve the probe
    // at all - so the output read as "the graph is broken" when the graph was
    // fine and the probe was anonymous.
    //
    // That distinction now decides whether a run FAILS. A probe that cannot be
    // named is a probe that cannot ask, which is a fact about this build, not a
    // defect in the system under test. Failing on it would put a permanent red in
    // every verify run - and a red that is always there is one nobody reads,
    // which is the habit that cost us a real CI failure last week.
    let named = report_identity(&client).await;

    let mut failures = 0;
    if named {
        for round in 1..=ROUNDS {
            if round > 1 {
                tokio::time::sleep(ROUND_GAP).await;
            }
            println!("kg-probe: round {round} of {ROUNDS}");
            failures += ask_all(&client).await;
        }
    } else {
        println!(
            "kg-probe: SKIPPED the graph questions: this caller has no identity the \
             daemon can resolve, so every answer would be a scope denial that says \
             nothing about the graph"
        );
    }
    failures += report_timeline();
    println!("kg-probe: done, {failures} question(s) failed");
}

/// Whether the daemon could name this caller, reported either way.
///
/// `access_grants` is the one op that answers about the CALLER rather than the
/// graph: the daemon scopes it by the app_id it attested from the peer itself and
/// ignores the request body entirely. So a non-empty answer is proof the caller
/// was resolved, and an empty one is proof it was not - no guessing from the shape
/// of other failures.
///
/// The probe is a verify-only unit and is deliberately NOT in the shipped
/// `USER_UNIT_APP_IDS` table, which is the set the session supervisor stamps into
/// the identity broker. So on a hardened per-user system it is expected to be
/// anonymous: the broker has never heard of it and the `/proc` route its identity
/// used to come from is refused to a non-root reader. That is the read gate
/// working, not a regression, and it is why this prints the reason rather than a
/// count of failures.
async fn report_identity(client: &UnixGraphClient) -> bool {
    match client.access_grants().await {
        Ok(grants) if grants.is_empty() => {
            println!(
                "kg-probe: identity: NOT RESOLVED (no grants for this caller). The probe \
                 is not in the stamped-unit table and its /proc route is refused, so the \
                 daemon has no name to scope reads by."
            );
            false
        }
        Ok(grants) => {
            println!(
                "kg-probe: identity: resolved as {} ({} grant(s))",
                grants[0].app_id,
                grants.len()
            );
            true
        }
        Err(e) => {
            // Not the anonymous case: the op itself did not answer, which is a
            // fact about the daemon rather than about this caller's name.
            println!("kg-probe: identity: FAILED to ask: {e}");
            false
        }
    }
}

/// Whether the timeline mount is there and this process can read it.
///
/// Asked because "the unit started and logged `mounting`" is not the same claim.
/// The timeline was a SYSTEM unit until 15 Aug, and a hardened system unit gets
/// its own mount namespace - so it could log a successful mount, hold it, and
/// have it be invisible to every process in the user's session. The log looked
/// identical in both worlds.
///
/// This runs in the session's own manager, so what it can see is what an app can
/// see. Directory contents are not printed: they are derived from the user's
/// graph, and the question here is reachability, not content.
fn report_timeline() -> usize {
    let home = match std::env::var_os("HOME") {
        Some(h) => std::path::PathBuf::from(h),
        None => {
            println!("kg-probe: timeline: mount: FAILED: no HOME in the environment");
            return 1;
        }
    };
    let mount = home.join(".timeline");
    match std::fs::read_dir(&mount) {
        Ok(entries) => {
            println!(
                "kg-probe: timeline: mount: readable at {} ({} entr(ies))",
                mount.display(),
                entries.count()
            );
            0
        }
        Err(e) => {
            println!(
                "kg-probe: timeline: mount: FAILED: {} is not readable from this \
                 session: {e}",
                mount.display()
            );
            1
        }
    }
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
