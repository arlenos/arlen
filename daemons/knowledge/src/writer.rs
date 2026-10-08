use crate::db;
use crate::proto::Event;
use anyhow::Result;
use prost::Message;
use sqlx::SqlitePool;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::time;
use tracing::{debug, error, info, warn};

/// Maximum number of events in the ring buffer before we start dropping.
const RING_BUFFER_CAPACITY: usize = 10_000;

/// Write a batch when this many events have accumulated.
const BATCH_SIZE_THRESHOLD: usize = 1_000;

/// Write a batch after this duration even if `BATCH_SIZE_THRESHOLD` is not reached.
const BATCH_TIMEOUT: Duration = Duration::from_millis(500);

/// First wait after a failed batch write; doubled per consecutive failure.
const RETRY_BASE: Duration = Duration::from_millis(500);

/// Longest wait between two attempts at a failing batch write.
const RETRY_MAX: Duration = Duration::from_secs(30);

/// Events this writer accepted from the bus and then lost for want of room:
/// the overload tiers' drops. The figure the anomaly detector needs, because a
/// store that silently loses part of what happened looks exactly like a quiet
/// afternoon.
static DROPPED: AtomicU64 = AtomicU64::new(0);

/// How many events have been dropped since the daemon started.
pub fn dropped_events() -> u64 {
    DROPPED.load(Ordering::Relaxed)
}

/// When the next batch write may be tried. A write that fails leaves its batch
/// in the buffer, and this keeps the retries from hammering a store that is
/// already refusing - a full disk or a locked database does not clear in 500ms.
#[derive(Debug, Default)]
struct Retry {
    failures: u32,
    not_before: Option<Instant>,
}

impl Retry {
    fn due(&self, now: Instant) -> bool {
        self.not_before.is_none_or(|t| now >= t)
    }

    fn failed(&mut self, now: Instant) -> Duration {
        self.failures = self.failures.saturating_add(1);
        let wait = RETRY_BASE
            .checked_mul(1u32 << (self.failures - 1).min(16))
            .unwrap_or(RETRY_MAX)
            .min(RETRY_MAX);
        self.not_before = Some(now + wait);
        wait
    }

    fn succeeded(&mut self) {
        *self = Self::default();
    }
}

/// Connect to the Event Bus as a consumer and stream events into `SQLite`.
///
/// This function runs forever. It reconnects automatically if the Event Bus
/// restarts, with a short delay between attempts.
pub async fn run(consumer_socket: &str, pool: SqlitePool) -> Result<()> {
    // The Knowledge app's Pause switch promises "Nothing is added until you
    // resume", so honouring it has to happen HERE, where events enter the store,
    // and not in any view built on top. Read once at startup: `graph.toml`
    // hot-reload is a separate sprint item (the promotion loop says the same
    // about its threshold), so a change takes effect on restart and the app's
    // switch keeps reporting that it could not pause - which is true until the
    // live path exists.
    let paused = crate::timeline_config::paused_flag();
    tokio::spawn(crate::timeline_config::watch_paused(paused.clone()));
    if paused.load(std::sync::atomic::Ordering::Relaxed) {
        info!("graph.toml [timeline] paused = true; events are read and discarded, nothing is stored");
    }
    // Lives across reconnects: a batch that could not be written when the bus
    // went away is still owed to the store, and dropping it with the
    // connection would be the same loss as dropping it on a failed write.
    let mut buffer: Vec<Event> = Vec::with_capacity(RING_BUFFER_CAPACITY);
    let mut retry = Retry::default();
    loop {
        match connect_and_consume(consumer_socket, &pool, &paused, &mut buffer, &mut retry).await {
            Ok(()) => {
                // Clean disconnect; Event Bus shut down intentionally.
                info!("event bus disconnected, waiting to reconnect");
            }
            Err(e) => {
                error!("consumer error: {e}, reconnecting in 2s");
            }
        }
        time::sleep(Duration::from_secs(2)).await;
    }
}

