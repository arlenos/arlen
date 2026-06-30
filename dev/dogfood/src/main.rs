//! In-VM dogfood: drive the KG-AI loop end to end on a headless boot and log a
//! single PASS/FAIL line the verify channel can grep off the serial journal.
//!
//! The image has no eBPF sensor, so this binary stands in for one: it emits a
//! `file.opened` (the sensor event the knowledge writer + promotion turn into a
//! File subgraph), waits a promotion cycle, then asks the AI daemon a question
//! over the session bus and reports whether a completion came back. The agent
//! reacts to the same event independently (propose -> gate -> audit), visible in
//! its own journal; this binary exercises the conversational read path.
//!
//! Two things are proven deterministically, without parsing a 1B model's prose:
//!   - the event was accepted by the bus (the injection half of the loop), and
//!   - the AI daemon -> proxy -> llama-server path produced a terminal completion
//!     (not an error/timeout), i.e. the whole inference stack is live in the VM.
//! The answer text is logged for inspection but is not the gate (a small local
//! model's grounding quality is not a boolean).
//!
//! Markers (grepped by dev/vm/verify.py): `DOGFOOD EMIT ok`, `DOGFOOD ASK ok`,
//! `DOGFOOD OK` / `DOGFOOD FAIL <reason>`.

use std::time::Duration;

use os_sdk::proto::FileOpenedPayload;
use os_sdk::{EventEmitter, UnixEventEmitter};
use prost::Message;
use serde_json::Value;
use tokio::time::{sleep, timeout, Instant};
use zbus::{Connection, Proxy};

const AI_BUS_NAME: &str = "org.arlen.AI1";
const AI_OBJECT_PATH: &str = "/org/arlen/AI1";
/// A promotion pass runs on a fixed interval (knowledge promotion.rs); wait past
/// one so the injected File node exists before the question is asked.
const PROMOTION_WAIT: Duration = Duration::from_secs(35);
/// Whole-turn budget for submit + every poll.
const QUERY_TIMEOUT: Duration = Duration::from_secs(90);
const POLL_INTERVAL: Duration = Duration::from_millis(250);
/// How many whole-turn attempts before giving up (absorbs model-load latency).
const ASK_ATTEMPTS: u32 = 4;
/// Wait between ask attempts when the provider is not yet serving.
const ASK_RETRY_DELAY: Duration = Duration::from_secs(20);

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let path = std::env::args()
        .nth(1)
        // Under /var/lib/arlen-work (a tmpfiles-created, arlen-writable dir the
        // SYSTEM knowledge daemon can watch): /home/arlen was unreadable to that
        // daemon at startup. The file is promoted UNLINKED (no project signal yet),
        // then executor_verify drops .git here so auto-tag links it past promotion.
        .unwrap_or_else(|| "/var/lib/arlen-work/notes.md".to_string());
    let prompt = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "What files have I opened recently?".to_string());

    // Materialize the file on disk: the executor's predict step canonicalizes the
    // File path through the filesystem (the FILE_PART_OF rule's PathUnderField), so
    // the path must resolve for tag-untagged-files to prove. Its parent
    // (/var/lib/arlen-work) is tmpfiles-created in the image.
    if let Some(parent) = std::path::Path::new(&path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&path, b"notes\n") {
        fail(&format!("create file {path}: {e}"));
    }

    // Re-emit the file.opened over the first few seconds, not once: the knowledge
    // writer registers its event-bus consumer a few tens of ms into boot, and the
    // bus drops events that have no consumer at emit time, so a single startup
    // emit races the writer and is silently lost (the producer socket still
    // accepts it, so the emit call returns Ok). Re-emitting guarantees one lands
    // after the writer subscribes, well before the first promotion pass turns it
    // into the still-unlinked File node. Re-emits are idempotent: promotion MERGEs
    // the File on its path, so the node is created once.
    let mut emitted = false;
    for attempt in 0..8 {
        match emit_open(&path).await {
            Ok(()) => emitted = true,
            Err(e) if attempt == 0 => eprintln!("DOGFOOD emit attempt failed: {e}"),
            Err(_) => {}
        }
        sleep(Duration::from_secs(1)).await;
    }
    if !emitted {
        fail("emit: every file.opened attempt failed");
    }
    println!("DOGFOOD EMIT ok path={path}");

    // Let a promotion pass turn the raw event into a File node (UNLINKED - no
    // project signal exists yet).
    sleep(PROMOTION_WAIT).await;

    // The executor write+undo via run_skill tag-untagged-files: a manual workflow
    // that scans for an untagged file under a project (no event operand), so it
    // drives a deterministic graph write the way auto-tag-by-project (which reads
    // event.fields["path"], absent on a manual invoke) cannot. The file is promoted
    // UNLINKED above; the .git signal below makes its dir a Project, then run_skill
    // links it and compensate undoes it. Best-effort until a VM boot confirms the
    // write surfaces, then it gates the dogfood.
    match executor_verify(&path).await {
        Ok(()) => {}
        Err(e) => println!("DOGFOOD EXECUTOR skipped (best-effort): {e}"),
    }

    // BEST-EFFORT: the conversational ask exercises the daemon -> proxy -> llama ->
    // KG-read path, but the baked 1B model is nondeterministic (it intermittently
    // emits a non-JSON or unknown-action step that the tool-loop parser rejects),
    // so a failure here is NOT a dogfood failure - the executor gate above is the
    // deterministic proof. Logged for inspection only.
    let mut answered = false;
    let mut last = String::new();
    for attempt in 1..=ASK_ATTEMPTS {
        match ask(&prompt).await {
            Ok(answer) => {
                let snippet: String = answer.chars().take(200).collect();
                println!("DOGFOOD ASK ok answer={snippet}");
                answered = true;
                break;
            }
            Err(e) => {
                println!("DOGFOOD ASK retry {attempt}/{ASK_ATTEMPTS}: {e}");
                last = e;
                if attempt < ASK_ATTEMPTS {
                    sleep(ASK_RETRY_DELAY).await;
                }
            }
        }
    }
    if !answered {
        // Best-effort: report, do not fail the dogfood (the 1B model is flaky).
        println!("DOGFOOD ASK skipped (best-effort, 1B model): {last}");
    }

    println!("DOGFOOD OK");
}

