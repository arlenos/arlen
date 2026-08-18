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
///
/// Clone rather than Copy since the per-core vector arrived: the alternative was
/// keeping the cores in a second field of the sampler, which would make one read
/// of `/proc/stat` the source of two states that could disagree.
#[derive(Debug, Clone, PartialEq)]
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
    /// Memory pressure, or `None` where the kernel exposes no PSI file. Carried
    /// as-read rather than as a number, so the absence survives to the surface.
    pub mem_pressure: Option<MemoryPressure>,
    /// Per-logical-core jiffies, in `cpuN` order, for the grid.
    pub cores: Vec<CoreTimes>,
    /// Load average, or `None` where `/proc/loadavg` could not be read.
    pub load: Option<LoadAverage>,
    /// Per-whole-disk sector counters, for the device breakdown.
    pub devices: Vec<DeviceCounters>,
    /// Per-physical-interface byte counters.
    pub links: Vec<LinkCounters>,
    /// CPU package temperature in Celsius, or `None` where no CPU sensor exists.
    pub cpu_temp_c: Option<CpuTemp>,
    /// Per-core clock in MHz, empty where the machine exposes no `cpufreq`.
    pub core_freqs: Vec<Option<f64>>,
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
#[derive(Debug, Clone, PartialEq, Serialize)]
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
    /// Memory pressure, or `None` where the kernel exposes no PSI. `None` is
    /// "not measured" and must render as such: a green meter over an absent
    /// reading is the surface inventing a verdict.
    pub mem_pressure: Option<MemoryPressure>,
    /// Per-core shares since the previous sample, in `cpuN` order. Empty on the
    /// first tick, when there is nothing to difference against.
    pub cores: Vec<CoreUsage>,
    /// Load average, or `None` where it could not be read. A load of 0.00 is a
    /// real reading, so absence is carried rather than defaulted.
    pub load: Option<LoadAverage>,
    /// Per-whole-disk rates. Empty on the first tick, and empty for any device
    /// that was not present in the previous sample.
    pub devices: Vec<DeviceRate>,
    /// Per-physical-interface rates, same rules as `devices`.
    pub links: Vec<LinkRate>,
    /// CPU temperature with its sensor label, `None` where unmeasured.
    pub cpu_temp_c: Option<CpuTemp>,
    /// Per-core clock in MHz, empty where unmeasured.
    pub core_freqs: Vec<Option<f64>>,
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
        // The same read, so the grid and the aggregate can never disagree about
        // which instant they describe.
        let cores = parse_cpu_cores(&stat);
        // Counted from the same `stat` read the cores came from, so the divisor
        // and the grid describe the same machine.
        let cpus = cores.len().max(1);
        let load = std::fs::read_to_string(self.proc_root.join("loadavg"))
            .ok()
            .and_then(|s| parse_loadavg(&s, cpus));
        let meminfo = std::fs::read_to_string(self.proc_root.join("meminfo")).unwrap_or_default();
        let (mem_total_kb, mem_available_kb) = parse_meminfo(&meminfo);
        let diskstats =
            std::fs::read_to_string(self.proc_root.join("diskstats")).unwrap_or_default();
        let (disk_read_sectors, disk_write_sectors) =
            parse_diskstats(&diskstats, &|name| self.is_whole_disk(name));
        // The same text and the same whole-disk rule as the totals, so the
        // breakdown can never disagree with the figure above it.
        let devices = parse_diskstats_devices(&diskstats, &|name| self.is_whole_disk(name));
        let netdev = std::fs::read_to_string(self.proc_root.join("net/dev")).unwrap_or_default();
        let (net_rx_bytes, net_tx_bytes) = parse_netdev(&netdev, &|n| self.is_real_link(n));
        let links = parse_netdev_links(&netdev, &|n| self.is_real_link(n));
        let cpu_temp_c = read_cpu_temp(&self.sys_root);
        let core_freqs = read_core_freqs(&self.sys_root, cores.len());
        // Absent on a kernel without CONFIG_PSI. `unwrap_or_default()` like the
        // others would turn that into an empty string and then into 0.00 - a
        // green meter over a machine nobody measured - so this one keeps the
        // read fallible all the way through.
        let mem_pressure = std::fs::read_to_string(self.proc_root.join("pressure/memory"))
            .ok()
            .and_then(|s| parse_pressure(&s));
        Counters {
            cpu_busy,
            cpu_total,
            mem_total_kb,
            mem_available_kb,
            disk_read_sectors,
            disk_write_sectors,
            net_rx_bytes,
            net_tx_bytes,
            mem_pressure,
            cores,
            load,
            devices,
            links,
            cpu_temp_c,
            core_freqs,
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
        let dir = self.sys_root.join("block").join(name);
        if !dir.exists() {
            return false;
        }
        // A STACKED device is not another disk, it is the same one seen through a
        // layer. `/sys/block/dm-0/slaves/` holds `nvme0n1p2` on this machine, so
        // every byte written through the LUKS mapper is also counted under the
        // nvme - and the total read about double what the hardware actually did.
        //
        // Found by building the per-device breakdown and seeing `nvme0n1` and
        // `dm-0` side by side, which is a thing one summed figure could never
        // show. It affects any machine with LUKS, LVM or software RAID, which is
        // most of them.
        //
        // The physical device wins because it is the one with finite throughput.
        // Naming the volume a person recognises instead is a presentation
        // question and a different change.
        !dir
            .join("slaves")
            .read_dir()
            .map(|mut d| d.next().is_some())
            .unwrap_or(false)
    }

    /// Whether an interface is real hardware rather than a virtual one.
    ///
    /// `/sys/class/net/<name>/device` is a link to the backing PCI or USB device
    /// and exists only for a physical NIC. Loopback, bridges, `docker0`, tun/tap
    /// and every `veth` lack it.
    ///
    /// This matters for the same reason the disk mapper did, and was found the
    /// same way. The total summed every interface that was not `lo`, and on this
    /// machine that is `wlan0` plus three bridges and three veths - where a
    /// bridge and its veth carry the SAME packets `wlan0` already counted, so a
    /// container pulling a gigabyte made the graph read several. A physical
    /// interface is where bytes actually enter and leave the machine.
    fn is_real_link(&self, name: &str) -> bool {
        self.sys_root.join("class/net").join(name).join("device").exists()
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

/// One logical CPU's jiffy counters, split the way htop splits them.
///
/// The plan asks for "the user/system/iowait color split (htop)" rather than one
/// bar per core, and the reason is diagnostic: a core pinned at 100% user is a
/// program working, a core at 100% system is the kernel thrashing on its behalf,
/// and a core deep in iowait is not busy at all - it is waiting for a disk. One
/// number cannot tell those apart and all three are the same height.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CoreTimes {
    /// user + nice.
    pub user: u64,
    /// system + irq + softirq: time the kernel spent on this core.
    pub system: u64,
    /// iowait. NOT idle, and NOT busy either - see `busy()`.
    pub iowait: u64,
    /// idle.
    pub idle: u64,
}

