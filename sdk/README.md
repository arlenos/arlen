# sdk

The Lunaris SDK is the library that first-party applications and system components use to interact with the Lunaris platform. It provides a stable interface over the underlying Unix socket protocols so that application code does not need to speak raw protobuf.

## Crates

### `os-sdk`

The system-facing SDK. Applications import this to emit events, query the knowledge graph, and read or write the clipboard.

```rust
use os_sdk::{UnixEventEmitter, EventEmitter, UnixGraphClient, GraphClient};
use std::collections::HashMap;

let emitter = UnixEventEmitter::new("/run/lunaris/event-bus-producer.sock");
emitter.emit("app.action", payload_bytes).await?;

let client = UnixGraphClient::new("/run/lunaris/knowledge.sock");
let rows = client.query(
    "MATCH (f:File) WHERE f.app_id = $app RETURN f.path LIMIT 10",
    HashMap::new(),
).await?;
```

Both `UnixEventEmitter` and `UnixGraphClient` reconnect automatically if the daemon restarts.

The clipboard client talks to the desktop-shell broker (`$XDG_RUNTIME_DIR/lunaris/clipboard.sock`):

```rust
use os_sdk::{UnixClipboardClient, WriteParams, ClipboardLabel};

let cb = UnixClipboardClient::connect().await?;
cb.write(WriteParams {
    content: b"hello".to_vec(),
    mime: "text/plain".into(),
    label: ClipboardLabel::Normal,
}).await?;

let entry = cb.read().await?;
let mut events = cb.subscribe().await?;
while let Some(entry) = events.recv().await {
    println!("clipboard changed: {}", entry.id);
}
```

See `docs/architecture/clipboard-api.md` for the broker design and sensitivity-label rules.

Apps can subscribe to annotation changes with the consumer-side Event Bus client:

```rust
use os_sdk::{Annotations, AnnotationChange, AnnotationTarget,
             UnixEventConsumer, UnixEventEmitter, UnixGraphClient};

let consumer = UnixEventConsumer::new("/run/lunaris/event-bus-consumer.sock");
let ann = Annotations::new(emitter, graph, "com.example.editor");

let mut sub = ann.on_changed(
    &consumer,
    AnnotationTarget::File { path: "/notes.md".into() },
    "com.example.editor".into(),
).await?;

while let Some(change) = sub.recv().await {
    match change {
        AnnotationChange::Set { data, .. } => println!("set: {data}"),
        AnnotationChange::Cleared { .. } => println!("cleared"),
    }
}
// Drop sub to unsubscribe.
```

See `docs/architecture/annotations-api.md` for the consumer-side wire protocol, lifecycle, and edge-case coverage.

**For testing**, use the mock implementations:

```rust
use os_sdk::mock::{MockEventConsumer, MockEventEmitter, MockGraphClient};
use os_sdk::MockClipboardClient;

let emitter = MockEventEmitter::new();
my_function(&emitter).await;
assert_eq!(emitter.emitted().await[0].event_type, "file.opened");

let clipboard = MockClipboardClient::new();
let consumer = MockEventConsumer::new();
```

### `module-sdk`

Stub crate for module-specific APIs. Will be expanded in Phase 2.

## Environment variables

`os-sdk` reads the following at runtime:

| Variable | Used for |
|---|---|
| `LUNARIS_APP_ID` | Identifies the emitting app in events |
| `LUNARIS_SESSION_ID` | Session identifier attached to all events |

## Testing

```bash
cargo test -p os-sdk                                  # unit and mock tests
cargo test -p os-sdk --test unix_implementations      # event-bus + graph socket integration
cargo test -p os-sdk --test clipboard_integration     # clipboard broker socket integration
cargo test -p os-sdk --test event_consumer_integration # event-bus consumer-side integration
cargo clippy --all-targets --all-features -- -D warnings
```

## Part of

[Lunaris](https://github.com/lunaris-sys): a Linux desktop OS built around a system-wide knowledge graph.