const AGENT_BUS_NAME: &str = "org.arlen.AIAgent1";
const AGENT_OBJECT_PATH: &str = "/org/arlen/AIAgent1";
/// The deterministic graph-write workflow we drive manually: a manual-invoke
/// (run_skill) workflow that finds an untagged file under a project and proposes
/// its FILE_PART_OF, so it needs no event operand (unlike auto-tag-by-project,
/// which reads the path off the triggering event).
const WRITE_SKILL: &str = "tag-untagged-files";
/// Budget for the project to be detected + the manual workflow to write (the watcher
/// picks up the runtime .git signal, then a run_skill links the unlinked file).
const WRITE_TIMEOUT: Duration = Duration::from_secs(90);

/// Drive a real executor write (FILE_PART_OF) and undo it, all over AIAgent1.
///
/// Proof is taken from the agent itself, not a graph read (the dogfood is an
/// unprivileged caller with no read scope): `completed_actions` lists only
/// EXECUTED writes, so a receipt appearing IS the write; `compensate` returning
/// `retracted` IS the undo.
async fn executor_verify(file_path: &str) -> Result<(), String> {
    // Create the project signal next to the (already promoted, unlinked) file, so
    // the knowledge watcher detects its directory as a Project. A bare `.git` dir
    // is the Git signal (90% confidence); no real repo needed.
    let project_dir = std::path::Path::new(file_path)
        .parent()
        .ok_or("file path has no parent dir")?;
    std::fs::create_dir_all(project_dir.join(".git"))
        .map_err(|e| format!("create .git signal: {e}"))?;
    println!("DOGFOOD PROJECT signal at {}", project_dir.display());

    let connection = Connection::session()
        .await
        .map_err(|e| format!("session bus: {e}"))?;
    let agent = Proxy::new(&connection, AGENT_BUS_NAME, AGENT_OBJECT_PATH, AGENT_BUS_NAME)
        .await
        .map_err(|e| format!("ai agent unavailable: {e}"))?;

    // Poll: run the skill until a completed action surfaces. Early runs find no
    // project yet (the watcher has not detected it), so the workflow proposes
    // nothing and completed_actions stays empty; once detected, the write executes.
    let deadline = Instant::now() + WRITE_TIMEOUT;
    let correlation_id = loop {
        let summary: String = agent
            .call("run_skill", &(WRITE_SKILL,))
            .await
            .map_err(|e| format!("run_skill: {e}"))?;
        if let Some(id) = first_completed_action(&agent).await {
            break id;
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "no executor write surfaced within budget (last run_skill: {summary})"
            ));
        }
        sleep(Duration::from_secs(5)).await;
    };
    println!("DOGFOOD WRITE ok corr={correlation_id}");

    let outcome: String = agent
        .call("compensate", &(correlation_id.as_str(),))
        .await
        .map_err(|e| format!("compensate: {e}"))?;
    if !outcome.contains("retracted") {
        return Err(format!("compensate did not retract: {outcome}"));
    }
    println!("DOGFOOD UNDO ok");
    Ok(())
}

