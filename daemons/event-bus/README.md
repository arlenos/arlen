# event-bus

The Arlen Event Bus is a Unix socket daemon that routes structured events between system components. Producers send events; consumers subscribe to event types and receive matching events.

This is the central nervous system of the Arlen data pipeline. Every component that wants to record or react to system activity goes through here.

## What it does

- Accepts producer connections on one Unix socket, consumer connections on another
- Routes events to consumers based on subscription filters (exact match, prefix match, or wildcard)
- Handles backpressure per consumer: a slow consumer gets dropped events, not a stalled bus
- Validates incoming events against required fields before dispatching

## Protocol

Events are length-prefixed protobuf messages. The schema lives in `proto/event.proto`.

**Producer:** connect to `ARLEN_PRODUCER_SOCKET`, send `[4-byte big-endian length][protobuf Event]`

**Consumer:** connect to `ARLEN_CONSUMER_SOCKET`, send registration:
```
<consumer-id>\n
<event-type1>,<event-type2>,...\n
```
Then receive `[4-byte big-endian length][protobuf Event]` messages as they arrive.

Event type filters support:
- Exact match: `file.opened`
- Prefix match: `file.` matches all file events
- Wildcard: `*` matches everything

## Running

```bash
ARLEN_PRODUCER_SOCKET=/run/arlen/event-bus-producer.sock \
ARLEN_CONSUMER_SOCKET=/run/arlen/event-bus-consumer.sock \
RUST_LOG=info \
./event-bus
```

## Configuration

| Variable | Default | Description |
|---|---|---|
| `ARLEN_PRODUCER_SOCKET` | `/run/arlen/event-bus-producer.sock` | Producer socket path |
| `ARLEN_CONSUMER_SOCKET` | `/run/arlen/event-bus-consumer.sock` | Consumer socket path |

## Testing

```bash
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

## Part of

[Arlen](https://github.com/arlenos): a Linux desktop OS built around a system-wide knowledge graph.
