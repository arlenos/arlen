//! Types shared between the eBPF kernel program and the user-space daemon.
//!
//! This crate must remain `#![no_std]` compatible because it is compiled
//! into the eBPF program which runs in kernel context without the standard
//! library. User-space code can enable the `user` feature to get additional
//! trait implementations that require `std`.

#![no_std]

/// Maximum length of a file path stored in a `FileOpenedEvent`.
/// Longer paths are truncated. 256 bytes covers most practical paths.
pub const MAX_PATH_LEN: usize = 256;

/// Byte offset of the FIRST syscall argument inside a `sys_enter_*` tracepoint
/// record, and the stride between arguments.
///
/// A tracepoint record starts with the common fields - `common_type` (2),
/// `common_flags` (1), `common_preempt_count` (1), `common_pid` (4) - then
/// `__syscall_nr`, then the arguments, each widened to 8 bytes. `read_at` indexes
/// the record, so an argument's offset is `syscall_arg(n)` and not the offset
/// documented in the kernel's own arg-relative tables.
///
/// This exists because both file probes had those two scales confused until 18
/// August: each carried its own comment saying "relative to args start" beside a
/// read that indexed the record, so the openat probe read `dfd` where it wanted
/// `filename` and forwarded nothing for months. Measured on the boot that found
/// it: 50687 opens seen, 50687 discarded for an empty path.
///
/// Naming it once means the next probe inherits the reasoning instead of
/// repeating the mistake, and `syscall_arg(1)` says which argument is wanted
/// where `24` says only where to look.
pub const SYSCALL_ARG_BASE: usize = 16;

/// Offset of the `n`th syscall argument within the tracepoint record. Argument 0
/// is the syscall's first parameter.
pub const fn syscall_arg(n: usize) -> usize {
    SYSCALL_ARG_BASE + n * 8
}

/// Offset of `child_pid` inside a `sched_process_fork` tracepoint record.
///
/// This one is NOT a syscall tracepoint, so `syscall_arg` does not apply: the
/// record is the common 8 bytes, then `parent_comm[16]` at 8, `parent_pid` at
/// 24, `child_comm[16]` at 28, and `child_pid` at 44. The layout comes from the
/// `TRACE_EVENT(sched_process_fork)` macro rather than from a syscall's argument
/// list, which is exactly the distinction the two file probes got wrong.
///
/// The daemon checks this against the kernel's own
/// `/sys/kernel/tracing/events/sched/sched_process_fork/format` at load time and
/// refuses the probe on a mismatch, because a wrong offset here does not fail:
/// it silently keys the fork map by whatever those four bytes happen to be, and
/// every later lookup misses while the counters all look healthy.
pub const SCHED_FORK_CHILD_PID: usize = 44;

/// Offset of `parent_pid` in the same record. Kept beside its sibling so the
/// pair can be checked together against the kernel's format file.
pub const SCHED_FORK_PARENT_PID: usize = 24;

/// An event emitted when a file is opened via the `openat` syscall.
///
/// Written by the eBPF tracepoint program into the ring buffer.
/// Read by the user-space daemon and forwarded to the Event Bus.
///
/// Must be `#[repr(C)]` so the layout is identical in kernel and user-space.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FileOpenedEvent {
    /// Process ID of the calling process.
    pub pid: u32,
    /// User ID of the calling process.
    pub uid: u32,
    /// Monotonic timestamp in nanoseconds (from bpf_ktime_get_ns).
    pub timestamp_ns: u64,
    /// Return value of openat: file descriptor on success, negative errno on failure.
    pub ret: i64,
    /// cgroup v2 id of the calling task (bpf_get_current_cgroup_id). Stable for the
    /// lifetime of the cgroup and free of PID-reuse hazards, so it is the join key
    /// that attributes the open to its per-command cgroup. Placed before `path` to
    /// keep natural 8-byte alignment (the two u32s pack, then u64/i64/u64) and avoid
    /// inserting tail padding the eBPF writer would have to zero.
    pub cgroup_id: u64,
    /// Null-terminated file path, truncated to MAX_PATH_LEN bytes.
    pub path: [u8; MAX_PATH_LEN],
}

// Safety: FileOpenedEvent is a plain C struct with no pointers.
// It is safe to send across threads in user-space.
#[cfg(feature = "user")]
unsafe impl Send for FileOpenedEvent {}

/// Maximum length of a process comm field (Linux TASK_COMM_LEN).
pub const MAX_COMM_LEN: usize = 16;

/// An event emitted when a process calls execve.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ProcessExecEvent {
    pub pid: u32,
    pub uid: u32,
    pub timestamp_ns: u64,
    /// cgroup v2 id of the task doing the exec: the CHILD's app identity.
    pub cgroup_id: u64,
    /// cgroup v2 id the parent held when it forked this task, or 0 when the fork
    /// was not seen (the sensor started mid-session, or the map evicted it).
    ///
    /// This is the whole point of the exec probe. An app's identity is its
    /// cgroup, so a launch is an edge between two cgroups; a shell running `ls`
    /// puts both ends in the same one and resolves to a self-edge, which is not
    /// recorded. That is the frequency filter, and it falls out of the identity
    /// rather than out of a list of interesting binaries.
    ///
    /// It cannot be read off the current task: `task_struct` is opaque in the
    /// generated aya bindings, so `real_parent->cgroups` is not a field read
    /// available here. It comes from the fork tracepoint instead, which sees the
    /// parent's cgroup as its own at the moment the child appears.
    pub parent_cgroup_id: u64,
    pub comm: [u8; MAX_COMM_LEN],
    pub filename: [u8; MAX_PATH_LEN],
}

#[cfg(feature = "user")]
unsafe impl Send for ProcessExecEvent {}

/// An event emitted when a process writes to a file descriptor.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FileWrittenEvent {
    pub pid: u32,
    pub uid: u32,
    pub timestamp_ns: u64,
    pub fd: u64,
    pub count: u64,
    /// cgroup v2 id of the writing task, for the same reason the open event
    /// carries one: a pid is reused within a boot and a cgroup id is not, so it
    /// is the better key for the App node this becomes. Last field, and every
    /// field before it is 8-aligned, so it adds no padding.
    pub cgroup_id: u64,
}

#[cfg(feature = "user")]
unsafe impl Send for FileWrittenEvent {}

/// Maximum length of an IP address stored as a string.
pub const MAX_ADDR_LEN: usize = 46;

/// An event emitted on a TCP state transition to ESTABLISHED.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NetStateEvent {
    pub pid: u32,
    pub uid: u32,
    pub timestamp_ns: u64,
    pub af: u16,       // AF_INET=2, AF_INET6=10
    pub sport: u16,
    pub dport: u16,
    pub _pad: u16,
    pub saddr: [u8; 4],
    pub daddr: [u8; 4],
    pub saddr_v6: [u8; 16],
    pub daddr_v6: [u8; 16],
    pub oldstate: u32,
    pub newstate: u32,
}

#[cfg(feature = "user")]
unsafe impl Send for NetStateEvent {}


#[cfg(test)]
mod arg_offset_tests {
    use super::syscall_arg;

    /// The three offsets the file probes actually use, spelled out so a change to
    /// the base or the stride has to be deliberate. These are the numbers that
    /// were wrong by exactly one common-field block until 18 August.
    #[test]
    fn the_arguments_sit_where_the_record_layout_says() {
        assert_eq!(syscall_arg(0), 16, "first argument, after the common fields and __syscall_nr");
        assert_eq!(syscall_arg(1), 24, "openat filename");
        assert_eq!(syscall_arg(2), 32, "write count");
    }
}
