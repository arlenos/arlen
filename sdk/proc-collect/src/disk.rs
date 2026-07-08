//! Disk I/O throughput over `/proc/diskstats` (system-monitor-plan.md item 2).
//! sysinfo has no disk-I/O throughput, so this is hand-rolled and WRITE-CLEAN.
//! diskstats is CUMULATIVE (sectors since boot), so like CPU and network the
//! [`Collector`](crate::Collector) holds the previous read and reports the delta.
//!
//! Only WHOLE-DISK devices are summed, never partitions: a partition's I/O is
//! already counted in its disk's line, so summing both would double-count. A
//! device is a whole disk when `/sys/block/<name>` exists.

use std::fs;

/// A disk sector is 512 bytes in the `/proc/diskstats` accounting, regardless of
/// the device's physical sector size (this is the kernel's fixed unit here).
const SECTOR_BYTES: u64 = 512;

/// Sum cumulative bytes read and written across the whole-disk devices in
/// `/proc/diskstats` content. Fields (0-indexed after splitting on whitespace):
/// 2 = device name, 5 = sectors read, 9 = sectors written. `is_whole_disk` filters
/// out partitions (and loop/ram/dm virtual devices). Returns `(read, written)` in
/// bytes, saturating.
pub fn parse_diskstats_cumulative(content: &str, is_whole_disk: impl Fn(&str) -> bool) -> (u64, u64) {
    let mut read_bytes = 0u64;
    let mut written_bytes = 0u64;
    for line in content.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 10 {
            continue;
        }
        if !is_whole_disk(f[2]) {
            continue;
        }
        if let (Ok(sectors_read), Ok(sectors_written)) = (f[5].parse::<u64>(), f[9].parse::<u64>()) {
            read_bytes = read_bytes.saturating_add(sectors_read.saturating_mul(SECTOR_BYTES));
            written_bytes =
                written_bytes.saturating_add(sectors_written.saturating_mul(SECTOR_BYTES));
        }
    }
    (read_bytes, written_bytes)
}

/// Whether `name` is a whole disk (has a `/sys/block/<name>` entry), so partitions
/// and virtual devices are excluded from the throughput total.
fn is_whole_disk(name: &str) -> bool {
    // A partition name can contain a slash-free device name only; reject anything
    // that could escape /sys/block before the existence check.
    if name.is_empty() || name.contains('/') || name.contains("..") {
        return false;
    }
    fs::metadata(format!("/sys/block/{name}")).is_ok()
}

/// Read the cumulative `(bytes_read, bytes_written)` across whole-disk devices from
/// `/proc/diskstats`. `(0, 0)` if it is unreadable.
pub fn read_disk_cumulative() -> (u64, u64) {
    match fs::read_to_string("/proc/diskstats") {
        Ok(content) => parse_diskstats_cumulative(&content, is_whole_disk),
        Err(_) => (0, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Two whole disks (sda, nvme0n1) and their partitions; summing partitions too
    // would double-count.
    const DISKSTATS: &str = "\
   8       0 sda 100 0 2000 50 200 0 4000 80 0 0 0
   8       1 sda1 40 0 800 20 90 0 1500 30 0 0 0
 259       0 nvme0n1 300 0 6000 40 400 0 8000 60 0 0 0
 259       1 nvme0n1p1 100 0 2000 10 100 0 2000 20 0 0 0
   7       0 loop0 5 0 40 1 0 0 0 0 0 0 0
";

    #[test]
    fn sums_only_whole_disks_not_partitions() {
        // Whole disks sda + nvme0n1: sectors read 2000 + 6000 = 8000; written
        // 4000 + 8000 = 12000. Times 512 bytes.
        let whole = |n: &str| matches!(n, "sda" | "nvme0n1");
        let (read, written) = parse_diskstats_cumulative(DISKSTATS, whole);
        assert_eq!(read, 8000 * 512);
        assert_eq!(written, 12000 * 512);
    }

    #[test]
    fn a_filter_that_accepts_everything_would_double_count() {
        // Proves the partition problem is real: accepting all rows sums the
        // partitions on top of their disks, so it exceeds the whole-disk total.
        let (read_all, _) = parse_diskstats_cumulative(DISKSTATS, |_| true);
        let (read_whole, _) = parse_diskstats_cumulative(DISKSTATS, |n| matches!(n, "sda" | "nvme0n1"));
        assert!(read_all > read_whole);
    }

    #[test]
    fn malformed_or_short_lines_are_skipped() {
        assert_eq!(parse_diskstats_cumulative("garbage\n8 0 sda 1 2\n", |_| true), (0, 0));
    }

    #[test]
    fn a_traversal_device_name_is_never_a_whole_disk() {
        assert!(!is_whole_disk("../../etc/passwd"));
        assert!(!is_whole_disk("a/b"));
        assert!(!is_whole_disk(""));
    }
}