impl CoreTimes {
    /// Everything the kernel accounted for on this core.
    pub fn total(&self) -> u64 {
        self.user + self.system + self.iowait + self.idle
    }

    /// The part that counts as busy.
    ///
    /// iowait is EXCLUDED, which is a judgement worth stating: Linux counts a
    /// core as iowait when it is idle and some task on it is blocked on I/O, so
    /// calling it busy would show a machine waiting on a slow disk as a machine
    /// out of CPU - the opposite diagnosis. It is reported separately so the
    /// colour can say "waiting" where a single bar would say "busy".
    pub fn busy(&self) -> u64 {
        self.user + self.system
    }
}

/// Per-core counters from `/proc/stat`, in `cpuN` order.
///
/// The `cpuN` lines are `user nice system idle iowait irq softirq steal ...`.
/// Anything shorter than the first five fields is skipped rather than
/// zero-filled: a truncated line is a read that went wrong, and a core reported
/// at 0% is indistinguishable from a core that is genuinely idle.
pub fn parse_cpu_cores(stat: &str) -> Vec<CoreTimes> {
    stat.lines()
        .filter(|l| l.starts_with("cpu") && !l.starts_with("cpu "))
        .filter_map(|l| {
            let v: Vec<u64> = l.split_whitespace().skip(1).filter_map(|f| f.parse().ok()).collect();
            if v.len() < 5 {
                return None;
            }
            Some(CoreTimes {
                user: v[0] + v[1],
                system: v[2] + v.get(5).copied().unwrap_or(0) + v.get(6).copied().unwrap_or(0),
                iowait: v[4],
                idle: v[3],
            })
        })
        .collect()
}

/// What each core did between two samples, as percentages of that core's own
/// elapsed jiffies.
///
/// Per core rather than per machine: a single core pegged on an eight-core box
/// is 12.5% of the machine and 100% of the thing that is stuck, and the grid
/// exists to show the second number.
///
/// A core whose total did not advance reports zeros rather than dividing - that
/// happens on the first sample and on a core the kernel offlined, and both are
/// "nothing measured" rather than "nothing happening". Cores that appeared or
/// vanished between samples are dropped, since there is no pair to difference.
pub fn core_percentages(now: &[CoreTimes], prev: &[CoreTimes]) -> Vec<CoreUsage> {
    now.iter()
        .zip(prev.iter())
        .map(|(n, p)| {
            let span = n.total().saturating_sub(p.total());
            if span == 0 {
                return CoreUsage::default();
            }
            let pct = |a: u64, b: u64| a.saturating_sub(b) as f64 / span as f64 * 100.0;
            CoreUsage {
                user: pct(n.user, p.user),
                system: pct(n.system, p.system),
                iowait: pct(n.iowait, p.iowait),
            }
        })
        .collect()
}

/// One core's share of its own last interval, in percent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct CoreUsage {
    pub user: f64,
    pub system: f64,
    /// Waiting on I/O, which is not the same as busy. Carried separately so the
    /// grid can colour it differently rather than adding it to the height.
    pub iowait: f64,
}

/// The three load averages, and what they mean on THIS machine.
///
/// Load is the number of tasks runnable or in uninterruptible sleep, averaged
/// over one, five and fifteen minutes. On its own it is not a percentage and not
/// comparable between machines: a load of 8 is half-idle on a sixteen-thread box
/// and badly oversubscribed on a dual-core one. So the count it is measured
/// against travels with it rather than being left for the reader to remember.
// `rename_all` is NOT inherited from the tick that carries this: serde applies
// it per struct, so a nested one without it ships `per_core` while the frontend
// reads `perCore` and the whole line silently vanishes. That is exactly what
// happened, and the screenshot probe is what said so.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadAverage {
    /// One-minute average: what is happening now.
    pub one: f64,
    /// Five-minute average.
    pub five: f64,
    /// Fifteen-minute average: whether this is a spike or the shape of the day.
    pub fifteen: f64,
    /// `one` divided by the logical CPU count - the figure that is comparable.
    /// At 1.0 the machine is exactly saturated; above it, work is queueing.
    pub per_core: f64,
}

