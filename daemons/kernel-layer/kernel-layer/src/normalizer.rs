/// Normalizer: reads events from multiple eBPF ring buffers,
/// applies deduplication and filtering, and forwards to the Event Bus
/// as length-prefixed protobuf messages.

use aya::maps::RingBuf;
use kernel_layer_common::{FileOpenedEvent, FileWrittenEvent, NetStateEvent, ProcessExecEvent};
use log::{debug, info, warn};
use prost::Message as _;
use std::{
    collections::HashMap,
    io::Write,
    os::unix::net::UnixStream,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

const BLOCKED_PREFIXES: &[&str] = &[
    "/proc/", "/sys/", "/dev/", "/run/", "/tmp/",
    "/usr/lib/", "/usr/lib64/", "/usr/share/", "/usr/bin/",
    "/usr/sbin/", "/lib/", "/lib64/",
];

const DEDUP_WINDOW_OPEN: Duration = Duration::from_millis(100);
const DEDUP_WINDOW_WRITE: Duration = Duration::from_millis(500);
const DEDUP_WINDOW_EXEC: Duration = Duration::from_secs(1);
const DEDUP_WINDOW_NET: Duration = Duration::from_secs(1);

struct DedupEntry {
    last_seen: Instant,
}

mod proto {
    include!(concat!(env!("OUT_DIR"), "/arlen.eventbus.rs"));
}

/// Run the normalizer loop. Blocks until the ring buffers are dropped.
pub fn run<T: std::borrow::Borrow<aya::maps::MapData>>(
    mut ring_open: RingBuf<T>,
    mut ring_exec: RingBuf<T>,
    mut ring_write: RingBuf<T>,
    mut ring_net: RingBuf<T>,
    producer_socket: &str,
    session_id: &str,
) {
    let mut dedup_open: HashMap<(u32, String), DedupEntry> = HashMap::new();
    let mut dedup_write: HashMap<(u32, u64), DedupEntry> = HashMap::new();
    let mut dedup_exec: HashMap<(u32, String), DedupEntry> = HashMap::new();
    let mut dedup_net: HashMap<(u32, u16, u16), DedupEntry> = HashMap::new();
    let mut stream: Option<UnixStream> = None;
    let mut tally_open = Tally::default();
    let mut tally_write = Tally::default();
    let mut last_tally = Instant::now();

    info!("normalizer started, forwarding to {}", producer_socket);

    loop {
        // Say what the probes saw and what became of it. The open probe is the one
        // under investigation, so it is the one counted; the others get the same
        // treatment when they need it rather than pre-emptively.
        if last_tally.elapsed() >= TALLY_INTERVAL {
            for line in [tally_open.line("file.opened"), tally_write.line("file.written")]
                .into_iter()
                .flatten()
            {
                info!("{line}");
            }
            last_tally = Instant::now();
        }

        let mut had_event = false;

        // --- file.opened ---
        while let Some(item) = ring_open.next() {
            had_event = true;
            if let Some(msg) = handle_file_opened(&item, session_id, &mut dedup_open, &mut tally_open) {
                send(&mut stream, producer_socket, &msg);
            }
        }

        // --- process.started ---
        while let Some(item) = ring_exec.next() {
            had_event = true;
            if let Some(msg) = handle_process_exec(&item, session_id, &mut dedup_exec) {
                send(&mut stream, producer_socket, &msg);
            }
        }

        // --- file.written ---
        while let Some(item) = ring_write.next() {
            had_event = true;
            if let Some(msg) = handle_file_written(&item, session_id, &mut dedup_write, &mut tally_write) {
                send(&mut stream, producer_socket, &msg);
            }
        }

        // --- network ---
        while let Some(item) = ring_net.next() {
            had_event = true;
            if let Some(msg) = handle_net_state(&item, session_id, &mut dedup_net) {
                send(&mut stream, producer_socket, &msg);
            }
        }

        if !had_event {
            std::thread::sleep(Duration::from_millis(1));
        }

        // Periodic dedup cleanup
        cleanup_dedup(&mut dedup_open, DEDUP_WINDOW_OPEN * 10);
        cleanup_dedup(&mut dedup_write, DEDUP_WINDOW_WRITE * 10);
        cleanup_dedup(&mut dedup_exec, DEDUP_WINDOW_EXEC * 10);
        cleanup_dedup_net(&mut dedup_net, DEDUP_WINDOW_NET * 10);
    }
}

fn cleanup_dedup<K: std::hash::Hash + Eq>(map: &mut HashMap<K, DedupEntry>, max_age: Duration) {
    if map.len() > 10_000 {
        let now = Instant::now();
        map.retain(|_, v| now.duration_since(v.last_seen) < max_age);
    }
}

fn cleanup_dedup_net(map: &mut HashMap<(u32, u16, u16), DedupEntry>, max_age: Duration) {
    if map.len() > 10_000 {
        let now = Instant::now();
        map.retain(|_, v| now.duration_since(v.last_seen) < max_age);
    }
}

// ===== file.opened handler =====

fn handle_file_opened(
    item: &[u8],
    session_id: &str,
    dedup: &mut HashMap<(u32, String), DedupEntry>,
    tally: &mut Tally,
) -> Option<Vec<u8>> {
    let event = bytemuck_cast::<FileOpenedEvent>(item)?;
    tally.seen += 1;
    let Some(path) = extract_string(&event.path) else {
        tally.empty_path += 1;
        return None;
    };

    if is_blocked(&path) {
        tally.blocked += 1;
        return None;
    }
    if !dedup_check(dedup, (event.pid, path.clone()), DEDUP_WINDOW_OPEN) {
        tally.deduped += 1;
        return None;
    }
    tally.forwarded += 1;

    debug!("file.opened pid={} path={}", event.pid, path);

    let payload = proto::FileOpenedPayload {
        path: path.clone(),
        // Deliberately EMPTY, not `ebpf:<pid>`. The sensor observes a syscall; it
        // does not know which application made it, and inventing a pid-shaped
        // stand-in here silently wins over the cgroup key in
        // `promote_file_opened` - whose whole point is that a pid is reused within
        // a boot and a cgroup id is not. The payload carries `cgroup_id` as its
        // own field; the consumer picks the best key it has from that.
        app_id: String::new(),
        flags: 0,
        cgroup_id: event.cgroup_id,
    };
    encode_envelope("file.opened", event.pid, event.uid, event.timestamp_ns, session_id, payload.encode_to_vec())
}

// ===== process.started handler =====

fn handle_process_exec(
    item: &[u8],
    session_id: &str,
    dedup: &mut HashMap<(u32, String), DedupEntry>,
) -> Option<Vec<u8>> {
    let event = bytemuck_cast::<ProcessExecEvent>(item)?;
    let filename = extract_string(&event.filename).unwrap_or_default();
    let comm = extract_string(&event.comm).unwrap_or_default();

    if !dedup_check(dedup, (event.pid, filename.clone()), DEDUP_WINDOW_EXEC) {
        return None;
    }

    debug!("process.started pid={} comm={} filename={}", event.pid, comm, filename);

    let payload = proto::ProcessLifecyclePayload {
        event_type: "started".into(),
        pid: event.pid,
        ppid: 0,
        comm,
        exit_code: 0,
        // The two cgroups ARE the identities. An app is its cgroup, so a launch
        // is an edge between two of them, and a shell running `ls` puts both ends
        // in the same one - a self-edge, which is not recorded. That is the
        // frequency filter, and it falls out of the identity rather than out of a
        // list of interesting binaries.
        cgroup_id: event.cgroup_id,
        parent_cgroup_id: event.parent_cgroup_id,
        // The child's path, for a human reading a row. It comes from the
        // tracepoint's own buffer, never from `/proc`.
        //
        // `parent_exe_path` is now always empty, and deliberately: it used to be
        // read through `/proc/<ppid>/exe`, which is the resolution route this
        // layer ruled out. A component that resolves identity through `/proc`
        // while carrying systemd hardening stops resolving the first time somebody
        // hardens the unit, and reports a quiet nothing rather than an error. The
        // cgroup carries the identity now, so the `/proc` read has no job left.
        //
        // No argument vector, here or ever: argv carries passwords, tokens and
        // paths, and the executable identity is the whole payload
        // (provenance-halo.md §7).
        exe_path: filename.clone(),
        parent_exe_path: String::new(),
    };
    encode_envelope("process.started", event.pid, event.uid, event.timestamp_ns, session_id, payload.encode_to_vec())
}

// ===== file.written handler =====

fn handle_file_written(
    item: &[u8],
    session_id: &str,
    dedup: &mut HashMap<(u32, u64), DedupEntry>,
    tally: &mut Tally,
) -> Option<Vec<u8>> {
    let event = bytemuck_cast::<FileWrittenEvent>(item)?;
    tally.seen += 1;

    if !dedup_check(dedup, (event.pid, event.fd), DEDUP_WINDOW_WRITE) {
        tally.deduped += 1;
        return None;
    }

    // Resolve fd to path via /proc. Falls back to fd:N if the process is gone.
    let path = resolve_fd(event.pid, event.fd);
    // This probe's version of "the path came back unusable": the fallback, not an
    // empty string. `resolve_fd` never returns empty - it returns `fd:N` when the
    // readlink fails - so counting emptiness here would count nothing, which is
    // the kind of check that reads as evidence and is not.
    if path.starts_with("fd:") {
        tally.empty_path += 1;
    }

    if is_blocked(&path) {
        tally.blocked += 1;
        return None;
    }
    tally.forwarded += 1;

    debug!("file.written pid={} fd={} path={} bytes={}", event.pid, event.fd, path, event.count);

    let payload = proto::FileWrittenPayload {
        path,
        // Empty for the same reason as the open payload: the sensor sees a write,
        // not an application. The cgroup below is the key that survives pid reuse.
        app_id: String::new(),
        bytes: event.count,
        cgroup_id: event.cgroup_id,
    };
    encode_envelope("file.written", event.pid, event.uid, event.timestamp_ns, session_id, payload.encode_to_vec())
}

/// Resolve a file descriptor to a path via /proc/pid/fd/N.
fn resolve_fd(pid: u32, fd: u64) -> String {
    let link = format!("/proc/{pid}/fd/{fd}");
    std::fs::read_link(&link)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| format!("fd:{fd}"))
}

