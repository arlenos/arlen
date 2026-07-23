//! `SCM_RIGHTS` fd passing for the identity broker's wire ops.
//!
//! The stamped-identity broker (`stamped-identity-plan.md`) needs a
//! caller to hand the broker a **pidfd** over a Unix socket: the
//! launcher's `RegisterIdentity` passes the child's pidfd, a daemon's
//! `LookupIdentity` passes its peer's pidfd. The config-broker's normal
//! request/response is length-framed serde JSON with no ancillary data,
//! so this module adds the one primitive that flow needs: send a small
//! payload together with exactly one file descriptor, and receive it,
//! **fail-closed** (a dropped or duplicated fd is refused, never
//! silently accepted, and no descriptor is leaked on the error path).
//!
//! The response direction carries no fd, so it keeps using the plain
//! framed path; only the fd-bearing request uses this.

use std::io::{self, ErrorKind};
use std::mem::size_of;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};

/// The largest fd-message payload accepted. Identity requests are a tiny
/// JSON object (a discriminant plus a short app_id), so this is generous;
/// a payload past it is refused rather than truncated.
pub const MAX_FD_MSG: usize = 4096;

/// Send `payload` together with exactly one file descriptor `fd` in a
/// single `sendmsg` carrying an `SCM_RIGHTS` control message.
///
/// The payload must be non-empty and at most [`MAX_FD_MSG`]; a partial
/// send is an error (the ancillary fd attaches to the first bytes only,
/// so a split would strand the fd). The receiver takes ownership of a
/// duplicate of `fd`; the caller keeps its own.
pub fn send_fd_msg<S: AsRawFd>(sock: &S, payload: &[u8], fd: BorrowedFd<'_>) -> io::Result<()> {
    if payload.is_empty() || payload.len() > MAX_FD_MSG {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "fd-message payload out of range",
        ));
    }

    let mut iov = libc::iovec {
        iov_base: payload.as_ptr() as *mut libc::c_void,
        iov_len: payload.len(),
    };

    // A control buffer sized for exactly one fd.
    // SAFETY: CMSG_SPACE is a pure size computation.
    let cmsg_space = unsafe { libc::CMSG_SPACE(size_of::<libc::c_int>() as u32) } as usize;
    let mut cmsg_buf = vec![0u8; cmsg_space];

    // SAFETY: msghdr is plain-old-data; zeroing then setting the public
    // fields is the standard construction on Linux.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cmsg_buf.as_mut_ptr() as *mut libc::c_void;
    msg.msg_controllen = cmsg_space as _;

    // SAFETY: msg.msg_control points at cmsg_buf (cmsg_space bytes); we
    // fill exactly one cmsghdr + one c_int, all within that buffer.
    unsafe {
        let cmsg = libc::CMSG_FIRSTHDR(&msg);
        // cmsg_buf is non-empty and correctly sized, so CMSG_FIRSTHDR is
        // non-null; assert defensively rather than deref a null.
        if cmsg.is_null() {
            return Err(io::Error::other("cmsg header unavailable"));
        }
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(size_of::<libc::c_int>() as u32) as _;
        let data = libc::CMSG_DATA(cmsg) as *mut libc::c_int;
        *data = fd.as_raw_fd();
    }

    // SAFETY: msg is fully initialised above and points at live buffers.
    let n = unsafe { libc::sendmsg(sock.as_raw_fd(), &msg, libc::MSG_NOSIGNAL) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    if n as usize != payload.len() {
        return Err(io::Error::new(
            ErrorKind::WriteZero,
            "short fd-message send",
        ));
    }
    Ok(())
}