/// Parse `/proc/loadavg`, which is `one five fifteen running/total lastpid`.
///
/// `None` rather than zeros when the line is missing or short: a load of 0.00 is
/// a real and reassuring reading, so a failed parse must not be able to produce
/// one. `cpus` is clamped to at least 1 so the division cannot blow up on a
/// machine whose core count could not be read either.
pub fn parse_loadavg(text: &str, cpus: usize) -> Option<LoadAverage> {
    let mut f = text.split_whitespace();
    let one: f64 = f.next()?.parse().ok()?;
    let five: f64 = f.next()?.parse().ok()?;
    let fifteen: f64 = f.next()?.parse().ok()?;
    Some(LoadAverage {
        one,
        five,
        fifteen,
        per_core: one / cpus.max(1) as f64,
    })
}

/// One whole disk's sector counters, kept apart from its siblings.
///
/// The plan asks for "per-device util + read/write" rather than one total,
/// because the total answers the wrong question: a machine writing hard to an
/// external drive and a machine writing hard to the disk its swap is on look
/// identical in a single figure, and only one of them is about to feel slow.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceCounters {
    /// Kernel name, `nvme0n1` or `sda`.
    pub name: String,
    pub read_sectors: u64,
    pub write_sectors: u64,
}

/// One whole disk's rates over the last interval.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRate {
    pub name: String,
    pub read_mbs: f64,
    pub write_mbs: f64,
}

/// Per-device sector counters, whole disks only, in the order the kernel lists
/// them.
///
/// Partitions are excluded by the same `keep` the totals use: counting `nvme0n1`
/// and `nvme0n1p2` both would double every byte, and a per-device list showing a
/// disk beside its own partition invites exactly that mistake by hand.
pub fn parse_diskstats_devices(text: &str, keep: &dyn Fn(&str) -> bool) -> Vec<DeviceCounters> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 10 || !keep(f[2]) {
                return None;
            }
            Some(DeviceCounters {
                name: f[2].to_string(),
                read_sectors: f[5].parse().unwrap_or(0),
                write_sectors: f[9].parse().unwrap_or(0),
            })
        })
        .collect()
}

/// Per-device rates between two samples, matched BY NAME rather than by
/// position.
///
/// Position would be wrong the moment a USB disk is plugged in or removed
/// mid-session: `/proc/diskstats` would shift under the index and every device
/// would report its neighbour's traffic. A device with no counterpart in the
/// previous sample is omitted - it has no interval to have a rate over.
pub fn device_rates(now: &[DeviceCounters], prev: &[DeviceCounters], secs: f64) -> Vec<DeviceRate> {
    if secs <= 0.0 {
        return Vec::new();
    }
    const SECTOR_MB: f64 = 512.0 / (1024.0 * 1024.0);
    now.iter()
        .filter_map(|n| {
            let p = prev.iter().find(|p| p.name == n.name)?;
            Some(DeviceRate {
                name: n.name.clone(),
                read_mbs: n.read_sectors.saturating_sub(p.read_sectors) as f64 * SECTOR_MB / secs,
                write_mbs: n.write_sectors.saturating_sub(p.write_sectors) as f64 * SECTOR_MB / secs,
            })
        })
        .collect()
}

/// One network interface's byte counters.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkCounters {
    pub name: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

/// One interface's rates over the last interval.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkRate {
    pub name: String,
    pub rx_mbs: f64,
    pub tx_mbs: f64,
}

/// Per-interface byte counters for the interfaces `keep` accepts.
pub fn parse_netdev_links(text: &str, keep: &dyn Fn(&str) -> bool) -> Vec<LinkCounters> {
    text.lines()
        .filter_map(|line| {
            let (name, rest) = line.split_once(':')?;
            let name = name.trim();
            if name.is_empty() || !keep(name) {
                return None;
            }
            let f: Vec<&str> = rest.split_whitespace().collect();
            if f.len() < 9 {
                return None;
            }
            Some(LinkCounters {
                name: name.to_string(),
                rx_bytes: f[0].parse().unwrap_or(0),
                tx_bytes: f[8].parse().unwrap_or(0),
            })
        })
        .collect()
}

/// Per-interface rates, matched by name for the same reason the disks are: an
/// interface appearing or going away shifts every later line.
pub fn link_rates(now: &[LinkCounters], prev: &[LinkCounters], secs: f64) -> Vec<LinkRate> {
    if secs <= 0.0 {
        return Vec::new();
    }
    const BYTE_MB: f64 = 1.0 / (1024.0 * 1024.0);
    now.iter()
        .filter_map(|n| {
            let p = prev.iter().find(|p| p.name == n.name)?;
            Some(LinkRate {
                name: n.name.clone(),
                rx_mbs: n.rx_bytes.saturating_sub(p.rx_bytes) as f64 * BYTE_MB / secs,
                tx_mbs: n.tx_bytes.saturating_sub(p.tx_bytes) as f64 * BYTE_MB / secs,
            })
        })
        .collect()
}

/// hwmon chip names that are a CPU temperature.
///
/// An allowlist rather than "the first sensor found", because `/sys/class/hwmon`
/// on this laptop holds eleven chips including the charger, two DIMM sensors and
/// the WiFi radio. Picking one by position would put the SSD's temperature under
/// a heading that says CPU, and a wrong number under a confident label is worse
/// than no number at all.
///
/// `k10temp` and `zenpower` are AMD, `coretemp` Intel, `cpu_thermal` the
/// ARM/Raspberry Pi driver.
const CPU_SENSORS: &[&str] = &["k10temp", "coretemp", "zenpower", "cpu_thermal"];

