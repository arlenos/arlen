//! The retrieval baseline: a scripted week, questions with their gold answers, and
//! a score per category (kg-richness-plan.md, "Measure memory before improving it").
//!
//! The week goes in the way a real one does: events into the SQLite store with
//! `db::write_batch`, then the promotion pass, so a defect in ingestion lowers the
//! score instead of hiding behind direct inserts. Questions are answered by
//! `retrieval::retrieve`, the same call the 0x03 socket makes.
//!
//! Four of the ten categories can be asked of retrieval as it stands and are
//! scored. The other six are listed with the reason they cannot be asked yet, so
//! the table says what is unmeasured rather than leaving it out. Answer quality is
//! not scored here: it needs a model, and this runs in CI without one.
//!
//! Each measured category has a FLOOR, the score on the day it was written. A
//! change that lowers one fails; a change that raises one should raise its floor,
//! which is how the baseline follows the work instead of describing a past.

use prost::Message;

use crate::db;
use crate::graph::GraphHandle;
use crate::project::{Project, ProjectStore};
use crate::proto::{CommandFinishedPayload, Event, FileOpenedPayload, WindowFocusedPayload};

const DAY: i64 = 86_400_000_000;
const T0: i64 = 1_780_000_000_000_000;

struct Week {
    events: Vec<Event>,
    n: u32,
}

impl Week {
    fn push(&mut self, kind: &str, at: i64, payload: Vec<u8>) {
        self.n += 1;
        self.events.push(Event {
            id: format!("ev-{:04}", self.n),
            r#type: kind.into(),
            timestamp: at,
            source: "fixture".into(),
            pid: 4242,
            // One session per day: a session is a sitting, not a week, and the
            // co-access edges the promotion draws are scoped to it.
            origin: format!("day-{}", (at - T0) / DAY),
            payload,
            ..Default::default()
        });
    }
    fn open(&mut self, day: i64, path: &str, app: &str) {
        let p = FileOpenedPayload { path: path.into(), app_id: app.into(), ..Default::default() };
        self.push("file.opened", T0 + day * DAY + i64::from(self.n), p.encode_to_vec());
    }
    fn focus(&mut self, day: i64, app: &str, title: &str) {
        let p = WindowFocusedPayload { app_id: app.into(), window_title: title.into(), ..Default::default() };
        self.push("window.focused", T0 + day * DAY + i64::from(self.n), p.encode_to_vec());
    }
    fn command(&mut self, day: i64, id: &str, line: &str, cwd: &str) {
        let p = CommandFinishedPayload {
            id: id.into(),
            command: line.into(),
            cwd: cwd.into(),
            origin: "you".into(),
            ..Default::default()
        };
        self.push("command.finished", T0 + day * DAY + i64::from(self.n), p.encode_to_vec());
    }
}

const STEUER: &str = "/home/ana/Dokumente/Steuer2026";
const OS: &str = "/home/ana/code/betriebssystem";
const URLAUB: &str = "/home/ana/Dokumente/Urlaub";

/// Ana's week: taxes, a holiday, some code.
fn week() -> Vec<Event> {
    let mut w = Week { events: Vec::new(), n: 0 };
    w.open(0, &format!("{STEUER}/Stromrechnung-Maerz.pdf"), "org.gnome.Evince");
    w.open(0, &format!("{STEUER}/Lohnzettel.pdf"), "org.gnome.Evince");
    w.open(1, &format!("{STEUER}/Rechnungspruefung-Notizen.md"), "org.gnome.TextEditor");
    w.open(1, &format!("{OS}/daemons/knowledge/src/journal.rs"), "dev.zed.Zed");
    w.open(2, &format!("{OS}/daemons/knowledge/src/fts.rs"), "dev.zed.Zed");
    w.command(2, "c-1", "cargo test --release journal", OS);
    w.focus(3, "org.mozilla.firefox", "Flugsuche Helsinki - Firefox");
    w.open(3, &format!("{URLAUB}/Flug-Helsinki.pdf"), "org.gnome.Evince");
    w.open(4, &format!("{URLAUB}/Helsinki-Stadtplan.png"), "org.gnome.Loupe");
    w.open(4, &format!("{URLAUB}/Hotel-Buchung.pdf"), "org.gnome.Evince");
    w.command(5, "c-2", "rsync -a Urlaub/ backup:/fotos", URLAUB);
    w.focus(6, "org.gnome.Calendar", "Quartalsplanung - Kalender");
    w.events
}

struct Question {
    category: &'static str,
    query: &'static str,
    gold: Vec<String>,
}

fn file(dir: &str, name: &str) -> String {
    format!("{dir}/{name}")
}

