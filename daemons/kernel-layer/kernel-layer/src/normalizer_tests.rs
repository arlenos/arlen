/// Tests for the kernel-layer normalizer.
///
/// These tests verify encoding, filtering, and deduplication logic
/// without requiring eBPF or root privileges.

#[cfg(test)]
mod tests {
    use kernel_layer_common::{
        FileOpenedEvent, FileWrittenEvent, NetStateEvent, ProcessExecEvent,
        MAX_COMM_LEN, MAX_PATH_LEN,
    };
    use prost::Message as _;
    use std::io::Read;
    use std::os::unix::net::UnixListener;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tempfile::tempdir;

    mod proto {
        include!(concat!(env!("OUT_DIR"), "/arlen.eventbus.rs"));
    }

    fn make_open_event(pid: u32, path: &str) -> FileOpenedEvent {
        let mut event = FileOpenedEvent {
            pid,
            uid: 1000,
            timestamp_ns: 1_000_000_000,
            ret: 3,
            cgroup_id: 0,
            path: [0u8; MAX_PATH_LEN],
        };
        let bytes = path.as_bytes();
        let len = bytes.len().min(MAX_PATH_LEN - 1);
        event.path[..len].copy_from_slice(&bytes[..len]);
        event
    }

    fn make_exec_event(pid: u32, comm: &str, filename: &str) -> ProcessExecEvent {
        let mut event = ProcessExecEvent {
            pid,
            uid: 1000,
            timestamp_ns: 2_000_000_000,
            comm: [0u8; MAX_COMM_LEN],
            filename: [0u8; MAX_PATH_LEN],
        };
        let cb = comm.as_bytes();
        event.comm[..cb.len().min(MAX_COMM_LEN - 1)]
            .copy_from_slice(&cb[..cb.len().min(MAX_COMM_LEN - 1)]);
        let fb = filename.as_bytes();
        event.filename[..fb.len().min(MAX_PATH_LEN - 1)]
            .copy_from_slice(&fb[..fb.len().min(MAX_PATH_LEN - 1)]);
        event
    }

    fn make_write_event(pid: u32, fd: u64, count: u64) -> FileWrittenEvent {
        FileWrittenEvent {
            pid,
            uid: 1000,
            timestamp_ns: 3_000_000_000,
            fd,
            count,
            cgroup_id: 0,
        }
    }

    fn make_net_event(pid: u32, af: u16, sport: u16, dport: u16) -> NetStateEvent {
        let mut event = NetStateEvent {
            pid,
            uid: 1000,
            timestamp_ns: 4_000_000_000,
            af,
            sport,
            dport,
            _pad: 0,
            saddr: [192, 168, 1, 100],
            daddr: [93, 184, 216, 34],
            saddr_v6: [0u8; 16],
            daddr_v6: [0u8; 16],
            oldstate: 2, // TCP_SYN_SENT
            newstate: 1, // TCP_ESTABLISHED
        };
        event
    }

    /// The uid `make_open_event` stamps, so an assertion can name it.
    const TEST_UID: u32 = 1000;

    /// Encode through the REAL encoder rather than a copy of it.
    ///
    /// This used to rebuild the envelope inline, "using the same logic as
    /// normalizer's encode_envelope" - and a copy of the logic is exactly as
    /// correct as the day it was written. The real encoder hardcoded `uid: 0` for
    /// months, which meant every kernel event claimed root did it and the bus
    /// delivered it to every consumer; no test could see that, because the tests
    /// were checking their own copy.
    fn encode_test(event_type: &str, pid: u32, ts: u64, session_id: &str, payload: Vec<u8>) -> Vec<u8> {
        crate::normalizer::encode_envelope(event_type, pid, TEST_UID, ts, session_id, payload)
            .expect("the envelope encodes")
    }

    #[test]
    fn the_envelope_carries_the_observed_uid() {
        // The regression for the hardcoded zero. It is the routing key for
        // per-user buses: a subject uid that is always 0 sends every observation
        // either everywhere or nowhere.
        let payload = proto::FileOpenedPayload {
            path: "/home/someone/notes.md".into(),
            app_id: "ebpf:1".into(),
            flags: 0,
            cgroup_id: 0,
        };
        let msg = encode_test("file.opened", 1, 1_000_000, "sess", payload.encode_to_vec());
        let decoded = proto::Event::decode(&msg[4..]).expect("the envelope decodes");
        assert_eq!(
            decoded.uid, TEST_UID,
            "the envelope must carry the observed task's uid, not root's"
        );
    }

    // ===== file.opened tests =====

    #[test]
    fn file_opened_payload_encodes_correctly() {
        let payload = proto::FileOpenedPayload {
            path: "/home/tim/file.txt".into(),
            app_id: "ebpf:1234".into(),
            flags: 0,
            cgroup_id: 0,
        };
        let msg = encode_test("file.opened", 1234, 1_000_000, "sess", payload.encode_to_vec());
        let decoded = proto::Event::decode(&msg[4..]).unwrap();
        let p = proto::FileOpenedPayload::decode(decoded.payload.as_slice()).unwrap();
        assert_eq!(p.path, "/home/tim/file.txt");
        assert_eq!(p.app_id, "ebpf:1234");
    }