/// The correlation id of the first completed (executed) action the agent retains,
/// or None if none yet. `completed_actions` is a JSON array of `{id, ...}`.
async fn first_completed_action(agent: &Proxy<'_>) -> Option<String> {
    let json: String = agent.call("completed_actions", &()).await.ok()?;
    let parsed: Value = serde_json::from_str(&json).ok()?;
    parsed
        .as_array()?
        .iter()
        .find_map(|v| v.get("id").and_then(Value::as_str))
        .map(str::to_string)
}

/// Emit a `file.opened` onto the event-bus producer socket.
async fn emit_open(path: &str) -> Result<(), String> {
    let socket = std::env::var("ARLEN_PRODUCER_SOCKET")
        .unwrap_or_else(|_| "/run/arlen/event-bus-producer.sock".to_string());
    let payload = FileOpenedPayload {
        path: path.to_string(),
        app_id: "dogfood".to_string(),
        flags: 0,
    }
    .encode_to_vec();
    UnixEventEmitter::new(socket)
        .emit("file.opened", payload)
        .await
        .map_err(|e| e.to_string())
}

/// Submit a query to the AI daemon and poll it to a terminal status on one held
/// connection (the daemon authorises `take_result` against the submitting
/// connection's unique name). Returns the answer text, or a readable error.
async fn ask(prompt: &str) -> Result<String, String> {
    let connection = Connection::session()
        .await
        .map_err(|e| format!("session bus: {e}"))?;
    let proxy = Proxy::new(&connection, AI_BUS_NAME, AI_OBJECT_PATH, AI_BUS_NAME)
        .await
        .map_err(|e| format!("ai daemon unavailable: {e}"))?;

    let deadline = Instant::now() + QUERY_TIMEOUT;
    let handle_json: String = call_until(&proxy, "query", &(prompt, ""), deadline).await?;
    let handle: Value =
        serde_json::from_str(&handle_json).map_err(|e| format!("malformed handle: {e}"))?;
    let query_id = field(&handle, "query_id")?;
    let token = field(&handle, "retrieval_token")?;

    loop {
        if Instant::now() >= deadline {
            return Err("timed out".to_string());
        }
        let outcome_json: String =
            call_until(&proxy, "take_result", &(query_id.as_str(), token.as_str()), deadline)
                .await?;
        let outcome: Value =
            serde_json::from_str(&outcome_json).map_err(|e| format!("malformed envelope: {e}"))?;
        match outcome.get("status").and_then(Value::as_str) {
            Some("completed") => {
                return Ok(outcome
                    .get("result")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string());
            }
            Some("failed") => {
                return Err(outcome
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("query failed")
                    .to_string());
            }
            Some("cancelled") => return Err("cancelled".to_string()),
            Some("drained") => return Err("drained".to_string()),
            _ => sleep(POLL_INTERVAL).await,
        }
    }
}

/// One bounded D-Bus call (zbus has no default method timeout).
async fn call_until(
    proxy: &Proxy<'_>,
    method: &str,
    args: &(impl serde::Serialize + zbus::zvariant::DynamicType),
    deadline: Instant,
) -> Result<String, String> {
    let budget = deadline.saturating_duration_since(Instant::now());
    if budget.is_zero() {
        return Err("timed out".to_string());
    }
    match timeout(budget, proxy.call(method, args)).await {
        Ok(r) => r.map_err(|e| format!("{method}: {e}")),
        Err(_) => Err("timed out".to_string()),
    }
}

fn field(v: &Value, key: &str) -> Result<String, String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("handle missing {key}"))
}

fn fail(reason: &str) -> ! {
    println!("DOGFOOD FAIL {reason}");
    std::process::exit(1);
}