/// Connect to the Event Bus consumer socket, register, and consume events.
async fn connect_and_consume(
    consumer_socket: &str,
    pool: &SqlitePool,
    paused: &std::sync::atomic::AtomicBool,
    buffer: &mut Vec<Event>,
    retry: &mut Retry,
) -> Result<()> {
    let mut stream = UnixStream::connect(consumer_socket).await?;
    info!(socket = consumer_socket, "connected to event bus");

    // Send registration: the bus reads three newline-terminated lines (consumer
    // id, comma-separated event patterns, UID filter). We subscribe to everything
    // ("*") because the Graph Writer stores all events (the promotion pipeline
    // decides later what is worth keeping in Ladybug), across all UIDs ("*"). The
    // third line is mandatory: the bus blocks reading it, so omitting it leaves
    // registration incomplete and the writer never receives any event.
    stream
        .write_all(crate::consumer::registration("graph-writer", "*", "*").as_bytes())
        .await?;

    info!("registered as consumer, starting event loop");

    // The buffer is a bounded staging area, drained on every batch write that
    // succeeds. It only fills toward RING_BUFFER_CAPACITY while writes are
    // failing, which is the case the overload tiers in `admit` exist for.
    let mut interval = time::interval(BATCH_TIMEOUT);

    loop {
        // tokio::select! polls multiple async operations concurrently and
        // executes the branch that completes first. This is how we implement
        // "write when either the buffer is full OR the timeout fires".
        // In C# you would use Task.WhenAny with a CancellationToken.
        tokio::select! {
            // Branch 1: a new event arrived from the socket.
            result = read_event(&mut stream) => {
                match result {
                    Ok(Some(event)) => {
                        // Read off the socket either way, then dropped while
                        // paused: leaving it unread would stall the bus for every
                        // other consumer, which is a different failure from the
                        // one the switch asks for.
                        // Excluded before admitted, not filtered at read: the
                        // row in Settings says "nothing these apps do is
                        // recorded", so the event must not reach the store at
                        // all. Dropping it later would leave the thing the
                        // person asked not to keep sitting in a database, which
                        // is the promise broken quietly instead of loudly.
                        if !paused.load(std::sync::atomic::Ordering::Relaxed)
                            && !excluded(&event)
                        {
                            admit(buffer, event);
                        }
                        if buffer.len() >= BATCH_SIZE_THRESHOLD {
                            flush(buffer, pool, retry).await;
                        }
                    }
                    Ok(None) => {
                        // Clean EOF: event bus closed the connection.
                        debug!("event bus closed connection");
                        flush(buffer, pool, retry).await;
                        return Ok(());
                    }
                    Err(e) => {
                        warn!("read error: {e}");
                        flush(buffer, pool, retry).await;
                        return Err(e);
                    }
                }
            }

            // Branch 2: the 500ms timer fired.
            _ = interval.tick() => {
                if !buffer.is_empty() {
                    flush(buffer, pool, retry).await;
                }
            }
        }
    }
}

/// Admit an event into the ring buffer, applying the three-tier backpressure
/// policy when the buffer is at capacity.
///
/// Whether the user's timeline rules exclude this event from the store.
///
/// Only the two kinds that carry an app or a path are decoded, and that bound is
/// honest rather than lazy: those are the events the promotion pipeline turns
/// into a File or an App, which is what a person means by "recorded". A payload
/// that will not decode is NOT excluded - failing open here keeps an
/// unrecognised producer visible instead of quietly dropping it, and the pause
/// switch above is the control that stops everything.
fn excluded(event: &Event) -> bool {
    match event.r#type.as_str() {
        "file.opened" => match crate::proto::FileOpenedPayload::decode(event.payload.as_slice()) {
            Ok(p) => crate::timeline_config::is_excluded(&p.app_id, &p.path),
            Err(_) => false,
        },
        "file.written" => match crate::proto::FileWrittenPayload::decode(event.payload.as_slice()) {
            Ok(p) => crate::timeline_config::is_excluded(&p.app_id, &p.path),
            Err(_) => false,
        },
        "window.focused" => {
            match crate::proto::WindowFocusedPayload::decode(event.payload.as_slice()) {
                Ok(p) => crate::timeline_config::is_excluded(&p.app_id, ""),
                Err(_) => false,
            }
        }
        _ => false,
    }
}