    // ===== process.started tests =====

    #[test]
    fn process_exec_payload_encodes_correctly() {
        let payload = proto::ProcessLifecyclePayload {
            event_type: "started".into(),
            pid: 5678,
            ppid: 0,
            comm: "firefox".into(),
            exit_code: 0,
            exe_path: "/usr/lib/arlen/apps/dev.arlen.browser/bin/firefox".into(),
            parent_exe_path: "/usr/bin/arlen-terminal".into(),
        };
        let msg = encode_test("process.started", 5678, 2_000_000, "sess", payload.encode_to_vec());
        let decoded = proto::Event::decode(&msg[4..]).unwrap();
        assert_eq!(decoded.r#type, "process.started");
        let p = proto::ProcessLifecyclePayload::decode(decoded.payload.as_slice()).unwrap();
        assert_eq!(p.comm, "firefox");
        assert_eq!(p.event_type, "started");
        assert_eq!(p.ppid, 0);
    }

    #[test]
    fn process_exec_event_struct_roundtrips() {
        let event = make_exec_event(42, "bash", "/usr/bin/bash");
        let bytes = unsafe {
            std::slice::from_raw_parts(
                &event as *const _ as *const u8,
                std::mem::size_of::<ProcessExecEvent>(),
            )
        };
        let cast = unsafe { &*(bytes.as_ptr() as *const ProcessExecEvent) };
        assert_eq!(cast.pid, 42);
        let comm_end = cast.comm.iter().position(|&b| b == 0).unwrap_or(cast.comm.len());
        assert_eq!(std::str::from_utf8(&cast.comm[..comm_end]).unwrap(), "bash");
    }

    // ===== file.written tests =====

    #[test]
    fn file_written_payload_encodes_correctly() {
        let payload = proto::FileWrittenPayload {
            path: "/home/tim/output.log".into(),
            app_id: "ebpf:999".into(),
            bytes: 4096,
            cgroup_id: 4242,
        };
        let msg = encode_test("file.written", 999, 3_000_000, "sess", payload.encode_to_vec());
        let decoded = proto::Event::decode(&msg[4..]).unwrap();
        assert_eq!(decoded.r#type, "file.written");
        let p = proto::FileWrittenPayload::decode(decoded.payload.as_slice()).unwrap();
        assert_eq!(p.path, "/home/tim/output.log");
        assert_eq!(p.bytes, 4096);
    }

    #[test]
    fn file_written_event_struct_roundtrips() {
        let event = make_write_event(100, 5, 1024);
        let bytes = unsafe {
            std::slice::from_raw_parts(
                &event as *const _ as *const u8,
                std::mem::size_of::<FileWrittenEvent>(),
            )
        };
        let cast = unsafe { &*(bytes.as_ptr() as *const FileWrittenEvent) };
        assert_eq!(cast.pid, 100);
        assert_eq!(cast.fd, 5);
        assert_eq!(cast.count, 1024);
    }

    // ===== network tests =====

    #[test]
    fn net_state_payload_encodes_correctly() {
        let payload = proto::NetworkConnectionPayload {
            app_id: "ebpf:200".into(),
            remote_addr: "93.184.216.34:443".into(),
            protocol: "tcp".into(),
            direction: "connect".into(),
        };
        let msg = encode_test("network.connect", 200, 4_000_000, "sess", payload.encode_to_vec());
        let decoded = proto::Event::decode(&msg[4..]).unwrap();
        assert_eq!(decoded.r#type, "network.connect");
        let p = proto::NetworkConnectionPayload::decode(decoded.payload.as_slice()).unwrap();
        assert_eq!(p.remote_addr, "93.184.216.34:443");
        assert_eq!(p.direction, "connect");
    }

    #[test]
    fn net_state_event_struct_roundtrips() {
        let event = make_net_event(300, 2, 54321, 443);
        let bytes = unsafe {
            std::slice::from_raw_parts(
                &event as *const _ as *const u8,
                std::mem::size_of::<NetStateEvent>(),
            )
        };
        let cast = unsafe { &*(bytes.as_ptr() as *const NetStateEvent) };
        assert_eq!(cast.pid, 300);
        assert_eq!(cast.af, 2);
        assert_eq!(cast.dport, 443);
        assert_eq!(cast.daddr, [93, 184, 216, 34]);
    }

    // ===== Shared logic tests =====

    /// The tally line has to distinguish the three ways an open is discarded,
    /// because telling them apart is the whole reason it exists: an empty path
    /// means the probe read nothing, a filtered path means the prefix list ate a
    /// real one, and a dedup means it was a repeat. A line that folded them into
    /// "dropped" would leave the question exactly where it was.
    /// `/proc/<pid>/stat` field 2 is the executable's `comm` and it is NOT
    /// sanitised - a process can be named with spaces and parentheses, which
    /// shifts every field a whitespace split would count. Parsing from the last
    /// `)` is the documented way and this pins it against the hostile name.
    #[test]
    fn the_parent_pid_survives_a_process_named_with_parentheses() {
        use crate::normalizer::ppid_from_stat;

        // The ordinary case: pid, (comm), state, ppid, ...
        assert_eq!(ppid_from_stat("42 (bash) S 7 42 42 0 -1 4194304").unwrap(), 7);

        // A name carrying a close-paren and spaces. A whitespace split would read
        // `3` here, which is a real pid belonging to someone else.
        assert_eq!(
            ppid_from_stat("42 (evil) 1 2 3) S 7 42 42 0 -1").unwrap(),
            7,
            "parsed from the LAST close-paren, not the first"
        );

        // A line with no close-paren at all is refused rather than guessed at.
        assert!(ppid_from_stat("nonsense without parens").is_none());
        // And a truncated line, which is what a read racing an exit can return.
        assert!(ppid_from_stat("42 (bash)").is_none());
    }

    #[test]
    fn a_tally_names_which_discard_it_was() {
        let t = crate::normalizer::Tally { seen: 9, empty_path: 4, blocked: 3, deduped: 1, forwarded: 1 };
        let line = t.line("file.opened").expect("a probe that saw something reports");
        assert!(line.contains("9 seen"), "{line}");
        assert!(line.contains("1 forwarded"), "{line}");
        assert!(line.contains("4 empty path"), "{line}");
        assert!(line.contains("3 filtered by prefix"), "{line}");
        assert!(line.contains("1 deduped"), "{line}");
    }

    /// And a probe that saw nothing says nothing, so one interesting row is not
    /// buried under three rows of zeroes every interval.
    #[test]
    fn a_probe_that_saw_nothing_prints_nothing() {
        assert!(crate::normalizer::Tally::default().line("file.opened").is_none());
    }

    #[test]
    fn blocked_paths_are_filtered() {
        let blocked = [
            "/proc/1/maps", "/sys/kernel/btf/vmlinux", "/dev/null",
            "/run/systemd/private", "/tmp/foo", "/usr/lib/libz.so",
        ];
        let allowed = ["/home/tim/file.txt", "/etc/hostname", "/opt/app/config.toml"];

        // The REAL filter, not a copy of it. This test used to re-implement the
        // prefix list as a closure, so it asserted about its own duplicate and
        // would have passed unchanged while the shipping filter drifted - and that
        // filter is a live suspect in why the open probe forwards nothing.
        use crate::normalizer::is_blocked;
        for p in &blocked { assert!(is_blocked(p), "expected {p} blocked"); }
        for p in &allowed { assert!(!is_blocked(p), "expected {p} allowed"); }
    }

    #[test]
    fn path_extraction_handles_null_terminator() {
        let event = make_open_event(1, "/etc/hostname");
        let end = event.path.iter().position(|&b| b == 0).unwrap_or(event.path.len());
        let path = std::str::from_utf8(&event.path[..end]).unwrap();
        assert_eq!(path, "/etc/hostname");
    }

    #[test]
    fn normalizer_sends_event_to_socket() {
        let tmp = tempdir().unwrap();
        let socket_path = tmp.path().join("producer.sock");
        let path_str = socket_path.to_str().unwrap().to_string();

        let received: Arc<Mutex<Vec<proto::Event>>> = Arc::new(Mutex::new(Vec::new()));
        let received_clone = received.clone();

        let listener = UnixListener::bind(&socket_path).unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                loop {
                    let mut len_buf = [0u8; 4];
                    if stream.read_exact(&mut len_buf).is_err() { break; }
                    let len = u32::from_be_bytes(len_buf) as usize;
                    let mut buf = vec![0u8; len];
                    if stream.read_exact(&mut buf).is_err() { break; }
                    if let Ok(event) = proto::Event::decode(buf.as_slice()) {
                        received_clone.lock().unwrap().push(event);
                    }
                }
            }
        });

        std::thread::sleep(Duration::from_millis(50));

        let payload = proto::FileOpenedPayload {
            path: "/home/tim/test.txt".into(),
            app_id: "ebpf:42".into(),
            flags: 0,
            cgroup_id: 0,
        };
        let msg = encode_test("file.opened", 42, 1_000_000, "test-session", payload.encode_to_vec());

        use std::os::unix::net::UnixStream;
        use std::io::Write;
        let mut stream = UnixStream::connect(&path_str).unwrap();
        stream.write_all(&msg).unwrap();
        drop(stream);

        std::thread::sleep(Duration::from_millis(100));

        let events = received.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].r#type, "file.opened");
        let p = proto::FileOpenedPayload::decode(events[0].payload.as_slice()).unwrap();
        assert_eq!(p.path, "/home/tim/test.txt");
    }
}
