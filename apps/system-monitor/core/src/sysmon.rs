//! System-wide device counters for the Performance tab: CPU, memory, disk and
//! network, read from `/proc` and `/sys` and turned into rates.
//!
//! The tab used to draw a random walk. It looked like a task manager and was one
//! only in the sense that a photograph of a thermometer is one - the numbers moved,
//! and nothing they moved with was on this machine. A task manager must be a task
//! manager, so these are the real counters, sampled the way every other tool on
//! Linux samples them.
//!
//! Three of the four are RATES, which means two samples and the time between them.
//! [`SystemMonitor`] holds the previous sample; the first call after start reports
//! zero for the rates (there is nothing to subtract from) and real memory
//! immediately, exactly like the process list. Memory is a level, not a rate, so it
//! is right from the first call.
//!
//! Everything reads through a root path so the parsing is unit-tested against a
//! fixture `/proc` in CI, where the real one belongs to the runner rather than to
//! the test.
//!
//! What this deliberately does NOT do:
//!
//!   * per-process network, which is not in `/proc` at all (it needs eBPF or cgroup
//!     attribution) and is reported as zero by the process list rather than guessed
//!   * GPU, temperatures and fan speeds, which live in a different tree again
//!   * the AI device (tokens/sec, context), which is the engine's own figure and
//!     not a kernel counter - it comes from the engine or it does not appear

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

use serde::Serialize;

/// A raw counter snapshot: the numbers as the kernel reports them, before any
/// rate is derived.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Counters {
    /// Jiffies spent doing anything other than idling.
    pub cpu_busy: u64,
    /// All jiffies, idle included.
    pub cpu_total: u64,
    /// `MemTotal` in kibibytes.
    pub mem_total_kb: u64,
    /// `MemAvailable` in kibibytes - what a new allocation can actually get,
    /// which is the honest figure. `MemFree` reads far lower because the page
    /// cache holds the rest, and a monitor that showed it would report a machine
    /// as full while it is fine.
    pub mem_available_kb: u64,
    /// Sectors read across the physical block devices.
    pub disk_read_sectors: u64,
    /// Sectors written across the physical block devices.
    pub disk_write_sectors: u64,
    /// Bytes received across the real interfaces.
    pub net_rx_bytes: u64,
    /// Bytes sent across the real interfaces.
    pub net_tx_bytes: u64,
}

/// One tick of the Performance tab: levels and rates, as numbers. Formatting and
/// wording belong to the frontend, which has the catalogue; a backend that
/// returned `"8 cores, 16 threads"` would be shipping untranslatable English.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemTick {
    /// Share of total CPU capacity used since the previous sample, 0 to 100.
    pub cpu_pct: f64,
    /// Logical CPUs, so the detail line can say how many there are.
    pub cpu_count: usize,
    /// Memory in use (total minus available) as a percentage.
    pub mem_pct: f64,
    /// Memory in use, in gibibytes.
    pub mem_used_gb: f64,
    /// Installed memory, in gibibytes.
    pub mem_total_gb: f64,
    /// Disk read rate in mebibytes per second.
    pub disk_read_mbs: f64,
    /// Disk write rate in mebibytes per second.
    pub disk_write_mbs: f64,
    /// Network receive rate in mebibytes per second.
    pub net_rx_mbs: f64,
    /// Network send rate in mebibytes per second.
    pub net_tx_mbs: f64,
    /// False on the first tick after start, when the rates have nothing to delta
    /// against and are reported as zero. The surface uses this to avoid drawing a
    /// zero it would otherwise present as a measurement.
    pub rates_ready: bool,
}

/// A `/proc` and `/sys` reader, rooted so tests can point it at a fixture.
#[derive(Debug, Clone)]
pub struct SysProbe {
    proc_root: PathBuf,
    sys_root: PathBuf,
}

impl Default for SysProbe {
    fn default() -> Self {
        Self::new()
    }
}

impl SysProbe {
    /// A probe over the real `/proc` and `/sys`.
    pub fn new() -> Self {
        Self { proc_root: PathBuf::from("/proc"), sys_root: PathBuf::from("/sys") }
    }

    /// A probe over fixture trees.
    pub fn with_roots(proc_root: impl Into<PathBuf>, sys_root: impl Into<PathBuf>) -> Self {
        Self { proc_root: proc_root.into(), sys_root: sys_root.into() }
    }

