// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Ask a guest's graph store directly what it contains.
//!
//! Every boot assertion about the KG grades what the guest SAID. The dogfood
//! prints `DOGFOOD WRITE ok` after asking the AGENT whether the agent wrote, and
//! `ai_verdict` greps the journal for that line. Nothing reads the graph. A
//! self-report cannot catch a component that reports wrongly about itself, and
//! the agent is a component.
//!
//! `dev/vm/ingest_verdict.py` already closes this for the event store: it copies
//! the SQLite out of the halted image and asks in SQL on the host. Its docstring
//! records why the graph half stayed open - a byte search of the graph store
//! finds schema strings and not values, so a miss would have two meanings. True
//! of a byte search. But the engine that wrote the store is in this repository,
//! so the host can open the copy and ask properly, which is what this does.
//!
//! It opens the store and NEVER creates anything. `graph::spawn` would run
//! `create_schema` and so answer "no rows" for a table the guest never created,
//! which is exactly the two-meanings failure the SQL check avoided. Here a
//! missing table is its own verdict.
//!
//! Usage:
//!
//! ```text
//! arlen-graph-verdict <store-path> --file <path-the-boot-emitted>
//! ```
//!
//! Exit 0 the graph confirms it, 1 it does not, 2 the store could not be read
//! (which is a different fact and must not read as a clean no).

use lbug::{Connection, Database, SystemConfig};

/// What a single question came back as.
enum Answer {
    Yes,
    No,
    /// The table is not in this store, so the guest never got as far as creating
    /// it. Distinct from an empty answer, on purpose.
    NoTable(String),
}

fn ask(conn: &Connection, cypher: &str) -> Answer {
    match conn.query(cypher) {
        Ok(mut result) => {
            if result.by_ref().count() > 0 {
                Answer::Yes
            } else {
                Answer::No
            }
        }
        Err(e) => Answer::NoTable(e.to_string()),
    }
}

