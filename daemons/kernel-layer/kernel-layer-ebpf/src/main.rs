#![no_std]
#![no_main]

use aya_ebpf::{
    macros::{map, tracepoint},
    maps::RingBuf,
    programs::TracePointContext,
    helpers::{
        bpf_get_current_cgroup_id, bpf_get_current_uid_gid, bpf_ktime_get_ns,
        // Both string readers, because the two probes read from different address
        // spaces and the distinction is not cosmetic: `openat` gets a userspace
        // pointer out of the syscall arguments, while `sched_process_exec` reads
        // its filename out of the tracepoint's own buffer, which is kernel
        // memory. Using the user reader on the kernel buffer returns nothing.
        bpf_probe_read_kernel_str_bytes, bpf_probe_read_user_str_bytes,
    },
    // `as_ptr` is a trait method rather than an inherent one, so the trait has to
    // be in scope for `ctx.as_ptr()` to resolve. Its absence is what made this
    // crate fail to build with `no method named as_ptr found for
    // TracePointContext` - a missing import reading like a missing API.
    EbpfContext,
};
use aya_log_ebpf::debug;
use kernel_layer_common::{
    FileOpenedEvent, FileWrittenEvent, NetStateEvent, ProcessExecEvent,
    MAX_COMM_LEN, MAX_PATH_LEN,
};

/// Ring buffer shared between the eBPF program and the user-space daemon.
/// Capacity: 256KB. Each FileOpenedEvent is ~280 bytes so this holds ~900 events.
/// The user-space daemon drains this continuously; the capacity is a safety buffer.
#[map]
static EVENTS: RingBuf = RingBuf::with_byte_size(256 * 1024, 0);

/// Tracepoint attached to sys_enter_openat.
///
/// The tracepoint fires on every openat syscall entry. We read the filename
/// from user-space memory and write a FileOpenedEvent into the ring buffer.
///
/// We use the entry tracepoint (sys_enter) rather than exit (sys_exit) because
/// the filename pointer is available at entry. The return value is not yet known
/// at entry; we record 0 as a placeholder. A future improvement would attach a
/// second tracepoint to sys_exit_openat to capture the return value.
#[tracepoint]
pub fn file_opened(ctx: TracePointContext) -> u32 {
    match try_file_opened(ctx) {
        Ok(()) => 0,
        Err(_) => 0, // Never fail; just drop the event on error.
    }
}

fn try_file_opened(ctx: TracePointContext) -> Result<(), i64> {
    // sys_enter_openat tracepoint args layout:
    //   +0:  __syscall_nr (int)
    //   +8:  dfd (int)
    //   +16: filename (const char __user *)
    //   +24: flags (int)
    //   +32: mode (umode_t)
    let filename_ptr = unsafe {
        ctx.read_at::<u64>(16).map_err(|_| -1i64)? as *const u8
    };

    let uid_gid = unsafe { bpf_get_current_uid_gid() };
    let uid = (uid_gid & 0xffffffff) as u32;
    let pid = (unsafe { aya_ebpf::helpers::bpf_get_current_pid_tgid() } >> 32) as u32;
    let timestamp_ns = unsafe { bpf_ktime_get_ns() };

    // Reserve space in the ring buffer.
    let mut entry = EVENTS.reserve::<FileOpenedEvent>(0).ok_or(-1i64)?;

    // Write fields directly into the reserved ring buffer slot.
    // We use unsafe because we are writing into a raw memory region.
    let event = unsafe { entry.assume_init_mut() };
    event.pid = pid;
    event.uid = uid;
    event.timestamp_ns = timestamp_ns;
    event.ret = 0; // Placeholder; return value not available at entry.
    // The reserved slot is uninitialized, so every field must be assigned
    // explicitly; an unassigned field is UB the verifier may reject. cgroup_id is
    // the join key attributing this open to its per-command cgroup.
    event.cgroup_id = unsafe { bpf_get_current_cgroup_id() };
    event.path = [0u8; MAX_PATH_LEN];

    // Copy the filename from user-space into the event.
    // bpf_probe_read_user_str_bytes reads a null-terminated string safely.
    unsafe {
        let _ = bpf_probe_read_user_str_bytes(
            filename_ptr,
            &mut event.path,
        );
    }

    debug!(&ctx, "openat pid={} uid={}", pid, uid);

    entry.submit(0);
    Ok(())
}

// ===== Process exec tracepoint =====

#[map]
static EXEC_EVENTS: RingBuf = RingBuf::with_byte_size(128 * 1024, 0);