/// A CPU temperature reading and WHAT it is a temperature of.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuTemp {
    pub celsius: f64,
    /// The sensor's own label - `Tdie`, `Tctl`, `Package id 0`.
    ///
    /// Carried rather than dropped because on AMD it changes what the number
    /// MEANS. This laptop's `k10temp` reports only `Tctl`, which is a control
    /// value carrying a vendor offset: it reads a steady 100.1 C while the die
    /// is far cooler. Printed bare under the word "temperature" that is a
    /// machine apparently about to melt, so the label goes with the figure and
    /// the reader can tell which one they are looking at.
    pub label: String,
}

/// The CPU temperature, or `None`.
///
/// `None` covers a machine with no such sensor at all - most VMs, and any chip
/// whose driver is not loaded - and it must stay distinct from a reading: 0 C is
/// a plausible-looking number and would be a lie on every machine that has ever
/// been switched on.
///
/// Within a matching chip, a sensor labelled `Tdie` wins over any other, because
/// it is the actual silicon temperature where `Tctl` is an offset control value.
/// Where only `Tctl` exists it is reported under its own name rather than
/// silently promoted.
pub fn read_cpu_temp(sys_root: &std::path::Path) -> Option<CpuTemp> {
    let dir = sys_root.join("class/hwmon").read_dir().ok()?;
    let mut chips: Vec<_> = dir.filter_map(|e| e.ok().map(|e| e.path())).collect();
    // Sorted so a machine with two matching chips picks the same one every tick
    // rather than flickering between them as readdir order changes.
    chips.sort();
    for chip in chips {
        let Ok(name) = std::fs::read_to_string(chip.join("name")) else { continue };
        if !CPU_SENSORS.contains(&name.trim()) {
            continue;
        }
        let mut found: Vec<CpuTemp> = Vec::new();
        for i in 1..=8 {
            let Ok(raw) = std::fs::read_to_string(chip.join(format!("temp{i}_input"))) else {
                continue;
            };
            let Ok(milli) = raw.trim().parse::<f64>() else { continue };
            let label = std::fs::read_to_string(chip.join(format!("temp{i}_label")))
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|_| format!("temp{i}"));
            found.push(CpuTemp { celsius: milli / 1000.0, label });
        }
        if let Some(die) = found.iter().find(|t| t.label.eq_ignore_ascii_case("Tdie")) {
            return Some(die.clone());
        }
        if let Some(first) = found.into_iter().next() {
            return Some(first);
        }
    }
    None
}

/// Current clock of each logical core, in megahertz, in `cpuN` order.
///
/// Empty where the machine exposes no `cpufreq` - a VM, or a driver that is not
/// loaded - rather than a list of zeros, for the same reason as the temperature.
/// `count` comes from the caller so the list lines up with the core grid even if
/// a core's `cpufreq` directory is missing: that core reports `None` in place.
pub fn read_core_freqs(sys_root: &std::path::Path, count: usize) -> Vec<Option<f64>> {
    let mut out = Vec::with_capacity(count);
    let mut any = false;
    for i in 0..count {
        let path = sys_root
            .join("devices/system/cpu")
            .join(format!("cpu{i}"))
            .join("cpufreq/scaling_cur_freq");
        // Kilohertz in the file; megahertz is what a person reads.
        let mhz = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| s.trim().parse::<f64>().ok())
            .map(|khz| khz / 1000.0);
        any |= mhz.is_some();
        out.push(mhz);
    }
    if any { out } else { Vec::new() }
}

/// The memory-pressure meter the plan asks for, backed by real PSI.
///
/// `/proc/pressure/memory` is the kernel telling you how much time tasks spent
/// STALLED waiting on memory, which is a different question from "how full is
/// it". A machine can sit at 95% used and be perfectly happy, and it can thrash
/// itself to a standstill at 60% if the working set does not fit - the plan
/// calls this "the single best 'is it actually thrashing' signal" for exactly
/// that reason, and a percentage-full bar cannot answer it.
///
/// Two lines matter and they mean different things:
///
///   some   at least one task was stalled. Some of this is normal on any busy
///          machine, so it is the early warning and not the alarm.
///   full   EVERY non-idle task was stalled - nothing could make progress. This
///          is the number that says thrashing rather than busy.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct MemoryPressure {
    /// `some avg10` as a percentage of the last ten seconds.
    pub some10: f64,
    /// `full avg10` as a percentage of the last ten seconds.
    pub full10: f64,
    /// `"ok" | "warn" | "critical"`, the meter's three states.
    pub level: &'static str,
}

/// Where PSI is not available, and why that is NOT "ok".
///
/// A kernel built without `CONFIG_PSI`, or one where it is off, has no file to
/// read - and a container may be showing the host's figures or nothing at all.
/// Returning green there would be the surface stating something nobody measured,
/// so the absence is `None` and the caller has to render it as unavailable.
pub fn parse_pressure(text: &str) -> Option<MemoryPressure> {
    let avg10 = |prefix: &str| -> Option<f64> {
        text.lines()
            .find(|l| l.starts_with(prefix))?
            .split_whitespace()
            .find_map(|f| f.strip_prefix("avg10=")?.parse().ok())
    };
    let some10 = avg10("some")?;
    let full10 = avg10("full")?;
    Some(MemoryPressure {
        some10,
        full10,
        level: pressure_level(some10, full10),
    })
}

