//! Dev dogfood injector: emit one synthetic event onto the Arlen event bus.
//!
//! The headless image carries no eBPF sensor, so the KG-AI loop cannot be
//! exercised by really opening a file. This tool stands in for the sensor: it
//! connects to the event-bus producer socket and emits a `file.opened` (the only
//! event type promotion turns into a File + Project subgraph), so an in-VM
//! dogfood can drive event -> KG promotion -> the agent's capability-scoped read.
//!
//! IT CONFIRMS DELIVERY RATHER THAN CLAIMING IT. It used to print `emitted` and
//! exit 0 as soon as the bytes reached the socket, which is not the same thing:
//! emitting is fire-and-forget, so a rejected event and a delivered one looked
//! identical from here. On 16 August that cost a full diagnosis - two events
//! reported `emitted`, the graph stayed empty, and the reason was in the BUS log
//! (`dropping invalid event error=missing required field: origin`) because
//! `ARLEN_SESSION_ID` was unset in the scratch stack. The SDK detects exactly
//! that and says so through `tracing::error!`, which a binary with no subscriber
//! discards. So this now subscribes before it emits and waits to see its own
//! event come back around, and says nothing at all about success until it does.
//!
//! Usage: `arlen-event-emit <absolute-path> [app-id]`
//! Sockets: `ARLEN_PRODUCER_SOCKET` / `ARLEN_CONSUMER_SOCKET`, else `/run/arlen/`.
//! Session: `ARLEN_SESSION_ID` must name the session the open belongs to.
//! Exit 0 on a CONFIRMED event, 2 on bad args or no session, 1 on emit failure
//! or on an event that never came back.

use os_sdk::event_consumer::{EventConsumer, UnixEventConsumer};
use os_sdk::proto::FileOpenedPayload;
use os_sdk::{EventEmitter, UnixEventEmitter};
use prost::Message;
use std::time::Duration;

/// How long to wait for the bus to hand the event back to us.
///
/// Generous for a loopback socket. A rejected event never arrives at all, so this
/// is the difference between "refused" and "slow", and being slow is not a thing
/// a local Unix socket does.
const CONFIRM_WAIT: Duration = Duration::from_secs(3);

/// How long to let a subscription land before emitting against it.
///
/// Two orders of magnitude over the gap that was actually measured, because being
/// slow here costs a fifth of a second on a dev tool and being early makes it
/// report a delivered event as dropped.
const REGISTRATION_SETTLE: Duration = Duration::from_millis(200);

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: arlen-event-emit <absolute-path> [app-id]");
        std::process::exit(2);
    };
    let app_id = args.next().unwrap_or_else(|| "dogfood".to_string());

    // CHECKED HERE, not left to the SDK's log line. `UnixEventEmitter::new` reads
    // the session id and refuses to invent one when it is missing - correct, and
    // it reports the miss with `tracing::error!`, which goes nowhere in a binary
    // that installs no subscriber. The result was a tool that knew the event
    // would be refused and printed `emitted` anyway. An empty origin is a
    // guaranteed rejection at the bus, so there is nothing to send.
    let session = std::env::var("ARLEN_SESSION_ID").unwrap_or_default();
    if session.is_empty() {
        eprintln!(
            "ARLEN_SESSION_ID is unset, so this event would carry an empty origin \
             and the bus would refuse it (missing required field: origin)."
        );
        eprintln!(
            "A file open belongs to a session and the id is not something this tool \
             may invent. Name it:  ARLEN_SESSION_ID=dogfood arlen-event-emit {path}"
        );
        std::process::exit(2);
    }

    // The SDK's resolution, same as dogfood's, and for the same reason: the
    // written-out fallback stopped naming anything the day the bus went per-user.
    let producer = os_sdk::runtime::socket_path("ARLEN_PRODUCER_SOCKET", "event-bus-producer.sock")
        .to_string_lossy()
        .into_owned();
    let consumer = os_sdk::runtime::socket_path("ARLEN_CONSUMER_SOCKET", "event-bus-consumer.sock")
        .to_string_lossy()
        .into_owned();

    // Subscribe BEFORE emitting. The bus fans out to whoever is registered at the
    // moment an event arrives, so a consumer that registers afterwards has already
    // missed it - the same ordering the integration suite had to learn.
    let mut inbox = match UnixEventConsumer::new(consumer.clone())
        .subscribe(vec!["file.opened".to_string()])
        .await
    {
        Ok(rx) => rx,
        Err(e) => {
            eprintln!("cannot watch for the event on {consumer}: {e}");
            eprintln!("without a subscription this tool cannot tell delivery from silence");
            std::process::exit(1);
        }
    };

    // Let the registration take effect. `subscribe` returns once the three lines are
    // WRITTEN, and the bus reads them on a different task from the one serving the
    // producer - so an emit that follows immediately can be dispatched before the
    // registry has the new consumer in it. Measured on 16 August: publish at
    // .901624, `consumer registered` at .901729, and the event was correctly
    // delivered to graph-writer while this tool saw nothing and called it a drop.
    // The protocol has no registration ack, so a settle is the honest way to wait
    // for something that cannot be observed.
    tokio::time::sleep(REGISTRATION_SETTLE).await;

    // flags 0 == a plain read-open (O_RDONLY); promotion only keys off the path.
    let payload = FileOpenedPayload {
        path: path.clone(),
        app_id,
        flags: 0,
    }
    .encode_to_vec();

    let emitter = UnixEventEmitter::new(producer);
    if let Err(e) = emitter.emit("file.opened", payload).await {
        eprintln!("emit failed: {e}");
        std::process::exit(1);
    }

    // Match on the path rather than the event id: the SDK mints the id inside
    // `emit`, so the caller never learns it. Another producer emitting the same
    // path in the same three seconds would satisfy this, which on a dev injector
    // is a trade worth making for a confirmation that is otherwise impossible.
    let confirmed = tokio::time::timeout(CONFIRM_WAIT, async {
        while let Some(event) = inbox.recv().await {
            if event.r#type != "file.opened" {
                continue;
            }
            if let Ok(p) = FileOpenedPayload::decode(&event.payload[..]) {
                if p.path == path {
                    return Some(event.origin);
                }
            }
        }
        None
    })
    .await;

    match confirmed {
        Ok(Some(origin)) => {
            println!("delivered file.opened path={path} origin={origin}");
        }
        Ok(None) => {
            eprintln!("the bus closed the subscription before the event came back");
            std::process::exit(1);
        }
        Err(_) => {
            eprintln!(
                "file.opened path={path} was written to the socket but never came back \
                 within {}s: the bus accepted the bytes and dropped the event.",
                CONFIRM_WAIT.as_secs()
            );
            eprintln!(
                "The bus log says which field it refused. Run it with RUST_LOG=debug and \
                 look for `dropping invalid event`."
            );
            std::process::exit(1);
        }
    }
}