// ===== network handler =====

fn handle_net_state(
    item: &[u8],
    session_id: &str,
    dedup: &mut HashMap<(u32, u16, u16), DedupEntry>,
) -> Option<Vec<u8>> {
    let event = bytemuck_cast::<NetStateEvent>(item)?;

    if !dedup_check(dedup, (event.pid, event.dport, event.sport), DEDUP_WINDOW_NET) {
        return None;
    }

    let (remote_addr, direction) = format_net_event(event);

    debug!("network.{direction} pid={} remote={remote_addr}", event.pid);

    let payload = proto::NetworkConnectionPayload {
        app_id: format!("ebpf:{}", event.pid),
        remote_addr,
        protocol: "tcp".into(),
        direction,
    };

    let event_type = format!("network.{}", payload.direction);
    encode_envelope(&event_type, event.pid, event.uid, event.timestamp_ns, session_id, payload.encode_to_vec())
}

fn format_net_event(event: &NetStateEvent) -> (String, String) {
    // Determine direction: if sport is a well-known port (< 1024), it's likely inbound.
    // Otherwise outbound. This is a heuristic.
    let direction = if event.sport < 1024 { "accept" } else { "connect" };

    let remote = if event.af == 2 {
        // IPv4
        let d = &event.daddr;
        format!("{}:{}", format_ipv4(d), event.dport)
    } else {
        // IPv6
        format!("[{}]:{}", format_ipv6(&event.daddr_v6), event.dport)
    };

    (remote, direction.into())
}