    /// Read every counter. A file that cannot be read leaves its counters at zero
    /// rather than failing the whole tick: a machine with no `diskstats` still has
    /// a CPU worth showing.
    pub fn read(&self) -> Counters {
        let stat = std::fs::read_to_string(self.proc_root.join("stat")).unwrap_or_default();
        let (cpu_busy, cpu_total) = parse_cpu(&stat).unwrap_or((0, 0));
        let meminfo = std::fs::read_to_string(self.proc_root.join("meminfo")).unwrap_or_default();
        let (mem_total_kb, mem_available_kb) = parse_meminfo(&meminfo);
        let diskstats =
            std::fs::read_to_string(self.proc_root.join("diskstats")).unwrap_or_default();
        let (disk_read_sectors, disk_write_sectors) =
            parse_diskstats(&diskstats, &|name| self.is_whole_disk(name));
        let netdev = std::fs::read_to_string(self.proc_root.join("net/dev")).unwrap_or_default();
        let (net_rx_bytes, net_tx_bytes) = parse_netdev(&netdev);
        Counters {
            cpu_busy,
            cpu_total,
            mem_total_kb,
            mem_available_kb,
            disk_read_sectors,
            disk_write_sectors,
            net_rx_bytes,
            net_tx_bytes,
        }
    }

    /// Logical CPU count, from the `cpuN` lines of `/proc/stat`. Falls back to 1 so
    /// a per-core division can never divide by zero.
    pub fn cpu_count(&self) -> usize {
        let stat = std::fs::read_to_string(self.proc_root.join("stat")).unwrap_or_default();
        let n = stat.lines().filter(|l| l.starts_with("cpu") && !l.starts_with("cpu ")).count();
        n.max(1)
    }

    /// Whether a `diskstats` name is a whole disk rather than one of its
    /// partitions. `/sys/block/<name>` exists only for whole disks, which is how
    /// the kernel itself draws the line - counting both would double every byte,
    /// and stripping trailing digits gets `nvme0n1` wrong.
    fn is_whole_disk(&self, name: &str) -> bool {
        if name.starts_with("loop") || name.starts_with("ram") || name.starts_with("zram") {
            return false;
        }
        self.sys_root.join("block").join(name).exists()
    }
}

/// Busy and total jiffies from the aggregate `cpu` line of `/proc/stat`.
///
/// Idle is `idle + iowait`: a process blocked on disk is not using the CPU, and
/// counting iowait as busy is the classic way to report 100% on an idle machine.
fn parse_cpu(stat: &str) -> Option<(u64, u64)> {
    let line = stat.lines().find(|l| l.starts_with("cpu "))?;
    let v: Vec<u64> = line.split_whitespace().skip(1).filter_map(|f| f.parse().ok()).collect();
    if v.len() < 5 {
        return None;
    }
    let total: u64 = v.iter().sum();
    let idle = v[3] + v[4];
    Some((total.saturating_sub(idle), total))
}

/// `MemTotal` and `MemAvailable` in kibibytes.
fn parse_meminfo(text: &str) -> (u64, u64) {
    let field = |key: &str| -> u64 {
        text.lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    };
    (field("MemTotal:"), field("MemAvailable:"))
}

/// Sectors read and written, summed over the whole disks `keep` accepts.
///
/// `/proc/diskstats` is `major minor name reads merges sectors_read ms ...`, so the
/// sector counts are fields 5 and 9 counting from zero. A sector is 512 bytes here
/// regardless of the device's own block size - that is the unit the kernel reports
/// in, not a hardware property.
fn parse_diskstats(text: &str, keep: &dyn Fn(&str) -> bool) -> (u64, u64) {
    let mut read = 0u64;
    let mut written = 0u64;
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 10 || !keep(f[2]) {
            continue;
        }
        read += f[5].parse::<u64>().unwrap_or(0);
        written += f[9].parse::<u64>().unwrap_or(0);
    }
    (read, written)
}

/// Bytes received and sent, summed over the real interfaces.
///
/// Loopback is excluded: traffic a machine sends to itself is not network use, and
/// including it makes any local IPC look like bandwidth.
fn parse_netdev(text: &str) -> (u64, u64) {
    let mut rx = 0u64;
    let mut tx = 0u64;
    for line in text.lines() {
        let Some((name, rest)) = line.split_once(':') else { continue };
        let name = name.trim();
        if name.is_empty() || name == "lo" {
            continue;
        }
        let f: Vec<&str> = rest.split_whitespace().collect();
        if f.len() < 9 {
            continue;
        }
        rx += f[0].parse::<u64>().unwrap_or(0);
        tx += f[8].parse::<u64>().unwrap_or(0);
    }
    (rx, tx)
}

/// The Performance tab's sampler: holds the previous counters so each call can
/// report rates.
pub struct SystemMonitor {
    probe: SysProbe,
    previous: Mutex<Option<(Counters, Instant)>>,
}

impl Default for SystemMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemMonitor {
    /// A monitor over the real `/proc` and `/sys`.
    pub fn new() -> Self {
        Self::with_probe(SysProbe::new())
    }

    /// A monitor over a given probe.
    pub fn with_probe(probe: SysProbe) -> Self {
        Self { probe, previous: Mutex::new(None) }
    }