/// The first row of an answer, rendered for a person to read.
///
/// `ask` reduces a query to yes or no and throws the row away, which is right
/// for most of these questions and wrong for the launch one: "at least one app
/// launched another" is true whether the pair is two sensible cgroups or one
/// piece of nonsense, and the whole reason the arm was rebuilt was that the ids
/// it keys on changed. A verdict that can see its subject and does not show it
/// makes the reader boot the image again to ask.
fn first_row(conn: &Connection, cypher: &str) -> Option<String> {
    let mut result = conn.query(cypher).ok()?;
    let row = result.next()?;
    Some(
        row.iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut store = None;
    let mut file = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--file" => {
                file = args.get(i + 1).cloned();
                i += 2;
            }
            other if !other.starts_with('-') && store.is_none() => {
                store = Some(other.to_string());
                i += 1;
            }
            other => {
                eprintln!("unexpected argument: {other}");
                return std::process::ExitCode::from(2);
            }
        }
    }

    let (Some(store), Some(file)) = (store, file) else {
        eprintln!("usage: arlen-graph-verdict <store-path> --file <path>");
        return std::process::ExitCode::from(2);
    };

    if !std::path::Path::new(&store).exists() {
        println!("GRAPH UNREADABLE: no store at {store}");
        return std::process::ExitCode::from(2);
    }

    // Read-only in spirit and in effect: open, ask, exit. Nothing here creates a
    // table, so an absent one stays visible as an absent one.
    let db = match Database::new(&store, SystemConfig::default()) {
        Ok(db) => db,
        Err(e) => {
            // Worth naming rather than folding into a generic failure: a store
            // that will not reopen is the exact defect fixed on 17 August, where
            // `create_schema` ALTERed node tables and broke their key index.
            println!("GRAPH UNREADABLE: the store did not open: {e}");
            return std::process::ExitCode::from(2);
        }
    };
    let conn = match Connection::new(&db) {
        Ok(c) => c,
        Err(e) => {
            println!("GRAPH UNREADABLE: no connection: {e}");
            return std::process::ExitCode::from(2);
        }
    };

    let id = escape(&file);
    let promoted = ask(&conn, &format!("MATCH (f:File {{id: '{id}'}}) RETURN f.id"));
    let linked = ask(
        &conn,
        &format!(
            "MATCH (f:File {{id: '{id}'}})-[r:FILE_PART_OF]->(p:Project) \
             WHERE r.invalid_at IS NULL AND r.expired_at IS NULL RETURN p.id"
        ),
    );

    // The sensor's own trace, read out of the graph rather than out of the event
    // store. `kernel_verdict` asks SQLite whether a row carried `source = 'ebpf'`;
    // this asks the graph whether promotion turned one into a node, which is the
    // question that says the machine-wide sensor reaches the surfaces we ship
    // rather than merely reaching the store.
    //
    // The App id is the join: an open with no app_id of its own is minted
    // `{source}:cgroup:{id}` (or `{source}:{pid}` when there is no cgroup) by
    // `promote_file_opened`, so a node whose id starts with the sensor's source
    // came from the sensor and from nothing else.
    //
    // REPORTED, not folded into the exit code, so `--require-graph` keeps meaning
    // exactly what it meant before this line existed.
    let sensor = ask(
        &conn,
        "MATCH (a:App) WHERE starts_with(a.id, 'ebpf:') RETURN a.id LIMIT 1",
    );
    match &sensor {
        Answer::Yes => println!("GRAPH sensor: yes, an App node minted by the kernel sensor"),
        Answer::No => println!("GRAPH sensor: no App node from the kernel sensor in this store"),
        Answer::NoTable(e) => println!("GRAPH sensor: not measured ({e})"),
    }

    // Whether any exec became a launch relationship. Reported, never gated - but
    // what a "no" MEANS changed on 18 August and the old wording would now
    // mislead whoever reads it.
    //
    // It used to say a bare "no" was correct, because the arm keyed both ends on
    // the executable's path: systemd's binary resolves to no installed app, so
    // nearly every exec on a boot was dropped for an unresolvable parent. The arm
    // is keyed on the CGROUP now, and systemd starting a service puts the two
    // ends in different cgroups, so those execs do resolve. A boot that records
    // no launch at all is therefore closer to suspicious than to expected: the
    // most likely causes are the fork probe not attaching, or its map never being
    // read, both of which look exactly like this and like nothing else.
    //
    // Still not gated, because "suspicious" is not "wrong" - a boot that halts
    // early enough genuinely has none - and a gate that fires on a boot-timing
    // difference teaches people to ignore it.
    const LAUNCH_QUERY: &str =
        "MATCH (p:App)-[l:LAUNCHED]->(c:App) RETURN p.id, c.id, l.count LIMIT 1";
    let launched = ask(&conn, LAUNCH_QUERY);
    match &launched {
        Answer::Yes => match first_row(&conn, LAUNCH_QUERY) {
            Some(row) => println!("GRAPH launches: at least one app launched another ({row})"),
            None => println!("GRAPH launches: at least one app launched another"),
        },
        Answer::No => println!(
            "GRAPH launches: none recorded - worth a look, since cgroup keying \
             should catch systemd starting its services"
        ),
        Answer::NoTable(e) => println!("GRAPH launches: not measured ({e})"),
    }

    for (what, answer) in [("promoted", &promoted), ("linked", &linked)] {
        match answer {
            Answer::Yes => println!("GRAPH {what}: yes"),
            Answer::No => println!("GRAPH {what}: no"),
            Answer::NoTable(e) => println!("GRAPH {what}: the table is not in this store ({e})"),
        }
    }

    match (&promoted, &linked) {
        (Answer::Yes, Answer::Yes) => {
            println!("GRAPH OK: the store itself holds the file and its project link");
            std::process::ExitCode::SUCCESS
        }
        (Answer::NoTable(_), _) | (_, Answer::NoTable(_)) => {
            println!("GRAPH UNREADABLE: the store has no such table, so nothing was measured");
            std::process::ExitCode::from(2)
        }
        (Answer::No, _) => {
            println!("GRAPH FAIL: no File node for {file} - promotion never reached the graph");
            std::process::ExitCode::from(1)
        }
        (_, Answer::No) => {
            println!(
                "GRAPH FAIL: {file} is in the graph but no live FILE_PART_OF - \
                 the agent reported a write the graph does not have"
            );
            std::process::ExitCode::from(1)
        }
    }
}
