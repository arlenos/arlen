// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! What one process is actually holding: its open files, its sockets, and
//! whether a camera or microphone is among them (system-monitor-plan.md (c) and
//! (d)3, the Activity-Monitor "Open Files and Ports" analogue).
//!
//! WHAT THIS REPLACES. The detail pane invented all of it. Every process was
//! shown three plausible paths built from its own name, and any process with
//! network traffic got `tcp 140.82.121.4:443 ESTABLISHED` - a real GitHub
//! address, printed for a process nobody had looked at. It read as a finding.
//! The pane also decided camera and microphone from a hand-keyed table of six
//! program names, so a call in an unlisted browser showed no camera while it was
//! filming, and a listed one showed a camera while it sat idle.
//!
//! ABSENT IS NOT EMPTY, and this is the whole design. `/proc/<pid>/fd` is
//! readable only by the process owner (and root), so for any process belonging
//! to another user the honest answer is "not measured" - NOT an empty list,
//! which says "this process holds nothing open" and is the more dangerous claim
//! of the two on a screen about what programs can reach. Every field here is an
//! `Option`, and the caller renders `None` as unmeasured.
//!
//! WHAT IT DOES NOT DO. It does not report the KG capability scopes the plan
//! also wants in this pane. Those live behind the knowledge daemon's
//! `access_grants` op, which scopes every answer to the CALLER's own attested
//! app id; the whole-machine view is gated on `is_privileged_authority_reader`,
//! which returns false for every caller until F3. So a task manager cannot read
//! another process's grants today, by the daemon's deliberate design, and
//! pretending otherwise would be the same defect this module removes.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// A socket the process holds, resolved from its fd to a real endpoint.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    /// `tcp`, `tcp6`, `udp` or `udp6`.
    pub proto: String,
    /// `address:port` as the kernel reports it, decoded from its hex form.
    pub local: String,
    /// The peer, or `*` for a socket that has none (a listener, or an unbound
    /// datagram socket).
    pub peer: String,
    /// `LISTEN`, `ESTABLISHED` and the rest, for TCP. UDP has no state machine,
    /// so it reports `-` rather than borrowing a TCP word.
    pub state: String,
}

/// What one process holds open. Every field distinguishes "measured and empty"
/// from "could not look".
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeldResources {
    /// Regular files the process has open, deduplicated and sorted. `None` when
    /// `/proc/<pid>/fd` could not be read.
    pub open_files: Option<Vec<String>>,
    /// Sockets, resolved through the network tables. `None` for the same reason.
    pub connections: Option<Vec<Connection>>,
    /// Does it hold a `/dev/video*` node open? `None` when unmeasured. A camera
    /// that is open is not necessarily recording, and this says "open", which is
    /// the strongest thing an fd can support.
    pub camera: Option<bool>,
    /// Does it hold an audio capture device open? Same caveat.
    pub mic: Option<bool>,
    /// Why the answers above are `None`, in plain words, so the pane can say
    /// which kind of nothing this is.
    pub unreadable: Option<String>,
}

/// Cap on how many fds are reported. A browser can hold thousands; the pane
/// shows a list, not a database, and an unbounded read of a huge fd table is
/// work done on the machine the user came here to relieve. The count reported is
/// the number KEPT, and the caller says so rather than implying it is the total.
pub const MAX_LISTED: usize = 200;