    /// Sample once. Rates are computed against the previous call; the first call
    /// reports them as zero with `rates_ready` false.
    pub fn sample(&self) -> SystemTick {
        self.sample_at(Instant::now())
    }

    /// Sample with an explicit clock reading, so a test can space two samples by a
    /// known interval instead of sleeping.
    pub fn sample_at(&self, now: Instant) -> SystemTick {
        let now_counters = self.probe.read();
        let mut previous = self.previous.lock().unwrap_or_else(|e| e.into_inner());
        let last = previous.replace((now_counters, now));
        drop(previous);

        let mem_total_gb = kib_to_gib(now_counters.mem_total_kb);
        let mem_used_kb = now_counters.mem_total_kb.saturating_sub(now_counters.mem_available_kb);
        let mem_pct = if now_counters.mem_total_kb > 0 {
            mem_used_kb as f64 / now_counters.mem_total_kb as f64 * 100.0
        } else {
            0.0
        };

        let Some((prev, prev_at)) = last else {
            return SystemTick {
                cpu_pct: 0.0,
                cpu_count: self.probe.cpu_count(),
                mem_pct,
                mem_used_gb: kib_to_gib(mem_used_kb),
                mem_total_gb,
                disk_read_mbs: 0.0,
                disk_write_mbs: 0.0,
                net_rx_mbs: 0.0,
                net_tx_mbs: 0.0,
                rates_ready: false,
            };
        };

        let secs = now.saturating_duration_since(prev_at).as_secs_f64();
        // A counter that went backwards means the machine's bookkeeping was reset
        // under us (a device removed, a namespace change, a suspend). Reporting a
        // huge negative-turned-positive spike would be worse than reporting nothing,
        // so `saturating_sub` floors it at zero and the next tick recovers.
        let d_total = now_counters.cpu_total.saturating_sub(prev.cpu_total);
        let d_busy = now_counters.cpu_busy.saturating_sub(prev.cpu_busy);
        let cpu_pct = if d_total > 0 { d_busy as f64 / d_total as f64 * 100.0 } else { 0.0 };

        let rate = |now_v: u64, prev_v: u64, unit: f64| -> f64 {
            if secs <= 0.0 {
                return 0.0;
            }
            now_v.saturating_sub(prev_v) as f64 * unit / secs
        };
        const SECTOR: f64 = 512.0 / (1024.0 * 1024.0);
        const BYTE: f64 = 1.0 / (1024.0 * 1024.0);

        SystemTick {
            cpu_pct,
            cpu_count: self.probe.cpu_count(),
            mem_pct,
            mem_used_gb: kib_to_gib(mem_used_kb),
            mem_total_gb,
            disk_read_mbs: rate(now_counters.disk_read_sectors, prev.disk_read_sectors, SECTOR),
            disk_write_mbs: rate(now_counters.disk_write_sectors, prev.disk_write_sectors, SECTOR),
            net_rx_mbs: rate(now_counters.net_rx_bytes, prev.net_rx_bytes, BYTE),
            net_tx_mbs: rate(now_counters.net_tx_bytes, prev.net_tx_bytes, BYTE),
            rates_ready: true,
        }
    }
}