fn format_ipv4(addr: &[u8; 4]) -> String {
    format!("{}.{}.{}.{}", addr[0], addr[1], addr[2], addr[3])
}

fn format_ipv6(addr: &[u8; 16]) -> String {
    let words: Vec<String> = (0..8)
        .map(|i| {
            let hi = addr[i * 2] as u16;
            let lo = addr[i * 2 + 1] as u16;
            format!("{:x}", (hi << 8) | lo)
        })
        .collect();
    words.join(":")
}

// ===== Shared helpers =====

fn dedup_check<K: std::hash::Hash + Eq + Clone>(
    map: &mut HashMap<K, DedupEntry>,
    key: K,
    window: Duration,
) -> bool {
    let now = Instant::now();
    if let Some(entry) = map.get_mut(&key) {
        if now.duration_since(entry.last_seen) < window {
            return false;
        }
        entry.last_seen = now;
    } else {
        map.insert(key, DedupEntry { last_seen: now });
    }
    true
}

fn bytemuck_cast<T: Copy>(bytes: &[u8]) -> Option<&T> {
    if bytes.len() < std::mem::size_of::<T>() {
        warn!("ring buffer item too small: {} bytes (expected {})", bytes.len(), std::mem::size_of::<T>());
        return None;
    }
    Some(unsafe { &*(bytes.as_ptr() as *const T) })
}