#[tracepoint]
pub fn process_exec(ctx: TracePointContext) -> u32 {
    match try_process_exec(ctx) {
        Ok(()) => 0,
        Err(_) => 0,
    }
}

fn try_process_exec(ctx: TracePointContext) -> Result<(), i64> {
    let uid_gid = unsafe { bpf_get_current_uid_gid() };
    let uid = (uid_gid & 0xffffffff) as u32;
    let pid = (unsafe { aya_ebpf::helpers::bpf_get_current_pid_tgid() } >> 32) as u32;
    let timestamp_ns = unsafe { bpf_ktime_get_ns() };

    // READ THE FIELD BEFORE RESERVING, and the order is load-bearing rather than
    // stylistic. This read is fallible, and its `?` used to sit AFTER the
    // reservation - so the error path returned while still holding the ring-buffer
    // slot. The verifier rejects the whole program for it:
    //
    //     Unreleased reference id=5 alloc_insn=11
    //     the BPF_PROG_LOAD syscall failed ... Invalid argument (os error 22)
    //
    // Nothing caught it until the sensor was put on an image and the kernel
    // refused to load it, because the program had never been loaded anywhere.
    // The sibling openat probe already reads its pointer before reserving; this
    // now matches, and a reservation is only taken once nothing left can fail.
    let data_loc: u32 = unsafe { ctx.read_at(8).map_err(|_| -1i64)? };

    let mut entry = EXEC_EVENTS.reserve::<ProcessExecEvent>(0).ok_or(-1i64)?;
    let event = unsafe { entry.assume_init_mut() };
    event.pid = pid;
    event.uid = uid;
    event.timestamp_ns = timestamp_ns;
    event.comm = [0u8; MAX_COMM_LEN];
    event.filename = [0u8; MAX_PATH_LEN];

    // Read process name (comm, max 16 bytes).
    let _ = unsafe {
        aya_ebpf::helpers::bpf_get_current_comm()
    }.map(|comm| event.comm = comm);

    // `data_loc` was read above, before the reservation. Its low 16 bits are the
    // offset of the string from the entry start, the high 16 its length.
    let str_offset = (data_loc & 0xFFFF) as usize;
    let filename_ptr = (ctx.as_ptr() as usize + str_offset) as *const u8;
    unsafe {
        let _ = bpf_probe_read_kernel_str_bytes(filename_ptr, &mut event.filename);
    }

    entry.submit(0);
    Ok(())
}

// ===== File write tracepoint =====

#[map]
static WRITE_EVENTS: RingBuf = RingBuf::with_byte_size(256 * 1024, 0);

#[tracepoint]
pub fn file_written(ctx: TracePointContext) -> u32 {
    match try_file_written(ctx) {
        Ok(()) => 0,
        Err(_) => 0,
    }
}

fn try_file_written(ctx: TracePointContext) -> Result<(), i64> {
    // sys_enter_write args (relative to args start):
    //   +0: __syscall_nr (4 bytes, padded to 8)
    //   +8: fd (8 bytes, unsigned long)
    //   +16: buf (8 bytes, pointer)
    //   +24: count (8 bytes, size_t)
    let fd: u64 = unsafe { ctx.read_at(8).map_err(|_| -1i64)? };
    let count: u64 = unsafe { ctx.read_at(24).map_err(|_| -1i64)? };

    // Skip stdin/stdout/stderr
    if fd <= 2 {
        return Ok(());
    }

    let uid_gid = unsafe { bpf_get_current_uid_gid() };
    let uid = (uid_gid & 0xffffffff) as u32;
    let pid = (unsafe { aya_ebpf::helpers::bpf_get_current_pid_tgid() } >> 32) as u32;
    let timestamp_ns = unsafe { bpf_ktime_get_ns() };

    let mut entry = WRITE_EVENTS.reserve::<FileWrittenEvent>(0).ok_or(-1i64)?;
    let event = unsafe { entry.assume_init_mut() };
    event.pid = pid;
    event.uid = uid;
    event.timestamp_ns = timestamp_ns;
    event.fd = fd;
    event.count = count;

    entry.submit(0);
    Ok(())
}

// ===== Network state change tracepoint =====

#[map]
static NET_EVENTS: RingBuf = RingBuf::with_byte_size(128 * 1024, 0);

/// TCP_ESTABLISHED from include/net/tcp_states.h
const TCP_ESTABLISHED: u32 = 1;

#[tracepoint]
pub fn net_state_change(ctx: TracePointContext) -> u32 {
    match try_net_state_change(ctx) {
        Ok(()) => 0,
        Err(_) => 0,
    }
}

