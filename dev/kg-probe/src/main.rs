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
    //
    // VERBATIM, including the aliases and the ordering. It used to be the same
    // MATCH with `RETURN p.name` and nothing else, and on 16 August that cost an
    // hour: the probe said 95 rows while the app's Projects pane rendered "Empty".
    // Both were telling the truth about different queries. A probe whose whole
    // claim is that it asks what the app asks has to carry the app's query
    // character for character, because the part that differed is the part that
    // broke.
    (
        "projects: live",
        "MATCH (p:Project) WHERE p.expired_at IS NULL \
         RETURN p.name AS name, p.created_at AS created_at \
         ORDER BY p.created_at DESC LIMIT 500",
    ),
    // The same nodes without the liveness filter, which separates "nothing was
    // recorded" from "everything recorded is closed".
    ("projects: any", "MATCH (p:Project) RETURN p.name LIMIT 500"),
    ("files: any", "MATCH (f:File) RETURN f.id LIMIT 500"),
    // apps/knowledge/src-tauri/src/projects.rs, list_members - the TRAVERSAL.
    //
    // Five surfaces across three apps carry a comment saying a query naming a
    // relationship type is refused for a caller that is not system-anchored, and
    // each falls back to a fixture rather than asking. The gate says otherwise
    // (daemon.rs:4394): a traversal is authorised by its ENDPOINTS, and only types
    // on `RESTRICTED_RELATIONS` need their own grant - a list that is currently
    // empty. This asks, so the answer is measured rather than assumed.
    (
        "traversal: project members",
        "MATCH (f:File)-[r:FILE_PART_OF]->(p:Project) \
         WHERE r.invalid_at IS NULL AND r.expired_at IS NULL \
         RETURN f.path AS path LIMIT 50",
    ),
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
    // The hardened control unit runs this same binary and must not ask the graph
    // anything - it exists to print one comparable sweep line, nothing else.
    if std::env::var_os("ARLEN_KG_PROBE_SWEEP_ONLY").is_some() {
        report_proc_sweep();
        println!("kg-probe: sweep-only run, asked the graph nothing");
        return;
    }

    // One ad-hoc read, as this caller, printed verbatim.
    //
    // The fixed questions below answer "what does the graph hold". They cannot
    // answer "why did THIS query fail", which is the question an app surface
    // raises every time it falls back to its fixture - and falling back is
    // silent, so the error never reaches anybody. Twice now the only way to see
    // it was to add a print to an app and rebuild it.
    //
    // Deliberately NOT part of a normal run: it takes the query on argv, prints
    // the rows or the refusal, and exits without the rounds.
    if let Some(cypher) = std::env::args().nth(1) {
        println!("kg-probe: asking one query as this caller");
        match client.query_rows(&cypher).await {
            Ok(rows) => {
                println!("kg-probe: {} row(s)", rows.len());
                for row in rows.iter().take(20) {
                    println!("  {}", serde_json::to_string(row).unwrap_or_default());
                }
            }
            Err(e) => {
                println!("kg-probe: REFUSED: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    report_profile();
    report_proc_sweep();
    wait_until_named(&client).await;
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
        // After the last round's snapshot, wait for the one answer that arrives
        // over time rather than declaring it absent at a fixed offset.
        //
        // The result is NOT counted as a probe failure, deliberately. This tool
        // reports and `probe_verdict` grades, and the verdict has a specific
        // sentence for an empty ingestion - "the graph answered with rows, but
        // nothing this run produced reached it" - which a raised failure count
        // short-circuits into the generic "reported failures". Counting it here
        // cost the reader the better message; the printed line is the report.
        if round == ROUNDS {
            await_ingestion(&client).await;
        }
    }
    failures += report_timeline();
    failures += report_event_store();
    println!("kg-probe: done, {failures} question(s) failed");
}

/// The path the dogfood emits and the graph question asks about.
///
/// Named once so the store read below and the Cypher above cannot drift apart;
/// the test at the bottom fails if the question stops mentioning it.
const INGESTED_PATH: &str = "/var/lib/arlen-work/notes.md";

/// Where the writer's event store lives, resolved the way the daemon resolves it.
///
/// A SECOND COPY OF A RESOLVER, which is how `ARLEN_KNOWLEDGE_SOCKET` and
/// `ARLEN_DAEMON_SOCKET` came to name one socket by two rules. Source of truth is
/// `daemons/knowledge/src/utils.rs::resolve_data_path`; this prints what it
/// resolved so a divergence shows up as a line in the journal rather than as a
/// silent pass on an empty file that was never the daemon's.
fn events_db_path() -> String {
    let pinned = std::env::var("ARLEN_DB_PATH").ok().filter(|s| !s.is_empty());
    if let Some(p) = pinned {
        return p;
    }
    if let Some(dir) = std::env::var("XDG_DATA_HOME").ok().filter(|s| !s.is_empty()) {
        return format!("{dir}/arlen/events.db");
    }
    if let Some(h) = std::env::var("HOME").ok().filter(|s| !s.is_empty()) {
        return format!("{h}/.local/share/arlen/events.db");
    }
    "/var/lib/arlen/knowledge/events.db".to_string()
}

/// How many stored events name `needle`, read from the file rather than asked of
/// the daemon.
///
/// The payload is protobuf, so the path sits in it as bytes; `instr` over a blob
/// is enough to answer "did this event reach the store", which is the only
/// question here. Read-only, and a missing or unreadable file is an error rather
/// than a zero - "no store" and "a store holding nothing" are the two answers
/// this exists to tell apart.
fn store_rows_for(db: &str, needle: &str) -> Result<i64, String> {
    use rusqlite::OpenFlags;
    let conn = rusqlite::Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("{e}"))?;
    conn.query_row(
        "SELECT COUNT(*) FROM events WHERE instr(payload, ?1) > 0",
        rusqlite::params![needle.as_bytes()],
        |r| r.get::<_, i64>(0),
    )
    .map_err(|e| format!("{e}"))
}

/// Ask the store the question the graph was asked, and print both answers.
///
/// Every other line this probe prints comes from the daemon: it answers about
/// itself, and nothing checks it. A graph returning a File node for a path whose
/// event never reached the store is exactly the shape of a broken writer, and the
/// verdict could not tell that from a healthy run. Two independent reads can.
fn report_event_store() -> usize {
    let db = events_db_path();
    println!("kg-probe: store path {db}");
    match store_rows_for(&db, INGESTED_PATH) {
        Ok(n) => {
            println!("kg-probe: store: {n} event row(s) naming this run's file");
            0
        }
        Err(e) => {
            println!("kg-probe: store: UNREADABLE: {e}");
            1
        }
    }
}

/// Can ANY same-uid process on this image read another's `/proc/<pid>/exe`?
///
/// The daemon refuses to name this caller because `readlinkat` on its exe link
/// returns EACCES, and the refusal survived every explanation: uid, gid,
/// capabilities, Yama, LSM label, seccomp and pid namespace are identical on both
/// sides, and the reader can read its own link. `cwd` and `root` are refused too
/// while `cmdline` is allowed, so it is the ptrace permission check itself.
///
/// That leaves one question worth asking from here, and it is a measurement
/// rather than a seventh hypothesis: is this pair special, or does the check
/// refuse every cross-process read on this image? A sweep answers it. If nothing
/// but self reads, the `/proc` identity route does not work here at all and the
/// stamped table is the only way any caller gets a name. If some pairs read, the
/// ones that do are the comparison that explains the ones that do not.
fn report_proc_sweep() {
    // SAFETY: getuid never fails.
    let me = unsafe { libc::getuid() };
    let mut mine = 0usize;
    let mut readable = 0usize;
    let mut refused = 0usize;
    let mut examples: Vec<String> = Vec::new();
    let mut refused_names: Vec<String> = Vec::new();

    let Ok(entries) = std::fs::read_dir("/proc") else {
        println!("kg-probe: proc sweep: /proc unreadable");
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        // Same uid only: a cross-uid refusal is expected and would drown the
        // signal we are after.
        let Ok(md) = std::fs::metadata(format!("/proc/{name}")) else { continue };
        use std::os::unix::fs::MetadataExt;
        if md.uid() != me {
            continue;
        }
        mine += 1;
        match std::fs::read_link(format!("/proc/{name}/exe")) {
            Ok(p) => {
                readable += 1;
                if examples.len() < 4 {
                    examples.push(format!("{name}={}", p.display()));
                }
            }
            Err(e) => {
                refused += 1;
                // Name them. The counts alone said 29 of 31 read, which killed
                // "the /proc route is broken on this image" but left the useful
                // question open: WHICH two, and what do they have that the other
                // twenty-nine do not. `comm` is readable without ptrace
                // permission, so it survives exactly the refusal being reported.
                let comm = std::fs::read_to_string(format!("/proc/{name}/comm"))
                    .unwrap_or_default();
                if refused_names.len() < 6 {
                    refused_names.push(format!("{name}={} ({})", comm.trim(), e.kind()));
                }
            }
        }
    }
    // The unit says which variant this is. Reading it from the environment rather
    // than inferring from pid order: three controls start in the same
    // millisecond, and pid order is a guess about systemd's internals - the kind
    // of guess that has been wrong at every step of this investigation.
    let label = std::env::var("ARLEN_KG_PROBE_LABEL").unwrap_or_else(|_| "plain".into());
    println!(
        "kg-probe: proc sweep [{label}]: {mine} same-uid process(es), exe readable for \
         {readable}, refused for {refused}; refused are {refused_names:?}; \
         readable examples {examples:?}"
    );
}

/// Wait until the daemon will accept this caller at all.
///
/// The unit's `ExecStartPre` waits for the knowledge socket to exist, and that is
/// a different condition from being nameable. The socket is bound within a second
/// of boot; the session supervisor registers this unit with the identity broker
/// on its first round, which was 10.1s on the 15 Aug boot. In between the daemon
/// accepts the connection and closes it immediately - correctly, since an
/// unregistered caller has no attested name and is refused rather than handed an
/// empty scope.
///
/// So the first round asked at 5.9s and every question came back `Connection
/// reset by peer`, while the second at 80.8s answered all seven with real rows.
/// Same probe, same daemon; the only difference was that registration had
/// happened. Reporting seven failures for that is reporting a race as a defect.
///
/// A cheap read is the readiness signal, because it is the exact thing being
/// waited for. Bounded, and it does NOT fail the run on timeout: if the wait
/// expires the questions run anyway, and their refusals are the report.
async fn wait_until_named(client: &UnixGraphClient) {
    const DEADLINE: std::time::Duration = std::time::Duration::from_secs(45);
    const POLL: std::time::Duration = std::time::Duration::from_millis(500);
    let start = std::time::Instant::now();
    let mut attempts = 0usize;
    loop {
        attempts += 1;
        if client.access_grants().await.is_ok() {
            println!(
                "kg-probe: named after {attempts} attempt(s), {:?} - the socket existed \
                 from the start, this waits for the supervisor to register the unit",
                start.elapsed()
            );
            return;
        }
        if start.elapsed() >= DEADLINE {
            println!(
                "kg-probe: still refused after {DEADLINE:?} ({attempts} attempts). Asking \
                 anyway - the refusals below are the finding, not this wait"
            );
            return;
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Whether this caller's permission profile is where the daemon will look.
///
/// Asked because a denial says "label outside the caller's read scope" whether
/// the caller was never granted anything or its profile is simply not on disk,
/// and those need different fixes. The daemon resolves the system tier at
/// `/var/lib/arlen/permissions/{uid}/{app_id}.toml`, so that is the exact path
/// checked here - not a plausible-looking one.
///
/// The probe's own profile is written by the verify build phase rather than
/// shipped in `mkosi.extra`, and the extra trees are copied over `$DESTDIR`
/// afterwards. Whether that merges or replaces the directory is the difference
/// between a granted probe and a denied one, and this line answers it from
/// inside the booted image.
fn report_profile() {
    // SAFETY: getuid never fails.
    let uid = unsafe { libc::getuid() };
    let path = format!("/var/lib/arlen/permissions/{uid}/kg-probe.toml");
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let reads = text.lines().filter(|l| l.trim_start().starts_with('"')).count();
            println!("kg-probe: profile: {path} present, {reads} read pattern(s)");
        }
        Err(e) => {
            println!("kg-probe: profile: {path} NOT USABLE: {e}");
        }
    }
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

/// Per column, how many rows carry nothing usable there - or an empty string if
/// every cell of every column is populated.
///
/// Counts only, never values: a column NAME is the query's own text, which this
/// file already prints, while a cell is the user's graph.
fn blanks(rows: &[std::collections::HashMap<String, serde_json::Value>]) -> String {
    let mut columns: Vec<&String> = rows.first().map(|r| r.keys().collect()).unwrap_or_default();
    columns.sort();
    let empty: Vec<String> = columns
        .into_iter()
        .filter_map(|column| {
            let n = rows
                .iter()
                .filter(|row| match row.get(column) {
                    None | Some(serde_json::Value::Null) => true,
                    Some(serde_json::Value::String(s)) => s.is_empty(),
                    Some(_) => false,
                })
                .count();
            (n > 0).then(|| format!("{column} empty in {n}"))
        })
        .collect();
    if empty.is_empty() {
        String::new()
    } else {
        format!(" ({})", empty.join(", "))
    }
}

/// How long the LAST round waits for the ingestion question, and how often it
/// re-asks.
///
/// The rounds above separate "nothing is ever recorded" from "nothing had been
/// recorded yet", and that was enough until the kernel sensor started forwarding
/// file events on 18 August. A boot now puts ~8000 events in the store instead of
/// ~100, promotion works through them at its own pace, and the dogfood's own file
/// arrives late in that queue: measured on the boot that caught it, both rounds
/// answered 0 and the graph read out of the halted image afterwards HELD the file.
/// The question was right and the moment was wrong.
///
/// So the last round asks until it is answered or this budget runs out, and says
/// which. A fixed offset is a guess about how busy the machine is.
const INGEST_POLL_BUDGET: std::time::Duration = std::time::Duration::from_secs(90);
const INGEST_POLL_GAP: std::time::Duration = std::time::Duration::from_secs(5);

/// Poll the ingestion question to a deadline. Returns whether it was answered.
///
/// Only this question polls: the others ask about shape (does the schema have
/// these labels, is the timeline mounted) and their answer does not arrive later,
/// so re-asking them would only make the log longer.
async fn await_ingestion(client: &UnixGraphClient) -> bool {
    let (name, cypher) = QUESTIONS
        .iter()
        .find(|(n, _)| *n == "ingestion: this run's file")
        .expect("the ingestion question is in the list");
    let start = std::time::Instant::now();
    loop {
        if let Ok(rows) = client.query_rows(cypher).await {
            if !rows.is_empty() {
                println!(
                    "kg-probe: {name}: {} row(s) after {}s of waiting",
                    rows.len(),
                    start.elapsed().as_secs()
                );
                return true;
            }
        }
        if start.elapsed() >= INGEST_POLL_BUDGET {
            println!(
                "kg-probe: {name}: still 0 row(s) after {}s. The store may hold it \
                 while promotion has not reached it yet - read the event store \
                 before reading this as an ingestion fault",
                start.elapsed().as_secs()
            );
            return false;
        }
        tokio::time::sleep(INGEST_POLL_GAP).await;
    }
}

/// Ask every question once; answer how many failed.
async fn ask_all(client: &UnixGraphClient) -> usize {
    let mut failures = 0;
    for (name, cypher) in QUESTIONS {
        match client.query_rows(cypher).await {
            // The count is the answer; the rows themselves are the user's content
            // and this prints none of them.
            //
            // Except for how many are EMPTY, per column, which is not content and
            // is the difference between two findings an app renders identically.
            // A row whose `name` cell is null still counts here, and the app drops
            // it (`text(r, "name")?` in projects.rs), so "95 rows" and "an empty
            // pane" can both be true at once. That is exactly what happened on
            // 16 August. Null and empty-string are counted together because the
            // app rejects both.
            Ok(rows) => println!("kg-probe: {name}: {} row(s){}", rows.len(), blanks(&rows)),
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

#[cfg(test)]
mod tests {
    use super::{store_rows_for, INGESTED_PATH, QUESTIONS};

    /// The store read and the graph question must name the same file.
    ///
    /// They are two independent observations of one claim, which is the whole
    /// point of the second reader - and worth nothing if they drift onto
    /// different paths, because then they always agree by never disagreeing.
    #[test]
    fn the_graph_question_asks_about_the_path_the_store_is_asked_for() {
        let asked = QUESTIONS
            .iter()
            .any(|(_, cypher)| cypher.contains(INGESTED_PATH));
        assert!(asked, "no question mentions {INGESTED_PATH}");
    }

    fn seed(dir: &std::path::Path, payloads: &[&str]) -> String {
        let db = dir.join("events.db");
        let conn = rusqlite::Connection::open(&db).expect("create");
        conn.execute_batch(
            "CREATE TABLE events (id TEXT PRIMARY KEY, type TEXT NOT NULL,
             timestamp INTEGER NOT NULL, source TEXT NOT NULL, pid INTEGER NOT NULL,
             origin TEXT NOT NULL, payload BLOB)",
        )
        .expect("schema");
        for (i, p) in payloads.iter().enumerate() {
            conn.execute(
                "INSERT INTO events VALUES (?1, 'file.opened', 0, 'test', 1, 'system:test', ?2)",
                rusqlite::params![format!("e{i}"), p.as_bytes()],
            )
            .expect("insert");
        }
        db.to_string_lossy().into_owned()
    }

    #[test]
    fn an_event_naming_the_path_is_counted() {
        let dir = std::env::temp_dir().join(format!("kgprobe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let db = seed(&dir, &[INGESTED_PATH, "/var/lib/arlen-work/other.md"]);
        assert_eq!(store_rows_for(&db, INGESTED_PATH), Ok(1));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The distinction the probe exists to draw: a store that holds nothing is a
    /// finding, and a store that is not there at all is a different one.
    #[test]
    fn an_empty_store_reads_zero_and_a_missing_one_errs() {
        let dir = std::env::temp_dir().join(format!("kgprobe-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let db = seed(&dir, &[]);
        assert_eq!(store_rows_for(&db, INGESTED_PATH), Ok(0));
        assert!(store_rows_for("/nonexistent/events.db", INGESTED_PATH).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
