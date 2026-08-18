// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Check the offsets the probes were compiled against against the kernel's own.
//!
//! A tracepoint record's layout is not part of any ABI. The probe reads a field
//! by byte offset, and a wrong offset does not fail: it returns whatever those
//! bytes happen to hold. That is not a hypothetical here. Both file probes read
//! eight bytes short of their filename argument for months, and the way it
//! showed up was a boot recording 50687 opens and discarding all 50687 for an
//! empty path, with no error anywhere.
//!
//! The fork probe is worse in that respect, because its wrong value would be a
//! plausible one. `child_pid` read from the wrong offset gives some other pid,
//! the map fills happily, every later lookup misses, and the sensor reports the
//! same counts it would if everything worked - only with no launches in the
//! graph, which reads as "this machine launches nothing".
//!
//! So the daemon reads `/sys/kernel/tracing/events/.../format`, which is the
//! kernel telling you where it put the fields, and refuses to attach the probe
//! if it disagrees. It runs as root already; nothing else in the tree may read
//! that path.

use anyhow::{bail, Context, Result};
use kernel_layer_common::{SCHED_FORK_CHILD_PID, SCHED_FORK_PARENT_PID};

/// Where the kernel publishes tracepoint layouts. `debugfs` is the older mount
/// point and is still where it lands on some kernels, so both are tried.
const FORMAT_PATHS: [&str; 2] = [
    "/sys/kernel/tracing/events/sched/sched_process_fork/format",
    "/sys/kernel/debug/tracing/events/sched/sched_process_fork/format",
];

/// Offset of a named field in a tracepoint format description.
///
/// The lines look like:
///
/// ```text
///     field:pid_t child_pid;  offset:44;      size:4; signed:1;
/// ```
///
/// Matching on the name as a whole word, because `parent_pid` and `child_pid`
/// both end in `_pid` and a substring match would take whichever came first.
pub fn field_offset(format: &str, field: &str) -> Option<usize> {
    for line in format.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("field:") else {
            continue;
        };
        let (decl, tail) = rest.split_once(';')?;
        // The declaration is `<type> <name>` or `<type> <name>[N]`; take the last
        // token and drop any array suffix.
        let name = decl.rsplit(|c: char| c.is_whitespace() || c == '*').next()?;
        let name = name.split('[').next()?;
        if name != field {
            continue;
        }
        let at = tail.find("offset:")? + "offset:".len();
        let digits: String = tail[at..].chars().take_while(|c| c.is_ascii_digit()).collect();
        return digits.parse().ok();
    }
    None
}

/// What the kernel says, against what the probe was built with.
///
/// Returns the offsets it read, so a caller can log them rather than only the
/// verdict. An unreadable format file is NOT an error: a kernel without tracefs
/// mounted still runs the probe, and refusing to start over a missing debug
/// mount would trade a real capability for a check.
pub fn check_fork_layout(format: &str) -> Result<(usize, usize)> {
    let child = field_offset(format, "child_pid")
        .context("the fork tracepoint format names no child_pid field")?;
    let parent = field_offset(format, "parent_pid")
        .context("the fork tracepoint format names no parent_pid field")?;
    if child != SCHED_FORK_CHILD_PID || parent != SCHED_FORK_PARENT_PID {
        bail!(
            "this kernel puts sched_process_fork's pids at parent={parent} child={child}, \
             the probe was built for parent={SCHED_FORK_PARENT_PID} child={SCHED_FORK_CHILD_PID}. \
             Attaching would record launches against the wrong process and report nothing wrong"
        );
    }
    Ok((parent, child))
}

/// Read the running kernel's layout and check it. Absent tracefs is a warning,
/// a disagreement is fatal.
pub fn verify_fork_record_layout() -> Result<()> {
    let Some(format) = FORMAT_PATHS.iter().find_map(|p| std::fs::read_to_string(p).ok()) else {
        log::warn!(
            "no tracefs to read sched_process_fork's layout from; attaching the fork probe \
             with the compiled-in offsets unchecked"
        );
        return Ok(());
    };
    let (parent, child) = check_fork_layout(&format)?;
    log::info!("sched_process_fork layout agrees with the probe: parent_pid={parent} child_pid={child}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real thing, from a 6.x x86_64 kernel.
    const REAL: &str = "name: sched_process_fork\nID: 313\nformat:\n\tfield:unsigned short common_type;\toffset:0;\tsize:2;\tsigned:0;\n\tfield:unsigned char common_flags;\toffset:2;\tsize:1;\tsigned:0;\n\tfield:unsigned char common_preempt_count;\toffset:3;\tsize:1;\tsigned:0;\n\tfield:int common_pid;\toffset:4;\tsize:4;\tsigned:1;\n\n\tfield:char parent_comm[16];\toffset:8;\tsize:16;\tsigned:0;\n\tfield:pid_t parent_pid;\toffset:24;\tsize:4;\tsigned:1;\n\tfield:char child_comm[16];\toffset:28;\tsize:16;\tsigned:0;\n\tfield:pid_t child_pid;\toffset:44;\tsize:4;\tsigned:1;\n\nprint fmt: \"comm=%s pid=%d child_comm=%s child_pid=%d\"\n";

    #[test]
    fn the_probe_offsets_match_a_real_kernels_format() {
        assert_eq!(check_fork_layout(REAL).unwrap(), (24, 44));
    }

    /// A name that is a suffix of another must not be matched by accident: this
    /// is the bug that would silently swap parent for child.
    #[test]
    fn a_field_is_matched_whole_and_not_by_suffix() {
        assert_eq!(field_offset(REAL, "child_pid"), Some(44));
        assert_eq!(field_offset(REAL, "parent_pid"), Some(24));
        assert_eq!(field_offset(REAL, "pid"), None);
    }

    /// An array field's brackets belong to the type, not the name.
    #[test]
    fn an_array_field_is_found_under_its_bare_name() {
        assert_eq!(field_offset(REAL, "parent_comm"), Some(8));
    }

    /// The case worth having the check for at all.
    #[test]
    fn a_kernel_that_moved_a_field_is_refused_by_name() {
        let moved = REAL.replace("child_pid;\toffset:44", "child_pid;\toffset:40");
        let err = check_fork_layout(&moved).unwrap_err().to_string();
        assert!(err.contains("child=40"), "{err}");
        assert!(err.contains("report nothing wrong"), "{err}");
    }

    #[test]
    fn a_format_missing_the_field_is_an_error_not_a_zero() {
        assert!(check_fork_layout("name: nothing\nformat:\n").is_err());
    }
}