/// Receive a payload plus at most one file descriptor in a single
/// `recvmsg`. Returns the received bytes (up to `max`, capped at
/// [`MAX_FD_MSG`]) and the fd, or `None` when the sender attached none.
///
/// Fail-closed: a truncated control message (`MSG_CTRUNC` - the kernel
/// dropped an fd for lack of buffer) or more than one attached fd is an
/// error, and every descriptor received on such an error path is closed
/// (dropping the `OwnedFd`s) rather than leaked. A zero-byte read (peer
/// hangup) is an error too. Received fds are `CLOEXEC` (`MSG_CMSG_CLOEXEC`).
pub fn recv_fd_msg<S: AsRawFd>(sock: &S, max: usize) -> io::Result<(Vec<u8>, Option<OwnedFd>)> {
    let cap = max.min(MAX_FD_MSG);
    let mut buf = vec![0u8; cap];
    let mut iov = libc::iovec {
        iov_base: buf.as_mut_ptr() as *mut libc::c_void,
        iov_len: buf.len(),
    };

    // SAFETY: CMSG_SPACE is a pure size computation.
    let cmsg_space = unsafe { libc::CMSG_SPACE(size_of::<libc::c_int>() as u32) } as usize;
    let mut cmsg_buf = vec![0u8; cmsg_space];

    // SAFETY: zeroed POD msghdr, public fields set to live buffers.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cmsg_buf.as_mut_ptr() as *mut libc::c_void;
    msg.msg_controllen = cmsg_space as _;

    // SAFETY: msg is initialised and points at live buffers.
    let n = unsafe { libc::recvmsg(sock.as_raw_fd(), &mut msg, libc::MSG_CMSG_CLOEXEC) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }

    // Collect every received fd first so the error paths below close them
    // (an OwnedFd closes on drop) instead of leaking.
    let mut fds: Vec<OwnedFd> = Vec::new();
    // SAFETY: we walk the kernel-filled control buffer with the CMSG_*
    // iterators; each SCM_RIGHTS datum is a c_int fd the kernel installed.
    unsafe {
        let mut cmsg = libc::CMSG_FIRSTHDR(&msg);
        while !cmsg.is_null() {
            if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
                let hdr = libc::CMSG_LEN(0) as usize;
                let payload_len = (*cmsg).cmsg_len as usize - hdr;
                let count = payload_len / size_of::<libc::c_int>();
                let data = libc::CMSG_DATA(cmsg) as *const libc::c_int;
                for i in 0..count {
                    fds.push(OwnedFd::from_raw_fd(*data.add(i)));
                }
            }
            cmsg = libc::CMSG_NXTHDR(&msg, cmsg);
        }
    }

    // Fail-closed conditions (fds already collected, so they close on the
    // early return via Vec drop).
    if msg.msg_flags & libc::MSG_CTRUNC != 0 {
        return Err(io::Error::other("fd-message control truncated"));
    }
    if fds.len() > 1 {
        return Err(io::Error::other("more than one fd received"));
    }
    if n == 0 {
        return Err(io::Error::new(ErrorKind::UnexpectedEof, "peer closed"));
    }

    let fd = fds.into_iter().next();
    buf.truncate(n as usize);
    Ok((buf, fd))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::fd::AsFd;
    use std::os::unix::net::UnixStream;

    /// Round-trip: a payload plus a pipe read-end travels over a
    /// socketpair, and the received fd is the SAME pipe (data written to
    /// the original write-end is read through the received fd).
    #[test]
    fn sends_and_receives_a_payload_and_fd() {
        let (tx, rx) = UnixStream::pair().unwrap();

        // A pipe whose read-end we pass across.
        let mut fds = [0i32; 2];
        // SAFETY: fds is a valid 2-int array for pipe(2).
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        // SAFETY: pipe returned two fresh owned fds.
        let read_end = unsafe { OwnedFd::from_raw_fd(fds[0]) };
        let mut write_end = unsafe { std::fs::File::from_raw_fd(fds[1]) };

        send_fd_msg(&tx, b"hello", read_end.as_fd()).unwrap();
        let (payload, got) = recv_fd_msg(&rx, 64).unwrap();
        assert_eq!(payload, b"hello");
        let got = got.expect("an fd was sent");

        // Prove the received fd is the same pipe.
        write_end.write_all(b"X").unwrap();
        let mut received = std::fs::File::from(got);
        let mut byte = [0u8; 1];
        received.read_exact(&mut byte).unwrap();
        assert_eq!(&byte, b"X");
    }

    /// An empty or oversized payload is refused (the fd is never sent).
    #[test]
    fn rejects_a_bad_payload_size() {
        let (tx, _rx) = UnixStream::pair().unwrap();
        let self_fd = tx.as_fd();
        assert!(send_fd_msg(&tx, b"", self_fd).is_err());
        let big = vec![0u8; MAX_FD_MSG + 1];
        assert!(send_fd_msg(&tx, &big, self_fd).is_err());
    }

    /// A message sent with NO ancillary fd (a plain send) is received as
    /// `(payload, None)` - the caller then refuses, since its op requires
    /// a pidfd. Proves the no-fd path is `None`, not a fabricated fd.
    #[test]
    fn a_message_without_an_fd_yields_none() {
        let (mut tx, rx) = UnixStream::pair().unwrap();
        tx.write_all(b"nofd").unwrap();
        let (payload, got) = recv_fd_msg(&rx, 64).unwrap();
        assert_eq!(payload, b"nofd");
        assert!(got.is_none());
    }
}
