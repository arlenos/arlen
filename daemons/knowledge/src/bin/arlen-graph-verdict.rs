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