/// What a probe saw and what became of it, so a probe that forwards nothing can
/// say WHY rather than looking identical to one that was never fired.
///
/// Built on 18 August after three boots spent guessing. The sensor's journal
/// proved all four tracepoints attach and the store proved `file.opened` and
/// `file.written` still arrive at zero, which leaves three silent discards in
/// each handler - an empty path, the path-prefix filter, and the dedup window -
/// and no way to tell them apart from outside. A count of each is the difference
/// between a diagnosis and another night of hypotheses.
#[derive(Default)]
pub(crate) struct Tally {
    pub(crate) seen: u64,
    pub(crate) empty_path: u64,
    pub(crate) blocked: u64,
    pub(crate) deduped: u64,
    pub(crate) forwarded: u64,
}

impl Tally {
    /// None when this probe has seen nothing, so a quiet probe does not print a
    /// row of zeroes every interval and bury the one that is interesting.
    pub(crate) fn line(&self, probe: &str) -> Option<String> {
        if self.seen == 0 {
            return None;
        }
        Some(format!(
            "{probe}: {} seen, {} forwarded ({} empty path, {} filtered by prefix, {} deduped)",
            self.seen, self.forwarded, self.empty_path, self.blocked, self.deduped
        ))
    }
}

/// How often the tallies are printed. Long enough not to be noise in a journal,
/// short enough that a 90-second verify boot gets at least one.
const TALLY_INTERVAL: Duration = Duration::from_secs(30);
fn extract_string(buf: &[u8]) -> Option<String> {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    let s = std::str::from_utf8(&buf[..end]).ok()?.to_string();
    if s.is_empty() { None } else { Some(s) }
}

pub(crate) fn is_blocked(path: &str) -> bool {
    BLOCKED_PREFIXES.iter().any(|prefix| path.starts_with(prefix))
}

/// Read CLOCK_MONOTONIC, the clock `bpf_ktime_get_ns` stamps events with.
///
/// Not CLOCK_BOOTTIME: that is `bpf_ktime_get_boot_ns`, and it counts suspend.
fn now_monotonic_ns() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: clock_gettime writes only the timespec we hand it.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) } != 0 {
        return 0;
    }
    (ts.tv_sec as u64) * 1_000_000_000 + (ts.tv_nsec as u64)
}

fn now_epoch_micros() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as i64)
        .unwrap_or(0)
}

/// Put a kernel stamp on the same clock as every other producer.
///
/// The kernel stamps with `bpf_ktime_get_ns()`, NANOSECONDS SINCE BOOT. The
/// envelope's `timestamp` is EPOCH MICROSECONDS, which is what installd and
/// every SDK emitter mint and what the knowledge daemon compares against. A
/// boot-relative number in that field is not merely wrong by a factor: it is
/// smaller than any epoch value, so once a single app event advances the
/// promotion high-water mark, `WHERE timestamp > hwm` excludes every kernel
/// event from then on and the whole eBPF feed goes dark without an error.
///
/// Anchoring against the wall clock rather than restamping "now" keeps the
/// event's real age, since the ring buffer can hold one for a while before
/// this daemon drains it.
pub(crate) fn epoch_micros_from_boot_ns(
    boot_ns: u64,
    now_boot_ns: u64,
    now_epoch_micros: i64,
) -> i64 {
    let age_micros = now_boot_ns.saturating_sub(boot_ns) / 1_000;
    now_epoch_micros.saturating_sub(age_micros as i64)
}

pub(crate) fn encode_envelope(
    event_type: &str,
    pid: u32,
    uid: u32,
    timestamp_ns: u64,
    session_id: &str,
    payload: Vec<u8>,
) -> Option<Vec<u8>> {
    let proto_event = proto::Event {
        id: Uuid::now_v7().to_string(),
        r#type: event_type.to_string(),
        timestamp: epoch_micros_from_boot_ns(
            timestamp_ns,
            now_monotonic_ns(),
            now_epoch_micros(),
        ),
        source: "ebpf".to_string(),
        pid,
        origin: session_id.to_string(),
        payload,
        // THE OBSERVED TASK'S UID, not zero.
        //
        // This was hardcoded to 0, which says "root did this" about every open,
        // exec, write and connection on the machine - and uid 0 is the value the
        // bus treats as system, so every kernel event was delivered to every
        // consumer regardless of whose it was. On a multi-user machine that is a
        // privacy boundary that never existed; on a single-user one it is a record
        // that attributes the user's whole day to root.
        //
        // It is also the precondition for per-user buses. The design routes a
        // system observer's events to a user by the SUBJECT's uid, so a subject
        // uid that is always 0 makes that routing meaningless: every event would
        // either go everywhere or nowhere. The eBPF probes have carried this field
        // all along - `kernel-layer-common` has `uid: u32` on all four event
        // structs - and only the envelope dropped it.
        uid,
        project_id: String::new(),
    };

    let encoded = proto_event.encode_to_vec();
    let len = u32::try_from(encoded.len()).ok()?;
    let mut out = Vec::with_capacity(4 + encoded.len());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&encoded);
    Some(out)
}