/// Read what `pid` holds, under `proc_root` (`/proc` in production, a fixture in
/// tests).
pub fn held_resources(proc_root: &Path, pid: u32) -> HeldResources {
    let fd_dir = proc_root.join(pid.to_string()).join("fd");
    let entries = match fs::read_dir(&fd_dir) {
        Ok(e) => e,
        Err(err) => {
            // The two cases a user meets, told apart, because they mean
            // different things: one is "not yours to see", the other is "it
            // exited while you were looking".
            let why = match err.kind() {
                std::io::ErrorKind::PermissionDenied => {
                    "not measured: this process belongs to another user"
                }
                std::io::ErrorKind::NotFound => "not measured: the process has exited",
                _ => "not measured: its file table could not be read",
            };
            return HeldResources { unreadable: Some(why.to_string()), ..Default::default() };
        }
    };

    let mut files: Vec<String> = Vec::new();
    let mut socket_inodes: Vec<u64> = Vec::new();
    let mut camera = false;
    let mut mic = false;

    for entry in entries.flatten() {
        // A dangling fd link is normal: fds close while the directory is being
        // walked. Skipping one is right; failing the whole read is not.
        let Ok(target) = fs::read_link(entry.path()) else { continue };
        let target = target.to_string_lossy().to_string();

        if let Some(inode) = target.strip_prefix("socket:[").and_then(|s| s.strip_suffix(']')) {
            if let Ok(n) = inode.parse::<u64>() {
                socket_inodes.push(n);
            }
            continue;
        }
        // pipe:[n], anon_inode:[…] and the rest are not files the user opened.
        if target.contains(":[") {
            continue;
        }
        if is_camera(&target) {
            camera = true;
        }
        if is_mic(&target) {
            mic = true;
        }
        files.push(target);
    }

    files.sort();
    files.dedup();
    files.truncate(MAX_LISTED);

    let mut connections = resolve_sockets(proc_root, &socket_inodes);
    connections.truncate(MAX_LISTED);

    HeldResources {
        open_files: Some(files),
        connections: Some(connections),
        camera: Some(camera),
        mic: Some(mic),
        unreadable: None,
    }
}

/// A video capture device. `/dev/video*` is the V4L2 node every webcam presents;
/// `/dev/media*` is the media controller beside it and is NOT capture on its own,
/// so it does not count.
fn is_camera(path: &str) -> bool {
    path.starts_with("/dev/video")
}

/// An audio capture device. ALSA capture nodes are `/dev/snd/pcmC<n>D<n>c` - the
/// trailing `c` is capture and `p` is playback, so matching the whole directory
/// would report a microphone for anything playing a sound.
fn is_mic(path: &str) -> bool {
    path.starts_with("/dev/snd/pcm") && path.ends_with('c')
}

/// Resolve socket inodes against the kernel's network tables.
fn resolve_sockets(proc_root: &Path, inodes: &[u64]) -> Vec<Connection> {
    if inodes.is_empty() {
        return Vec::new();
    }
    let mut table: HashMap<u64, Connection> = HashMap::new();
    for (file, proto) in
        [("tcp", "tcp"), ("tcp6", "tcp6"), ("udp", "udp"), ("udp6", "udp6")]
    {
        let path = proc_root.join("net").join(file);
        let Ok(text) = fs::read_to_string(&path) else { continue };
        for (inode, conn) in parse_net_table(&text, proto) {
            table.insert(inode, conn);
        }
    }
    let mut out: Vec<Connection> = inodes.iter().filter_map(|i| table.get(i).cloned()).collect();
    out.sort_by(|a, b| (&a.proto, &a.local).cmp(&(&b.proto, &b.local)));
    out.dedup();
    out
}

/// Parse one `/proc/net/{tcp,udp,...}` table into (inode, connection) pairs.
///
/// The columns are fixed by the kernel: `sl local_address rem_address st ...`
/// then the inode at index 9. Pure, so the hex decoding is testable against the
/// real lines rather than against a live socket.
pub fn parse_net_table(text: &str, proto: &str) -> Vec<(u64, Connection)> {
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 10 {
            continue;
        }
        let Some(local) = decode_endpoint(f[1]) else { continue };
        let peer = decode_endpoint(f[2]).unwrap_or_else(|| "*".into());
        // A peer of 0.0.0.0:0 is not a peer; it is a socket that has none.
        let peer = if peer.ends_with(":0") { "*".to_string() } else { peer };
        let Ok(inode) = f[9].parse::<u64>() else { continue };
        let state = if proto.starts_with("udp") {
            // UDP has no connection state. Borrowing a TCP word here would put
            // "ESTABLISHED" next to a datagram socket that established nothing.
            "-".to_string()
        } else {
            tcp_state(f[3]).to_string()
        };
        out.push((inode, Connection { proto: proto.to_string(), local, peer, state }));
    }
    out
}

