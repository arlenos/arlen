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
    report_grants(&client).await;

    // Always ask. An earlier cut skipped the questions when `access_grants` came
    // back empty, on the reasoning that an unnamed caller can only produce scope
    // denials. That reasoning was wrong twice over: empty grants do not mean the
    // caller is unnamed (the daemon resolved `/usr/bin/arlen-kg-probe` to
    // `kg-probe` perfectly well - what is missing is a Grant NODE in the graph,
    // which is a different thing), and skipping replaced the measurement with an
    // inference. The run then reported "identity NOT RESOLVED" as fact, and the
    // verify verdict repeated it as a decision Tim had to make.
    //
    // A denial is a result. Print it and let the reader see which questions were
    // refused and which were answered.
    let mut failures = 0;
    for round in 1..=ROUNDS {
        if round > 1 {
            tokio::time::sleep(ROUND_GAP).await;
        }
        println!("kg-probe: round {round} of {ROUNDS}");
        failures += ask_all(&client).await;
    }
    failures += report_timeline();
    println!("kg-probe: done, {failures} question(s) failed");
}

/// Report the capability grants the daemon holds for this caller.
///
/// `access_grants` is the one op that answers about the CALLER rather than the
/// graph: the daemon scopes it by the app_id it attested from the peer itself and
/// ignores the request body entirely.
///
/// WHAT IT DOES AND DOES NOT PROVE. A non-empty answer proves the caller was
/// named. An empty one proves nothing on its own, and the first cut of this
/// function got that backwards - it printed "identity: NOT RESOLVED" and skipped
/// every graph question.
///
/// Empty is what a correctly-named caller sees when no Grant NODE has been
/// written for it yet. Those nodes are emitted into the graph at connect time; a
/// probe that connects and immediately asks can beat its own grant into
/// existence. Meanwhile the binary route names `/usr/bin/arlen-kg-probe` as
/// `kg-probe` without difficulty - measured directly against `path_to_app_id`,
/// not assumed.
///
/// So this reports and does not decide. The graph questions run either way, and
/// their answers say more about the caller's scope than this op does.
async fn report_grants(client: &UnixGraphClient) {
    match client.access_grants().await {
        Ok(grants) if grants.is_empty() => {
            println!(
                "kg-probe: grants: none recorded for this caller yet. That is not \
                 proof of an unnamed caller - a Grant node is written at connect \
                 time and this probe asks immediately. The questions below are the \
                 real signal."
            );
        }
        Ok(grants) => {
            let ids: Vec<&str> = grants.iter().map(|g| g.app_id.as_str()).collect();
            println!(
                "kg-probe: grants: {} for caller(s) {:?}",
                grants.len(),
                ids
            );
        }
        Err(e) => {
            println!("kg-probe: grants: FAILED to ask: {e}");
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
