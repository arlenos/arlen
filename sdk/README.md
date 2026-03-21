# sdk

The Lunaris SDK is the library that first-party applications and system components use to interact with the Lunaris platform. It provides a stable interface over the underlying Unix socket protocols so that application code does not need to speak raw protobuf.

## Crates

### `os-sdk`

The system-facing SDK. Applications import this to emit events and query the knowledge graph.

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

**For testing**, use the mock implementations:

```rust
use os_sdk::mock::{MockEventEmitter, MockGraphClient};

let emitter = MockEventEmitter::new();
my_function(&emitter).await;
assert_eq!(emitter.emitted().await[0].event_type, "file.opened");
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
cargo test -p os-sdk                              # unit and mock tests
cargo test -p os-sdk --test unix_implementations  # real socket integration tests
cargo clippy --all-targets --all-features -- -D warnings
```

## Part of

[Lunaris](https://github.com/lunaris-sys): a Linux desktop OS built around a system-wide knowledge graph.