fn send(stream: &mut Option<UnixStream>, socket_path: &str, msg: &[u8]) {
    if let Err(e) = send_with_reconnect(stream, socket_path, msg) {
        warn!("failed to send event to Event Bus: {e}");
    }
}

fn send_with_reconnect(
    stream: &mut Option<UnixStream>,
    socket_path: &str,
    msg: &[u8],
) -> Result<(), std::io::Error> {
    for attempt in 0..2u8 {
        if stream.is_none() {
            match UnixStream::connect(socket_path) {
                Ok(s) => {
                    info!("connected to Event Bus at {}", socket_path);
                    *stream = Some(s);
                }
                Err(e) => {
                    if attempt == 0 {
                        warn!("Event Bus not available, will retry: {e}");
                        std::thread::sleep(Duration::from_secs(1));
                        continue;
                    }
                    return Err(e);
                }
            }
        }

        match stream.as_mut().unwrap().write_all(msg) {
            Ok(()) => return Ok(()),
            Err(e) => {
                *stream = None;
                if attempt == 1 {
                    return Err(e);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // A plausible wall clock and a machine that has been up eight days.
    const NOW_EPOCH_US: i64 = 1_787_000_000_000_000;
    const NOW_BOOT_NS: u64 = 8 * 24 * 60 * 60 * 1_000_000_000;

    #[test]
    fn an_event_keeps_the_age_it_had_when_the_kernel_stamped_it() {
        let five_seconds_ago = NOW_BOOT_NS - 5_000_000_000;
        let stamped = epoch_micros_from_boot_ns(five_seconds_ago, NOW_BOOT_NS, NOW_EPOCH_US);
        assert_eq!(stamped, NOW_EPOCH_US - 5_000_000);
    }

    #[test]
    fn a_stamp_from_this_instant_lands_on_the_wall_clock() {
        assert_eq!(
            epoch_micros_from_boot_ns(NOW_BOOT_NS, NOW_BOOT_NS, NOW_EPOCH_US),
            NOW_EPOCH_US
        );
    }

    #[test]
    fn a_stamp_from_ahead_of_us_does_not_travel_into_the_future() {
        // Reads on different CPUs can come back slightly out of order.
        let ahead = NOW_BOOT_NS + 2_000_000;
        assert_eq!(
            epoch_micros_from_boot_ns(ahead, NOW_BOOT_NS, NOW_EPOCH_US),
            NOW_EPOCH_US
        );
    }

    #[test]
    fn the_boot_relative_number_never_reaches_the_envelope() {
        // The defect this replaced: `timestamp: timestamp_ns as i64`. A
        // boot-relative value is far below any epoch stamp, so the knowledge
        // daemon's `WHERE timestamp > hwm` drops it the moment one app event
        // moves the mark.
        let raw = NOW_BOOT_NS - 1_000_000_000;
        let stamped = epoch_micros_from_boot_ns(raw, NOW_BOOT_NS, NOW_EPOCH_US);
        assert_ne!(stamped, raw as i64);
        assert!(stamped > NOW_EPOCH_US - 60_000_000);
    }

    #[test]
    fn the_envelope_carries_an_epoch_stamp_whatever_the_kernel_gave_it() {
        // At the producer, not just at the helper. `epoch_micros_from_boot_ns` is
        // unit-tested above with both clocks handed in; this asserts that
        // `encode_envelope` actually consults it, since the defect it replaced
        // was one line in this function passing the kernel's number straight
        // through.
        use prost::Message as _;
        let bytes = encode_envelope("file.opened", 42, 1000, 1_500_000_000, "s", Vec::new())
            .expect("the envelope encodes");
        let event = proto::Event::decode(&bytes[4..]).expect("decodes as an Event");
        // 2020-01-01 in epoch micros. A boot-relative stamp is orders of
        // magnitude below this on any machine that has not been up for decades.
        assert!(
            event.timestamp > 1_577_836_800_000_000,
            "timestamp {} is not an epoch stamp",
            event.timestamp
        );
    }
}