/// The three states, from thresholds chosen with reasons rather than measured.
///
/// Said plainly because it matters for how much weight the colour carries: these
/// numbers are a judgement, not a finding. `full > 0` at all means something got
/// nothing done waiting for memory, which is worth a colour; 10% of the last ten
/// seconds fully stalled is a machine in trouble by any reading. `some` alone is
/// ordinary on a busy machine, so it takes a much higher bar to say anything.
fn pressure_level(some10: f64, full10: f64) -> &'static str {
    if full10 >= 10.0 {
        "critical"
    } else if full10 > 0.0 || some10 >= 20.0 {
        "warn"
    } else {
        "ok"
    }
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
fn parse_netdev(text: &str, keep: &dyn Fn(&str) -> bool) -> (u64, u64) {
    let mut rx = 0u64;
    let mut tx = 0u64;
    for line in text.lines() {
        let Some((name, rest)) = line.split_once(':') else { continue };
        let name = name.trim();
        // The same predicate the breakdown uses, so the two can never disagree
        // about which interfaces this machine has.
        if name.is_empty() || !keep(name) {
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
        // Cloned rather than moved: `Counters` carries the per-core vector now,
        // so the stored copy and the one this tick reads from are separate.
        let last = previous.replace((now_counters.clone(), now));
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
                mem_pressure: now_counters.mem_pressure,
                cores: Vec::new(),
                load: now_counters.load,
                devices: Vec::new(),
                links: Vec::new(),
                cpu_temp_c: now_counters.cpu_temp_c.clone(),
                core_freqs: now_counters.core_freqs.clone(),
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
            mem_pressure: now_counters.mem_pressure,
            cores: core_percentages(&now_counters.cores, &prev.cores),
            load: now_counters.load,
            devices: device_rates(&now_counters.devices, &prev.devices, secs),
            links: link_rates(&now_counters.links, &prev.links, secs),
            cpu_temp_c: now_counters.cpu_temp_c.clone(),
            core_freqs: now_counters.core_freqs.clone(),
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
        // `eth0` is the fixture's NIC, so it needs the `device` link a real one
        // has - the same way each whole disk needs its `/sys/block` entry. Added
        // when the interface rule landed: without it every fixture's network
        // counters read zero, which looked like a broken rate rather than a
        // fixture that had not kept up.
        std::fs::create_dir_all(dir.join("sys/class/net/eth0/device")).unwrap();
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
    fn only_real_interfaces_count_as_network_traffic() {
        // `eth0` is the hardware; `lo`, a bridge and a veth are not. The bridge
        // and veth carry packets the NIC already counted, so summing them
        // multiplies a container's download into the graph several times over.
        let text = concat!(
            "    lo: 5000 10 0 0 0 0 0 0 5000 10 0 0 0 0 0 0\n",
            "  eth0: 1048576 100 0 0 0 0 0 0 524288 50 0 0 0 0 0 0\n",
            "docker0: 900000 90 0 0 0 0 0 0 400000 40 0 0 0 0 0 0\n",
            "veth42: 900000 90 0 0 0 0 0 0 400000 40 0 0 0 0 0 0\n",
        );
        let (rx, tx) = parse_netdev(text, &|n| n == "eth0");
        assert_eq!((rx, tx), (1048576, 524288), "the NIC's bytes, once");
    }

    #[test]
    fn a_virtual_interface_is_not_real_hardware() {
        let dir = tmp("links");
        let probe = fixture(&dir, &["sda"]);
        // `/sys/class/net/<name>/device` exists only for a physical NIC.
        std::fs::create_dir_all(dir.join("sys/class/net/eth0/device")).unwrap();
        std::fs::create_dir_all(dir.join("sys/class/net/docker0")).unwrap();
        std::fs::write(
            dir.join("proc/net/dev"),
            "  eth0: 100 1 0 0 0 0 0 0 200 1 0 0 0 0 0 0\n             docker0: 999 1 0 0 0 0 0 0 999 1 0 0 0 0 0 0\n",
        )
        .unwrap();
        let c = probe.read();
        assert_eq!(c.links.len(), 1);
        assert_eq!(c.links[0].name, "eth0");
        assert_eq!(c.net_rx_bytes, 100, "and the total agrees with the breakdown");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn interface_rates_are_matched_by_name() {
        let prev = vec![LinkCounters { name: "eth0".into(), rx_bytes: 0, tx_bytes: 0 }];
        let now = vec![
            LinkCounters { name: "wlan0".into(), rx_bytes: 9_999_999, tx_bytes: 0 },
            LinkCounters { name: "eth0".into(), rx_bytes: 1_048_576, tx_bytes: 0 },
        ];
        let r = link_rates(&now, &prev, 1.0);
        assert_eq!(r.len(), 1, "the newcomer has no interval yet");
        assert_eq!(r[0].name, "eth0");
        assert_eq!(r[0].rx_mbs, 1.0);
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

    /// This machine's own format, copied off `/proc/pressure/memory`.
    const PRESSURE: &str = "some avg10=0.00 avg60=0.02 avg300=0.00 total=159475849\nfull avg10=0.00 avg60=0.02 avg300=0.00 total=146254960\n";

    #[test]
    fn a_quiet_machine_reads_ok() {
        let p = parse_pressure(PRESSURE).expect("the real format parses");
        assert_eq!(p.some10, 0.0);
        assert_eq!(p.full10, 0.0);
        assert_eq!(p.level, "ok");
    }

    /// The number that means thrashing rather than busy: every task stalled.
    #[test]
    fn any_full_stall_at_all_is_worth_a_colour() {
        let text = "some avg10=3.00 avg60=0.00 avg300=0.00 total=1\nfull avg10=0.40 avg60=0.00 avg300=0.00 total=1\n";
        assert_eq!(parse_pressure(text).unwrap().level, "warn");
    }

    #[test]
    fn a_tenth_of_the_window_fully_stalled_is_critical() {
        let text = "some avg10=60.00 avg60=0.00 avg300=0.00 total=1\nfull avg10=12.50 avg60=0.00 avg300=0.00 total=1\n";
        assert_eq!(parse_pressure(text).unwrap().level, "critical");
    }

    /// `some` alone is ordinary on a busy machine - this box sits around 30 on
    /// the CPU file - so it takes a much higher bar before it says anything.
    #[test]
    fn some_pressure_alone_stays_quiet_until_it_is_high() {
        let mild = "some avg10=8.00 avg60=0.00 avg300=0.00 total=1\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=1\n";
        assert_eq!(parse_pressure(mild).unwrap().level, "ok");
        let heavy = "some avg10=25.00 avg60=0.00 avg300=0.00 total=1\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=1\n";
        assert_eq!(parse_pressure(heavy).unwrap().level, "warn");
    }

    /// A kernel with no PSI is NOT a healthy one. The absence has to survive.
    #[test]
    fn a_kernel_without_psi_reads_as_absent_and_not_as_ok() {
        assert!(parse_pressure("").is_none());
        assert!(parse_pressure("some avg10=1.00 total=5\n").is_none(), "no full line");
        assert!(parse_pressure("garbage\n").is_none());
    }

    /// The probe must carry that absence rather than defaulting it away, which
    /// is what every other counter in `read()` does.
    #[test]
    fn a_proc_tree_with_no_pressure_file_yields_no_reading() {
        let dir = tmp("no-psi");
        let probe = fixture(&dir, &["sda"]);
        assert!(probe.read().mem_pressure.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_proc_tree_with_a_pressure_file_yields_one() {
        let dir = tmp("psi");
        let probe = fixture(&dir, &["sda"]);
        std::fs::create_dir_all(dir.join("proc/pressure")).unwrap();
        std::fs::write(dir.join("proc/pressure/memory"), PRESSURE).unwrap();
        assert_eq!(probe.read().mem_pressure.unwrap().level, "ok");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- the per-core grid --------------------------------------------------

    /// Real shape: `cpu ` aggregate first, then one line per logical core.
    #[test]
    fn the_aggregate_line_is_not_a_core() {
        let cores = parse_cpu_cores(STAT);
        assert_eq!(cores.len(), 2, "cpu0 and cpu1, not the `cpu ` total");
    }

    #[test]
    fn a_core_line_splits_into_user_system_iowait_and_idle() {
        // user nice system idle iowait irq softirq
        let c = parse_cpu_cores("cpu0 10 5 20 100 7 1 2\n");
        assert_eq!(c[0].user, 15, "user + nice");
        assert_eq!(c[0].system, 23, "system + irq + softirq");
        assert_eq!(c[0].iowait, 7);
        assert_eq!(c[0].idle, 100);
    }

    /// The judgement worth pinning: a core waiting on a disk is NOT busy. Calling
    /// it busy would diagnose a slow disk as a CPU shortage.
    #[test]
    fn iowait_does_not_count_as_busy() {
        let c = CoreTimes { user: 10, system: 5, iowait: 80, idle: 5 };
        assert_eq!(c.busy(), 15);
        assert_eq!(c.total(), 100);
    }

    /// A truncated line is a read that went wrong, and a core at 0% reads exactly
    /// like a core that is idle - so it is dropped rather than zero-filled.
    #[test]
    fn a_truncated_core_line_is_dropped_rather_than_reported_idle() {
        assert!(parse_cpu_cores("cpu0 1 2\n").is_empty());
        assert_eq!(parse_cpu_cores("cpu0 1 2\ncpu1 1 1 1 1 1\n").len(), 1);
    }

    /// Per core, not per machine: one pegged core on a two-core box is 100% of
    /// itself, which is the number the grid exists to show.
    #[test]
    fn one_pegged_core_reads_full_while_its_neighbour_reads_idle() {
        let prev = parse_cpu_cores("cpu0 0 0 0 0 0\ncpu1 0 0 0 0 0\n");
        let now = parse_cpu_cores("cpu0 100 0 0 0 0\ncpu1 0 0 0 100 0\n");
        let u = core_percentages(&now, &prev);
        assert_eq!(u[0].user, 100.0);
        assert_eq!(u[1].user, 0.0);
    }

    #[test]
    fn the_three_shares_are_reported_separately() {
        let prev = parse_cpu_cores("cpu0 0 0 0 0 0\n");
        let now = parse_cpu_cores("cpu0 25 0 25 25 25\n");
        let u = core_percentages(&now, &prev);
        assert_eq!(u[0].user, 25.0);
        assert_eq!(u[0].system, 25.0);
        assert_eq!(u[0].iowait, 25.0);
    }

    /// The first sample, and an offlined core: nothing advanced, so nothing is
    /// claimed. Zeros here mean "not measured" and must not divide.
    #[test]
    fn a_core_that_did_not_advance_reports_zero_rather_than_dividing() {
        let same = parse_cpu_cores("cpu0 5 5 5 5 5\n");
        let u = core_percentages(&same, &same);
        assert_eq!(u[0], CoreUsage::default());
    }

    /// A core count that changed between samples has no pair to difference.
    #[test]
    fn cores_without_a_partner_in_the_previous_sample_are_dropped() {
        let prev = parse_cpu_cores("cpu0 0 0 0 0 0\n");
        let now = parse_cpu_cores("cpu0 10 0 0 0 0\ncpu1 10 0 0 0 0\n");
        assert_eq!(core_percentages(&now, &prev).len(), 1);
    }

    // ---- load average -------------------------------------------------------

    /// This machine's real line, including the trailing fields nobody wants.
    #[test]
    fn the_real_loadavg_line_parses_and_ignores_its_tail() {
        let l = parse_loadavg("2.45 3.48 4.04 5/3333 2710519\n", 16).unwrap();
        assert_eq!(l.one, 2.45);
        assert_eq!(l.five, 3.48);
        assert_eq!(l.fifteen, 4.04);
    }

    /// The number that is actually comparable. A load of 8 is half-idle on this
    /// box and badly oversubscribed on a dual-core one, and the raw figure looks
    /// identical in both.
    #[test]
    fn the_same_load_reads_differently_on_different_machines() {
        assert_eq!(parse_loadavg("8 0 0", 16).unwrap().per_core, 0.5);
        assert_eq!(parse_loadavg("8 0 0", 2).unwrap().per_core, 4.0);
    }

    #[test]
    fn saturation_is_one_per_core() {
        assert_eq!(parse_loadavg("16 0 0", 16).unwrap().per_core, 1.0);
    }

    /// 0.00 is a real and reassuring reading, so a failed parse must not be able
    /// to produce one.
    #[test]
    fn an_unreadable_loadavg_is_absent_rather_than_zero() {
        assert!(parse_loadavg("", 4).is_none());
        assert!(parse_loadavg("1.0 2.0", 4).is_none(), "a short line is not a load");
        assert!(parse_loadavg("banana 2 3", 4).is_none());
    }

    /// A machine whose core count could not be read either must not divide by it.
    #[test]
    fn a_zero_core_count_does_not_blow_up_the_division() {
        let l = parse_loadavg("4 0 0", 0).unwrap();
        assert_eq!(l.per_core, 4.0);
    }

    #[test]
    fn a_proc_tree_with_no_loadavg_yields_none() {
        let dir = tmp("no-load");
        let probe = fixture(&dir, &["sda"]);
        assert!(probe.read().load.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- per-device disk ----------------------------------------------------

    #[test]
    fn only_whole_disks_appear_in_the_breakdown() {
        // The real shape: a disk and two of its partitions.
        let text = concat!(
            " 259 0 nvme0n1 1 0 1000 1 1 0 2000 1 0 1 1\n",
            " 259 1 nvme0n1p1 1 0 500 1 1 0 500 1 0 1 1\n",
            " 259 2 nvme0n1p2 1 0 400 1 1 0 400 1 0 1 1\n",
        );
        let d = parse_diskstats_devices(text, &|n| n == "nvme0n1");
        assert_eq!(d.len(), 1, "a disk beside its own partitions is counted once");
        assert_eq!(d[0].name, "nvme0n1");
        assert_eq!(d[0].read_sectors, 1000);
        assert_eq!(d[0].write_sectors, 2000);
    }

    /// The reason the breakdown exists: two disks doing different things read
    /// identically in one total.
    #[test]
    fn two_disks_report_their_own_traffic() {
        let prev = vec![
            DeviceCounters { name: "sda".into(), read_sectors: 0, write_sectors: 0 },
            DeviceCounters { name: "sdb".into(), read_sectors: 0, write_sectors: 0 },
        ];
        // 2048 sectors = 1 MiB, over one second.
        let now = vec![
            DeviceCounters { name: "sda".into(), read_sectors: 2048, write_sectors: 0 },
            DeviceCounters { name: "sdb".into(), read_sectors: 0, write_sectors: 4096 },
        ];
        let r = device_rates(&now, &prev, 1.0);
        assert_eq!(r[0].name, "sda");
        assert_eq!(r[0].read_mbs, 1.0);
        assert_eq!(r[0].write_mbs, 0.0);
        assert_eq!(r[1].write_mbs, 2.0);
    }

    /// Matched by NAME, not by position: a USB disk appearing mid-session shifts
    /// every later line, and by index every device would report its neighbour's
    /// traffic.
    #[test]
    fn a_disk_appearing_between_samples_does_not_shift_the_others() {
        let prev = vec![DeviceCounters { name: "sdb".into(), read_sectors: 0, write_sectors: 0 }];
        let now = vec![
            DeviceCounters { name: "sda".into(), read_sectors: 9999, write_sectors: 9999 },
            DeviceCounters { name: "sdb".into(), read_sectors: 2048, write_sectors: 0 },
        ];
        let r = device_rates(&now, &prev, 1.0);
        assert_eq!(r.len(), 1, "the newcomer has no interval yet");
        assert_eq!(r[0].name, "sdb");
        assert_eq!(r[0].read_mbs, 1.0, "and sdb still reports its own traffic");
    }

    #[test]
    fn a_counter_that_went_backwards_does_not_wrap_into_a_huge_rate() {
        let prev = vec![DeviceCounters { name: "sda".into(), read_sectors: 500, write_sectors: 0 }];
        let now = vec![DeviceCounters { name: "sda".into(), read_sectors: 0, write_sectors: 0 }];
        assert_eq!(device_rates(&now, &prev, 1.0)[0].read_mbs, 0.0);
    }

    #[test]
    fn a_zero_interval_yields_no_rates_rather_than_infinity() {
        let prev = vec![DeviceCounters { name: "sda".into(), read_sectors: 0, write_sectors: 0 }];
        let now = vec![DeviceCounters { name: "sda".into(), read_sectors: 2048, write_sectors: 0 }];
        assert!(device_rates(&now, &prev, 0.0).is_empty());
    }

    /// The double count this found: a LUKS mapper's bytes are the backing disk's
    /// bytes, and `/sys/block/dm-0` exists just like a real disk's does.
    #[test]
    fn a_stacked_device_is_not_counted_beside_the_disk_it_sits_on() {
        let dir = tmp("stacked");
        let probe = fixture(&dir, &["nvme0n1", "dm-0"]);
        // dm-0 sits on a partition of the nvme, the way LUKS and LVM do.
        std::fs::create_dir_all(dir.join("sys/block/dm-0/slaves/nvme0n1p2")).unwrap();
        std::fs::write(
            dir.join("proc/diskstats"),
            " 259 0 nvme0n1 1 0 2048 1 1 0 2048 1 0 1 1\n             253 0 dm-0 1 0 2048 1 1 0 2048 1 0 1 1\n",
        )
        .unwrap();
        let c = probe.read();
        assert_eq!(c.devices.len(), 1, "the mapper is the same traffic, not more of it");
        assert_eq!(c.devices[0].name, "nvme0n1");
        assert_eq!(c.disk_read_sectors, 2048, "and the total is not doubled");
    }

    // ---- temperature and clock ---------------------------------------------

    /// The case the allowlist exists for: this laptop's `/sys/class/hwmon` holds
    /// the charger, two DIMM sensors and the WiFi radio alongside the CPU, and
    /// picking by position would put one of those under a "CPU" heading.
    #[test]
    fn the_cpu_sensor_is_chosen_by_name_and_not_by_position() {
        let dir = tmp("temp");
        for (i, (name, val)) in [("ACAD", "0"), ("spd5118", "45000"), ("k10temp", "61500")]
            .iter()
            .enumerate()
        {
            let h = dir.join("sys/class/hwmon").join(format!("hwmon{i}"));
            std::fs::create_dir_all(&h).unwrap();
            std::fs::write(h.join("name"), name).unwrap();
            std::fs::write(h.join("temp1_input"), val).unwrap();
        }
        let got = read_cpu_temp(&dir.join("sys")).unwrap();
        assert_eq!(got.celsius, 61.5, "the k10temp, not the first chip");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A machine with sensors but none of them a CPU one reads as unmeasured. A
    /// plausible-looking 0 C would be a lie on any machine that is switched on.
    #[test]
    fn a_machine_with_no_cpu_sensor_reports_nothing_rather_than_zero() {
        let dir = tmp("temp-none");
        let h = dir.join("sys/class/hwmon/hwmon0");
        std::fs::create_dir_all(&h).unwrap();
        std::fs::write(h.join("name"), "nvme").unwrap();
        std::fs::write(h.join("temp1_input"), "40000").unwrap();
        assert_eq!(read_cpu_temp(&dir.join("sys")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_hwmon_directory_at_all_is_not_an_error() {
        assert_eq!(read_cpu_temp(std::path::Path::new("/nonexistent")), None);
    }

    #[test]
    fn core_clocks_are_read_per_core_in_megahertz() {
        let dir = tmp("freq");
        for (i, khz) in ["4244342", "1400000"].iter().enumerate() {
            let c = dir.join("sys/devices/system/cpu").join(format!("cpu{i}")).join("cpufreq");
            std::fs::create_dir_all(&c).unwrap();
            std::fs::write(c.join("scaling_cur_freq"), khz).unwrap();
        }
        let f = read_core_freqs(&dir.join("sys"), 2);
        assert_eq!(f[0], Some(4244.342));
        assert_eq!(f[1], Some(1400.0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A machine with no cpufreq at all - a VM, or a driver not loaded - gets an
    /// empty list rather than a row of zeros that would read as idle cores.
    #[test]
    fn a_machine_without_cpufreq_reports_an_empty_list() {
        assert!(read_core_freqs(std::path::Path::new("/nonexistent"), 4).is_empty());
    }

    /// One core missing its directory does not lose the others, and keeps its
    /// place so the list still lines up with the grid.
    #[test]
    fn a_core_without_cpufreq_holds_its_place_as_unmeasured() {
        let dir = tmp("freq-gap");
        let c = dir.join("sys/devices/system/cpu/cpu1/cpufreq");
        std::fs::create_dir_all(&c).unwrap();
        std::fs::write(c.join("scaling_cur_freq"), "2000000").unwrap();
        let f = read_core_freqs(&dir.join("sys"), 3);
        assert_eq!(f, vec![None, Some(2000.0), None]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The finding this label exists for: `Tctl` is a control value with a
    /// vendor offset, and on this laptop it reads a steady 100.1 C while the die
    /// is far cooler. Where a chip offers both, the real one wins.
    #[test]
    fn a_die_reading_beats_the_offset_control_value() {
        let dir = tmp("tdie");
        let h = dir.join("sys/class/hwmon/hwmon0");
        std::fs::create_dir_all(&h).unwrap();
        std::fs::write(h.join("name"), "k10temp").unwrap();
        std::fs::write(h.join("temp1_input"), "100125").unwrap();
        std::fs::write(h.join("temp1_label"), "Tctl").unwrap();
        std::fs::write(h.join("temp2_input"), "73000").unwrap();
        std::fs::write(h.join("temp2_label"), "Tdie").unwrap();
        let got = read_cpu_temp(&dir.join("sys")).unwrap();
        assert_eq!(got.celsius, 73.0);
        assert_eq!(got.label, "Tdie");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// And where only Tctl exists, it is reported UNDER ITS OWN NAME rather than
    /// promoted to a die temperature it is not.
    #[test]
    fn a_tctl_only_chip_says_that_is_what_it_is() {
        let dir = tmp("tctl");
        let h = dir.join("sys/class/hwmon/hwmon0");
        std::fs::create_dir_all(&h).unwrap();
        std::fs::write(h.join("name"), "k10temp").unwrap();
        std::fs::write(h.join("temp1_input"), "100125").unwrap();
        std::fs::write(h.join("temp1_label"), "Tctl").unwrap();
        let got = read_cpu_temp(&dir.join("sys")).unwrap();
        assert_eq!(got.label, "Tctl");
        assert_eq!(got.celsius, 100.125);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