fn try_net_state_change(ctx: TracePointContext) -> Result<(), i64> {
    // inet_sock_set_state tracepoint args (relative to args start):
    //   +0:  skaddr (8 bytes, pointer)
    //   +8:  oldstate (4 bytes)
    //   +12: newstate (4 bytes)
    //   +16: sport (2 bytes)
    //   +18: dport (2 bytes)
    //   +20: family (2 bytes)
    //   +22: protocol (2 bytes)
    //   +24: saddr[4]
    //   +28: daddr[4]
    //   +32: saddr_v6[16]
    //   +48: daddr_v6[16]
    let newstate: u32 = unsafe { ctx.read_at(12).map_err(|_| -1i64)? };
    if newstate != TCP_ESTABLISHED {
        return Ok(());
    }
    let oldstate: u32 = unsafe { ctx.read_at(8).map_err(|_| -1i64)? };
    let family: u16 = unsafe { ctx.read_at(20).map_err(|_| -1i64)? };

    // Only handle IPv4 (2) and IPv6 (10)
    if family != 2 && family != 10 {
        return Ok(());
    }

    let sport: u16 = unsafe { ctx.read_at(16).map_err(|_| -1i64)? };
    let dport: u16 = unsafe { ctx.read_at(18).map_err(|_| -1i64)? };

    // Skip loopback: check daddr for 127.0.0.1
    if family == 2 {
        let daddr_first: u8 = unsafe { ctx.read_at(28).map_err(|_| -1i64)? };
        if daddr_first == 127 {
            return Ok(());
        }
    }

    let uid_gid = unsafe { bpf_get_current_uid_gid() };
    let uid = (uid_gid & 0xffffffff) as u32;
    let pid = (unsafe { aya_ebpf::helpers::bpf_get_current_pid_tgid() } >> 32) as u32;
    let timestamp_ns = unsafe { bpf_ktime_get_ns() };

    // Same rule as the other probes, for the same measured reason: every fallible
    // read happens BEFORE the reservation. The address reads used to sit after it,
    // and their `?` returned holding the slot - the verifier answered
    // `Unreleased reference id=7 alloc_insn=74` and refused to load the program.
    //
    // The IPv6 loopback case still discards explicitly further down; that path
    // decides not to send an event it has already reserved for, which is what
    // `discard` is for. What must never happen is LEAVING without saying either.
    let (saddr_v4, daddr_v4, saddr_v6, daddr_v6) = if family == 2 {
        // IPv4: 4 bytes each at offsets 24 and 28.
        let s: [u8; 4] = unsafe { ctx.read_at(24).map_err(|_| -1i64)? };
        let d: [u8; 4] = unsafe { ctx.read_at(28).map_err(|_| -1i64)? };
        (s, d, [0u8; 16], [0u8; 16])
    } else {
        // IPv6: 16 bytes each at offsets 32 and 48.
        let s: [u8; 16] = unsafe { ctx.read_at(32).map_err(|_| -1i64)? };
        let d: [u8; 16] = unsafe { ctx.read_at(48).map_err(|_| -1i64)? };
        // Loopback ::1 is dropped before anything is reserved, so this case no
        // longer needs a discard at all.
        if d == [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1] {
            return Ok(());
        }
        ([0u8; 4], [0u8; 4], s, d)
    };

    let mut entry = NET_EVENTS.reserve::<NetStateEvent>(0).ok_or(-1i64)?;
    let event = unsafe { entry.assume_init_mut() };
    event.pid = pid;
    event.uid = uid;
    event.timestamp_ns = timestamp_ns;
    event.af = family;
    event.sport = sport;
    event.dport = dport;
    event._pad = 0;
    event.oldstate = oldstate;
    event.newstate = newstate;
    event.saddr = [0u8; 4];
    event.daddr = [0u8; 4];
    event.saddr_v6 = [0u8; 16];
    event.daddr_v6 = [0u8; 16];

    // The addresses were read above, before the reservation.
    event.saddr = saddr_v4;
    event.daddr = daddr_v4;
    event.saddr_v6 = saddr_v6;
    event.daddr_v6 = daddr_v6;

    entry.submit(0);
    Ok(())
}

/// Required by the eBPF verifier: every eBPF program must handle panics.
#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

/// License declaration required by the kernel for certain eBPF helper functions.
#[unsafe(link_section = "license")]
#[unsafe(no_mangle)]
static LICENSE: [u8; 13] = *b"Dual MIT/GPL\0";