fn kib_to_gib(kib: u64) -> f64 {
    kib as f64 / (1024.0 * 1024.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::time::Duration;

    const STAT: &str = "cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 50 0 25 400 25 0 0 0 0 0\ncpu1 50 0 25 400 25 0 0 0 0 0\nintr 1\n";
    const MEMINFO: &str = "MemTotal:       16384000 kB\nMemFree:         1000000 kB\nMemAvailable:    8192000 kB\nBuffers:          100000 kB\n";
    // major minor name reads merges sectors_read ms writes merges sectors_written
    const DISKSTATS: &str = concat!(
        "   8       0 sda 100 0 2048 10 50 0 1024 5 0 0 0\n",
        "   8       1 sda1 90 0 2000 10 40 0 1000 5 0 0 0\n",
        "   7       0 loop0 1 0 8 0 0 0 0 0 0 0 0\n",
    );
    const NETDEV: &str = concat!(
        "Inter-|   Receive                                                |  Transmit\n",
        " face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets\n",
        "    lo: 5000 10 0 0 0 0 0 0 5000 10 0 0 0 0 0 0\n",
        "  eth0: 1048576 100 0 0 0 0 0 0 524288 50 0 0 0 0 0 0\n",
    );

    fn fixture(dir: &Path, whole_disks: &[&str]) -> SysProbe {
        std::fs::create_dir_all(dir.join("proc/net")).unwrap();
        std::fs::write(dir.join("proc/stat"), STAT).unwrap();
        std::fs::write(dir.join("proc/meminfo"), MEMINFO).unwrap();
        std::fs::write(dir.join("proc/diskstats"), DISKSTATS).unwrap();
        std::fs::write(dir.join("proc/net/dev"), NETDEV).unwrap();
        for d in whole_disks {
            std::fs::create_dir_all(dir.join("sys/block").join(d)).unwrap();
        }
        SysProbe::with_roots(dir.join("proc"), dir.join("sys"))
    }

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("arlen-sysmon-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn iowait_is_idle_not_busy() {
        // 1000 jiffies total, 800 idle + 50 iowait: 150 busy, not 200. A monitor
        // that counted iowait as work would show 20% on a machine doing nothing but
        // waiting for a disk.
        assert_eq!(parse_cpu(STAT), Some((150, 1000)));
    }

    #[test]
    fn a_partition_is_not_counted_beside_its_disk() {
        // sda and sda1 both report the same reads. Counting both doubles every byte.
        let (r, w) = parse_diskstats(DISKSTATS, &|n| n == "sda");
        assert_eq!((r, w), (2048, 1024));
        let (r_all, _) = parse_diskstats(DISKSTATS, &|n| n == "sda" || n == "sda1");
        assert_eq!(r_all, 4048, "the double-counting this filter exists to prevent");
    }

    #[test]
    fn loopback_is_not_network_traffic() {
        let (rx, tx) = parse_netdev(NETDEV);
        assert_eq!((rx, tx), (1048576, 524288), "lo's 5000 bytes must not appear");
    }

    #[test]
    fn the_first_sample_reports_memory_but_not_rates() {
        let dir = tmp("first");
        let probe = fixture(&dir, &["sda"]);
        let m = SystemMonitor::with_probe(probe);
        let t = m.sample();
        assert!(!t.rates_ready);
        assert_eq!(t.cpu_pct, 0.0);
        assert_eq!(t.disk_read_mbs, 0.0);
        // Memory is a level, so it is real immediately: 16000 MiB total, 8000
        // available, half used.
        assert!((t.mem_pct - 50.0).abs() < 0.01, "{}", t.mem_pct);
        assert!((t.mem_total_gb - 15.625).abs() < 0.01, "{}", t.mem_total_gb);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_second_sample_turns_counters_into_rates() {
        let dir = tmp("second");
        let probe = fixture(&dir, &["sda"]);
        let m = SystemMonitor::with_probe(probe);
        let t0 = Instant::now();
        m.sample_at(t0);

        // Advance the counters by a known amount over a known second.
        std::fs::write(
            dir.join("proc/stat"),
            "cpu  200 0 100 1600 100 0 0 0 0 0\ncpu0 1 0 1 1 1 0 0 0 0 0\ncpu1 1 0 1 1 1 0 0 0 0 0\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("proc/diskstats"),
            "   8       0 sda 100 0 4096 10 50 0 3072 5 0 0 0\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("proc/net/dev"),
            "  eth0: 2097152 100 0 0 0 0 0 0 524288 50 0 0 0 0 0 0\n",
        )
        .unwrap();

        let t = m.sample_at(t0 + Duration::from_secs(1));
        assert!(t.rates_ready);
        // busy 150 -> 300 (+150), total 1000 -> 2000 (+1000): 15%.
        assert!((t.cpu_pct - 15.0).abs() < 0.01, "{}", t.cpu_pct);
        // +2048 sectors read in a second = 1 MiB/s; +2048 written likewise.
        assert!((t.disk_read_mbs - 1.0).abs() < 0.01, "{}", t.disk_read_mbs);
        assert!((t.disk_write_mbs - 1.0).abs() < 0.01, "{}", t.disk_write_mbs);
        // +1 MiB received, nothing sent.
        assert!((t.net_rx_mbs - 1.0).abs() < 0.01, "{}", t.net_rx_mbs);
        assert_eq!(t.net_tx_mbs, 0.0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_counter_that_went_backwards_reports_zero_rather_than_a_spike() {
        let dir = tmp("reset");
        let probe = fixture(&dir, &["sda"]);
        let m = SystemMonitor::with_probe(probe);
        let t0 = Instant::now();
        m.sample_at(t0);
        std::fs::write(dir.join("proc/net/dev"), "  eth0: 1 1 0 0 0 0 0 0 1 1 0 0 0 0 0 0\n")
            .unwrap();
        let t = m.sample_at(t0 + Duration::from_secs(1));
        assert_eq!(t.net_rx_mbs, 0.0, "a device reset must not read as a burst");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_unreadable_proc_reports_zero_rather_than_failing_the_tick() {
        let m = SystemMonitor::with_probe(SysProbe::with_roots("/nonexistent", "/nonexistent"));
        let t = m.sample();
        assert_eq!(t.mem_total_gb, 0.0);
        assert_eq!(t.cpu_count, 1, "never divide by zero cores");
    }
}