/// `0100007F:0050` to `127.0.0.1:80`.
///
/// The address is little-endian per 32-bit word, which is why the octets come
/// out reversed and why an IPv6 address is grouped in fours before reversing.
/// Reading it left to right gives a plausible-looking address that is wrong,
/// which is the worst kind: `0100007F` reads as `1.0.0.127`.
fn decode_endpoint(field: &str) -> Option<String> {
    let (addr, port) = field.split_once(':')?;
    let port = u16::from_str_radix(port, 16).ok()?;
    let ip = match addr.len() {
        8 => {
            let n = u32::from_str_radix(addr, 16).ok()?;
            let b = n.to_le_bytes();
            format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3])
        }
        32 => {
            let mut groups = Vec::new();
            for word in 0..4 {
                let w = u32::from_str_radix(&addr[word * 8..word * 8 + 8], 16).ok()?;
                let b = w.to_le_bytes();
                groups.push(format!("{:02x}{:02x}", b[0], b[1]));
                groups.push(format!("{:02x}{:02x}", b[2], b[3]));
            }
            format!("[{}]", groups.join(":"))
        }
        _ => return None,
    };
    Some(format!("{ip}:{port}"))
}

/// The kernel's TCP state numbers, in hex, as `/proc/net/tcp` writes them.
fn tcp_state(hex: &str) -> &'static str {
    match hex {
        "01" => "ESTABLISHED",
        "02" => "SYN_SENT",
        "03" => "SYN_RECV",
        "04" => "FIN_WAIT1",
        "05" => "FIN_WAIT2",
        "06" => "TIME_WAIT",
        "07" => "CLOSE",
        "08" => "CLOSE_WAIT",
        "09" => "LAST_ACK",
        "0A" => "LISTEN",
        "0B" => "CLOSING",
        _ => "-",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    /// A fake `/proc` with one process holding the fds named.
    fn fixture(pid: u32, fds: &[(&str, &str)], net: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let fd_dir = dir.path().join(pid.to_string()).join("fd");
        fs::create_dir_all(&fd_dir).unwrap();
        for (n, target) in fds {
            // `read_link` reports the target verbatim, so a symlink to a path
            // that does not exist is exactly what a live fd to a deleted file
            // looks like.
            symlink(target, fd_dir.join(n)).unwrap();
        }
        let net_dir = dir.path().join("net");
        fs::create_dir_all(&net_dir).unwrap();
        for (name, body) in net {
            fs::write(net_dir.join(name), body).unwrap();
        }
        dir
    }

    const TCP_HEADER: &str =
        "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n";

    #[test]
    fn a_process_reports_the_files_it_holds() {
        let d = fixture(42, &[("0", "/home/tim/notes.md"), ("1", "/etc/hosts")], &[]);
        let h = held_resources(d.path(), 42);
        assert_eq!(h.open_files.unwrap(), vec!["/etc/hosts", "/home/tim/notes.md"]);
        assert!(h.unreadable.is_none());
    }

    #[test]
    fn pipes_and_anon_inodes_are_not_files_the_user_opened() {
        let d = fixture(
            42,
            &[("0", "pipe:[12345]"), ("1", "anon_inode:[eventfd]"), ("2", "/etc/hosts")],
            &[],
        );
        assert_eq!(held_resources(d.path(), 42).open_files.unwrap(), vec!["/etc/hosts"]);
    }

    #[test]
    fn a_process_that_is_not_ours_is_unmeasured_and_not_empty() {
        // The load-bearing case. An empty list would say "holds nothing open",
        // which on a screen about what programs can reach is a false all-clear.
        let d = tempfile::tempdir().unwrap();
        let h = held_resources(d.path(), 999);
        assert!(h.open_files.is_none());
        assert!(h.camera.is_none() && h.mic.is_none());
        assert!(h.unreadable.unwrap().starts_with("not measured"));
    }

    #[test]
    fn an_open_camera_is_seen_and_a_media_node_is_not() {
        let d = fixture(7, &[("0", "/dev/video0")], &[]);
        assert_eq!(held_resources(d.path(), 7).camera, Some(true));
        // `/dev/media0` is the media controller, not a capture device; counting
        // it would put a camera warning on anything that merely enumerated one.
        let d = fixture(7, &[("0", "/dev/media0")], &[]);
        assert_eq!(held_resources(d.path(), 7).camera, Some(false));
    }

    #[test]
    fn playback_is_not_a_microphone() {
        // `…D0p` is playback and `…D0c` is capture. Matching the directory would
        // report a microphone for every process making a sound.
        let d = fixture(7, &[("0", "/dev/snd/pcmC0D0p")], &[]);
        assert_eq!(held_resources(d.path(), 7).mic, Some(false));
        let d = fixture(7, &[("0", "/dev/snd/pcmC0D0c")], &[]);
        assert_eq!(held_resources(d.path(), 7).mic, Some(true));
    }

    #[test]
    fn a_socket_fd_resolves_through_the_kernel_table() {
        let net = format!(
            "{TCP_HEADER}   0: 0100007F:1F90 0200007F:C001 01 00000000:00000000 00:00000000 00000000  1000        0 55501 1"
        );
        let d = fixture(7, &[("0", "socket:[55501]")], &[("tcp", &net)]);
        let h = held_resources(d.path(), 7);
        let c = &h.connections.unwrap()[0];
        assert_eq!(c.local, "127.0.0.1:8080");
        assert_eq!(c.peer, "127.0.0.2:49153");
        assert_eq!(c.state, "ESTABLISHED");
        // ...and a socket is not also listed as a file.
        assert!(h.open_files.unwrap().is_empty());
    }

    #[test]
    fn the_address_is_decoded_little_endian() {
        // Read left to right, `0100007F` gives `1.0.0.127` - a plausible address
        // that is wrong, which is worse than no address at all.
        assert_eq!(decode_endpoint("0100007F:0050").unwrap(), "127.0.0.1:80");
    }

    #[test]
    fn a_listener_has_no_peer_rather_than_a_zero_one() {
        let net = format!(
            "{TCP_HEADER}   0: 00000000:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 900 1"
        );
        let d = fixture(7, &[("0", "socket:[900]")], &[("tcp", &net)]);
        let c = &held_resources(d.path(), 7).connections.unwrap()[0];
        assert_eq!(c.state, "LISTEN");
        assert_eq!(c.peer, "*");
        assert_eq!(c.local, "0.0.0.0:8080");
    }

    #[test]
    fn a_udp_socket_does_not_borrow_a_tcp_state_word() {
        // The `st` column of a UDP row is a socket state, not a connection one;
        // printing "ESTABLISHED" beside a datagram socket would describe a
        // connection that does not exist.
        let net = format!(
            "{TCP_HEADER}   0: 0100007F:14E9 00000000:0000 07 00000000:00000000 00:00000000 00000000  1000        0 777 2"
        );
        let d = fixture(7, &[("0", "socket:[777]")], &[("udp", &net)]);
        let c = &held_resources(d.path(), 7).connections.unwrap()[0];
        assert_eq!(c.proto, "udp");
        assert_eq!(c.state, "-");
    }

    #[test]
    fn a_socket_the_tables_do_not_hold_is_dropped_not_invented() {
        // Unix-domain sockets live in `/proc/net/unix`, which this does not read.
        // The right answer is to omit them, never to guess an address.
        let d = fixture(7, &[("0", "socket:[4242]")], &[("tcp", TCP_HEADER)]);
        assert!(held_resources(d.path(), 7).connections.unwrap().is_empty());
    }

    #[test]
    fn an_ipv6_endpoint_decodes_in_word_order() {
        let net = format!(
            "{TCP_HEADER}   0: 00000000000000000000000001000000:0050 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 31 1"
        );
        let d = fixture(7, &[("0", "socket:[31]")], &[("tcp6", &net)]);
        let c = &held_resources(d.path(), 7).connections.unwrap()[0];
        assert_eq!(c.proto, "tcp6");
        assert_eq!(c.local, "[0000:0000:0000:0000:0000:0000:0000:0001]:80");
    }

    #[test]
    fn the_list_is_capped_so_a_browser_cannot_stall_the_pane() {
        let targets: Vec<(String, String)> =
            (0..MAX_LISTED + 50).map(|i| (i.to_string(), format!("/tmp/f{i:04}"))).collect();
        let refs: Vec<(&str, &str)> =
            targets.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let d = fixture(7, &refs, &[]);
        assert_eq!(held_resources(d.path(), 7).open_files.unwrap().len(), MAX_LISTED);
    }
}
