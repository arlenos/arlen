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

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/home/arlen/work/notes.md".to_string());
    let prompt = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "What files have I opened recently?".to_string());

    if let Err(e) = emit_open(&path).await {
        fail(&format!("emit: {e}"));
    }
    println!("DOGFOOD EMIT ok path={path}");

    // Let a promotion pass turn the raw event into a File node before asking.
    sleep(PROMOTION_WAIT).await;

    match ask(&prompt).await {
        Ok(answer) => {
            // One line, truncated: prose is for inspection, not the gate.
            let snippet: String = answer.chars().take(200).collect();
            println!("DOGFOOD ASK ok answer={snippet}");
            println!("DOGFOOD OK");
        }
        Err(e) => fail(&format!("ask: {e}")),
    }
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