/// Tier 1: check if the incoming event is a duplicate of one already in the buffer.
///         If so, update the existing event's timestamp and discard the new one.
/// Tier 2: if no duplicate, drop the lowest-value event in the buffer.
///         Raw eBPF read/write events with no app-level context are lowest priority.
/// Tier 3: if no low-value event to drop, discard the incoming event.
fn admit(buffer: &mut Vec<Event>, event: Event) {
    if buffer.len() < RING_BUFFER_CAPACITY {
        buffer.push(event);
        return;
    }

    // Tier 1: duplicate detection
    if let Some(existing) = buffer
        .iter_mut()
        .find(|e| e.r#type == event.r#type && e.source == event.source && e.pid == event.pid)
    {
        existing.timestamp = event.timestamp;
        debug!(event_type = %event.r#type, "deduplicated event in buffer");
        return;
    }

    // Tier 2: drop a low-value eBPF read/write event
    if let Some(pos) = buffer.iter().position(|e| {
        e.source == "ebpf"
            && (e.r#type == "file.read" || e.r#type == "file.write")
            && e.payload.is_empty()
    }) {
        buffer.swap_remove(pos);
        buffer.push(event);
        let total = DROPPED.fetch_add(1, Ordering::Relaxed) + 1;
        debug!(dropped_total = total, "dropped low-value eBPF event to make room");
        return;
    }

    // Tier 3: drop the incoming event
    let total = DROPPED.fetch_add(1, Ordering::Relaxed) + 1;
    warn!(
        event_type = %event.r#type,
        dropped_total = total,
        "ring buffer full, dropping incoming event"
    );
}

/// Write all buffered events to `SQLite`, clearing the buffer only when the
/// write succeeded.
///
/// It used to clear unconditionally, so one failed write - a full disk, a
/// locked database - erased up to a thousand events behind a single log line.
/// Now the batch stays, the next attempt waits out an exponential backoff, and
/// what accumulates meanwhile meets the overload tiers in `admit`, which count
/// what they finally drop.
async fn flush(buffer: &mut Vec<Event>, pool: &SqlitePool, retry: &mut Retry) {
    if buffer.is_empty() || !retry.due(Instant::now()) {
        return;
    }
    match db::write_batch(pool, buffer).await {
        Ok(n) => {
            debug!(count = n, "flushed batch to SQLite");
            if retry.failures > 0 {
                info!(count = n, after_failures = retry.failures, "batch write recovered");
            }
            retry.succeeded();
            buffer.clear();
        }
        Err(e) => {
            let wait = retry.failed(Instant::now());
            error!(
                held = buffer.len(),
                retry_in_ms = wait.as_millis() as u64,
                "batch write failed, keeping the batch: {e}"
            );
        }
    }
}

/// Read one length-prefixed protobuf Event from the stream.
/// Returns Ok(None) on clean EOF, Ok(Some(event)) on success, Err on error.
async fn read_event(stream: &mut UnixStream) -> Result<Option<Event>> {
    let mut len_buf = [0u8; 4];
    match stream.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }

    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 || len > 1024 * 1024 {
        anyhow::bail!("invalid event length: {len}");
    }

    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;

    let event = Event::decode(buf.as_slice())?;
    Ok(Some(event))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_event(event_type: &str, source: &str) -> Event {
        Event {
            id: uuid::Uuid::now_v7().to_string(),
            r#type: event_type.to_string(),
            timestamp: 1_000_000,
            source: source.to_string(),
            pid: 1,
            origin: "session-test".to_string(),
            payload: vec![],
            uid: 0,
            project_id: String::new(),
        }
    }

    #[test]
    fn the_retry_wait_doubles_and_stops_at_the_cap() {
        let now = Instant::now();
        let mut r = Retry::default();
        assert!(r.due(now));
        assert_eq!(r.failed(now), RETRY_BASE);
        assert_eq!(r.failed(now), RETRY_BASE * 2);
        assert!(!r.due(now));
        assert!(r.due(now + RETRY_BASE * 2));
        for _ in 0..40 {
            r.failed(now);
        }
        assert_eq!(r.failed(now), RETRY_MAX);
        r.succeeded();
        assert!(r.due(now));
    }

    #[tokio::test]
    async fn a_failed_write_keeps_the_batch() {
        let dir = tempfile::tempdir().unwrap();
        let pool = db::open(dir.path().join("events.db").to_str().unwrap())
            .await
            .unwrap();
        pool.close().await;
        let mut buffer = vec![make_event("file.opened", "ebpf"), make_event("file.opened", "ebpf")];
        let mut retry = Retry::default();
        flush(&mut buffer, &pool, &mut retry).await;
        assert_eq!(buffer.len(), 2, "a failed write must not discard its batch");
        assert_eq!(retry.failures, 1);
        // Inside the backoff window nothing is attempted.
        flush(&mut buffer, &pool, &mut retry).await;
        assert_eq!(retry.failures, 1);
    }

    #[tokio::test]
    async fn a_successful_write_clears_the_batch() {
        let dir = tempfile::tempdir().unwrap();
        let pool = db::open(dir.path().join("events.db").to_str().unwrap())
            .await
            .unwrap();
        let mut buffer = vec![make_event("file.opened", "ebpf")];
        let mut retry = Retry {
            failures: 3,
            not_before: None,
        };
        flush(&mut buffer, &pool, &mut retry).await;
        assert!(buffer.is_empty());
        assert_eq!(retry.failures, 0);
    }

    #[test]
    fn a_tier_three_drop_is_counted() {
        let before = dropped_events();
        let mut buffer = Vec::with_capacity(RING_BUFFER_CAPACITY);
        for _ in 0..RING_BUFFER_CAPACITY {
            buffer.push(make_event("app.action", "app:com.example"));
        }
        admit(&mut buffer, make_event("network.connection", "ebpf"));
        assert!(dropped_events() > before);
    }

    #[test]
    fn admit_under_capacity() {
        let mut buffer = Vec::new();
        admit(&mut buffer, make_event("file.opened", "ebpf"));
        assert_eq!(buffer.len(), 1);
    }

    #[test]
    fn tier1_deduplication() {
        let mut buffer = Vec::with_capacity(RING_BUFFER_CAPACITY);
        // Fill buffer to capacity with low-value events that are NOT duplicates
        // of our test event (different type).
        for _ in 0..RING_BUFFER_CAPACITY {
            let mut e = make_event("file.read", "ebpf");
            e.pid = 9999; // different pid so tier 1 doesn't match
            buffer.push(e);
        }

        // Add one event that matches what we will try to deduplicate
        let mut original = make_event("window.focused", "wayland");
        original.pid = 42;
        original.timestamp = 100;
        buffer[0] = original;

        let mut duplicate = make_event("window.focused", "wayland");
        duplicate.pid = 42;
        duplicate.timestamp = 200;

        admit(&mut buffer, duplicate);

        // Buffer size unchanged
        assert_eq!(buffer.len(), RING_BUFFER_CAPACITY);
        // Timestamp updated on the existing entry
        let updated = buffer.iter().find(|e| e.r#type == "window.focused").unwrap();
        assert_eq!(updated.timestamp, 200);
    }

    #[test]
    fn tier2_drops_low_value_ebpf() {
        let mut buffer = Vec::with_capacity(RING_BUFFER_CAPACITY);
        for _ in 0..RING_BUFFER_CAPACITY {
            buffer.push(make_event("file.read", "ebpf"));
        }
        let high_value = make_event("app.action", "app:com.example");
        admit(&mut buffer, high_value);
        assert_eq!(buffer.len(), RING_BUFFER_CAPACITY);
        assert!(buffer.iter().any(|e| e.r#type == "app.action"));
    }

    #[test]
    fn tier3_drops_incoming_when_no_low_value_available() {
        let mut buffer = Vec::with_capacity(RING_BUFFER_CAPACITY);
        for _ in 0..RING_BUFFER_CAPACITY {
            buffer.push(make_event("app.action", "app:com.example"));
        }
        let incoming = make_event("network.connection", "ebpf");
        let before_len = buffer.len();
        admit(&mut buffer, incoming);
        assert_eq!(buffer.len(), before_len);
        assert!(!buffer.iter().any(|e| e.r#type == "network.connection"));
    }
}
