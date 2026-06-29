//! Dev dogfood injector: emit one synthetic event onto the Arlen event bus.
//!
//! The headless image carries no eBPF sensor, so the KG-AI loop cannot be
//! exercised by really opening a file. This tool stands in for the sensor: it
//! connects to the event-bus producer socket and emits a `file.opened` (the only
//! event type promotion turns into a File + Project subgraph), so an in-VM
//! dogfood can drive event -> KG promotion -> the agent's capability-scoped read.
//!
//! Usage: `arlen-event-emit <absolute-path> [app-id]`
//! Socket: `ARLEN_PRODUCER_SOCKET` env, else `/run/arlen/event-bus-producer.sock`.
//! Exit 0 on a delivered event, 2 on bad args, 1 on an emit failure.

use os_sdk::proto::FileOpenedPayload;
use os_sdk::{EventEmitter, UnixEventEmitter};
use prost::Message;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: arlen-event-emit <absolute-path> [app-id]");
        std::process::exit(2);
    };
    let app_id = args.next().unwrap_or_else(|| "dogfood".to_string());

    let socket = std::env::var("ARLEN_PRODUCER_SOCKET")
        .unwrap_or_else(|_| "/run/arlen/event-bus-producer.sock".to_string());

    // flags 0 == a plain read-open (O_RDONLY); promotion only keys off the path.
    let payload = FileOpenedPayload {
        path: path.clone(),
        app_id,
        flags: 0,
    }
    .encode_to_vec();

    let emitter = UnixEventEmitter::new(socket);
    match emitter.emit("file.opened", payload).await {
        Ok(()) => println!("emitted file.opened path={path}"),
        Err(e) => {
            eprintln!("emit failed: {e}");
            std::process::exit(1);
        }
    }
}