/// The scored questions. `gold` is what a correct slice must contain; an empty
/// gold is an abstention, where the right answer is to retrieve nothing.
fn questions(project_ids: &[(String, String)]) -> Vec<Question> {
    let project = |name: &str| project_ids.iter().find(|(n, _)| n == name).unwrap().1.clone();
    vec![
        Question { category: "point lookup", query: "Stromrechnung", gold: vec![file(STEUER, "Stromrechnung-Maerz.pdf")] },
        Question { category: "point lookup", query: "Lohnzettel", gold: vec![file(STEUER, "Lohnzettel.pdf")] },
        Question { category: "point lookup", query: "release journal", gold: vec!["c-1".into()] },
        // The project is named nowhere in the words of the file: only the
        // FILE_PART_OF edge connects them.
        Question {
            category: "multi-hop",
            query: "journal.rs",
            gold: vec![file(&format!("{OS}/daemons/knowledge/src"), "journal.rs"), project("Betriebssystem")],
        },
        Question {
            category: "multi-hop",
            query: "Hotel-Buchung.pdf",
            gold: vec![file(URLAUB, "Hotel-Buchung.pdf"), project("Urlaub")],
        },
        // Words shared with the answer: the map shares "Helsinki" with the flight.
        Question { category: "collision", query: "Flug Helsinki", gold: vec![file(URLAUB, "Flug-Helsinki.pdf")] },
        Question { category: "collision", query: "Rechnung", gold: vec![file(STEUER, "Stromrechnung-Maerz.pdf")] },
        Question { category: "abstention", query: "Steuerberater Termin", gold: vec![] },
        Question { category: "abstention", query: "Zahnarzt", gold: vec![] },
    ]
}

/// The six categories retrieval cannot be asked yet, and why.
const UNMEASURED: &[(&str, &str)] = &[
    ("knowledge update", "retrieval returns ids, with no notion of the current value of something that changed"),
    ("as-of (transaction time)", "retrieve takes no as_of"),
    ("valid-time interval", "retrieve takes no interval"),
    ("identity", "no Mention, REFERS_TO or SAME_AS yet (item 8)"),
    ("proactive", "nothing retrieves on an event without a question"),
    ("post-correction", "no curation commands to replay yet (item 8)"),
];

/// The floor per measured category: (category, precision, recall), the scores on
/// 8 October 2026. Raise one when a change raises the score.
const FLOORS: &[(&str, f64, f64)] = &[
    ("point lookup", 0.46, 1.0),
    ("multi-hop", 0.39, 1.0),
    ("collision", 0.15, 1.0),
    ("abstention", 1.0, 1.0),
];

async fn ingest() -> (sqlx::SqlitePool, GraphHandle, Vec<(String, String)>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::open(dir.path().join("events.db").to_str().unwrap()).await.unwrap();
    let graph = crate::graph::spawn(dir.path().join("graph").to_str().unwrap()).unwrap();
    crate::fts::create_fact_text_index(&pool).await.unwrap();
    let store = ProjectStore::new(graph.clone(), pool.clone());
    let mut ids = Vec::new();
    for (name, root) in [("Steuer 2026", STEUER), ("Betriebssystem", OS), ("Urlaub", URLAUB)] {
        let p = Project::new_explicit(uuid::Uuid::now_v7(), name.into(), root.into());
        store.create(&p).await.unwrap();
        ids.push((name.to_string(), p.id.to_string()));
    }
    db::write_batch(&pool, &week()).await.unwrap();
    crate::promotion::run_pass(&pool, &graph, &store, 3).await.unwrap();
    // A second pass picks up what the first one indexed after its event loop.
    crate::promotion::run_pass(&pool, &graph, &store, 3).await.unwrap();
    (pool, graph, ids, dir)
}

#[tokio::test]
async fn retrieval_baseline_holds() {
    let (pool, graph, ids, _dir) = ingest().await;
    let mut per: std::collections::BTreeMap<&str, (f64, f64, usize)> = Default::default();
    let mut lines = Vec::new();
    for q in questions(&ids) {
        let got = crate::retrieval::retrieve(&pool, &graph, q.query, 10).await.unwrap();
        let hit = got.iter().filter(|g| q.gold.contains(g)).count() as f64;
        let (p, r) = if q.gold.is_empty() {
            let clean = if got.is_empty() { 1.0 } else { 0.0 };
            (clean, clean)
        } else {
            let p = if got.is_empty() { 0.0 } else { hit / got.len() as f64 };
            (p, hit / q.gold.len() as f64)
        };
        lines.push(format!("  {:<14} {:<22} p {:.2} r {:.2}  got {got:?}", q.category, q.query, p, r));
        let e = per.entry(q.category).or_insert((0.0, 0.0, 0));
        e.0 += p;
        e.1 += r;
        e.2 += 1;
    }
    println!("retrieval baseline, per question:\n{}", lines.join("\n"));
    println!("per category:");
    let mut below = Vec::new();
    for (cat, (p, r, n)) in &per {
        let (p, r) = (p / *n as f64, r / *n as f64);
        println!("  {cat:<14} precision {p:.2} recall {r:.2} over {n}");
        let floor = FLOORS.iter().find(|f| f.0 == *cat).expect("every measured category has a floor");
        if p + 1e-9 < floor.1 || r + 1e-9 < floor.2 {
            below.push(format!("{cat}: precision {p:.2} recall {r:.2}, floor {:.2}/{:.2}", floor.1, floor.2));
        }
    }
    for (cat, why) in UNMEASURED {
        println!("  {cat:<24} not measured: {why}");
    }
    assert!(below.is_empty(), "retrieval fell below its baseline: {below:?}");
}
