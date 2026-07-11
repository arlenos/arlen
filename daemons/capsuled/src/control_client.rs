//! A synchronous client to the `capsuled` owner control socket: list the active
//! capsules and revoke one by handle. One-shot per connection (connect, send one
//! request, read one reply), matching the consent-broker control-client shape, so a
//! Tauri caller drives it on a blocking thread. Framed the same way as the serve
//! loop (4-byte big-endian length prefix + JSON body).

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use crate::control::{ControlRequest, ControlResponse};
use crate::control_server::control_socket_path;
use crate::revocation::CapsuleListEntry;

/// The largest control reply accepted. The active-capsules list is bounded by the
/// number of a user's capsules; the cap guards against a hostile length.
const MAX_CONTROL_REPLY: usize = 4 * 1024 * 1024;

/// A synchronous one-shot client to the capsule owner control socket.
pub struct CapsuleControlClient {
    path: PathBuf,
}

impl CapsuleControlClient {
    /// A client for a specific control-socket path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// A client for the default control socket
    /// (`$XDG_RUNTIME_DIR/arlen/capsule-control.sock`).
    pub fn at_default_path() -> io::Result<Self> {
        control_socket_path().map(Self::new).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "no XDG_RUNTIME_DIR for the capsule control socket",
            )
        })
    }

    fn round_trip(&self, req: &ControlRequest) -> io::Result<ControlResponse> {
        let stream = UnixStream::connect(&self.path)?;
        round_trip_on(stream, req)
    }

    /// List the registered capsules (the active-capsules surface).
    pub fn list(&self) -> io::Result<Vec<CapsuleListEntry>> {
        map_list(self.round_trip(&ControlRequest::List)?)
    }

    /// Revoke a capsule by handle. Idempotent (revoking an unknown or already-revoked
    /// handle still succeeds).
    pub fn revoke(&self, handle: &str) -> io::Result<()> {
        map_revoke(self.round_trip(&ControlRequest::Revoke {
            handle: handle.to_string(),
        })?)
    }
}

/// Interpret a reply to `List`.
fn map_list(resp: ControlResponse) -> io::Result<Vec<CapsuleListEntry>> {
    match resp {
        ControlResponse::Capsules(list) => Ok(list),
        ControlResponse::Error(e) => Err(io::Error::other(e)),
        other => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unexpected reply to list: {other:?}"),
        )),
    }
}

/// Interpret a reply to `Revoke`.
fn map_revoke(resp: ControlResponse) -> io::Result<()> {
    match resp {
        ControlResponse::Revoked => Ok(()),
        ControlResponse::Error(e) => Err(io::Error::other(e)),
        other => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unexpected reply to revoke: {other:?}"),
        )),
    }
}

/// Send one framed request and read one framed reply over `stream` (4-byte BE length
/// prefix + JSON body, matching the serve framing). The reply length is bounded.
fn round_trip_on<S: Read + Write>(
    mut stream: S,
    req: &ControlRequest,
) -> io::Result<ControlResponse> {
    let body = serde_json::to_vec(req).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let len = u32::try_from(body.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "request too large"))?;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(&body)?;
    stream.flush()?;

    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let rlen = u32::from_be_bytes(len_buf) as usize;
    if rlen > MAX_CONTROL_REPLY {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "reply exceeds the maximum",
        ));
    }
    let mut resp = vec![0u8; rlen];
    stream.read_exact(&mut resp)?;
    serde_json::from_slice(&resp).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream as StdUnixStream;
    use std::thread;

    fn read_frame_sync(s: &mut impl Read) -> Vec<u8> {
        let mut len = [0u8; 4];
        s.read_exact(&mut len).unwrap();
        let mut b = vec![0u8; u32::from_be_bytes(len) as usize];
        s.read_exact(&mut b).unwrap();
        b
    }

    fn write_frame_sync(s: &mut impl Write, bytes: &[u8]) {
        s.write_all(&(bytes.len() as u32).to_be_bytes()).unwrap();
        s.write_all(bytes).unwrap();
    }

    #[test]
    fn a_list_request_round_trips_and_parses_the_capsules() {
        let (client, mut server) = StdUnixStream::pair().unwrap();
        let srv = thread::spawn(move || {
            let req = read_frame_sync(&mut server);
            assert_eq!(
                serde_json::from_slice::<ControlRequest>(&req).unwrap(),
                ControlRequest::List
            );
            let resp = ControlResponse::Capsules(vec![CapsuleListEntry {
                handle: "h-1".into(),
                revoked: false,
                ops_used: 2,
            }]);
            write_frame_sync(&mut server, &serde_json::to_vec(&resp).unwrap());
        });
        let resp = round_trip_on(client, &ControlRequest::List).unwrap();
        srv.join().unwrap();
        assert_eq!(map_list(resp).unwrap()[0].handle, "h-1");
    }

    #[test]
    fn an_error_reply_maps_to_err() {
        assert!(map_list(ControlResponse::Error("nope".into())).is_err());
        assert!(map_revoke(ControlResponse::Error("nope".into())).is_err());
        // A wrong-variant reply is also an error, not a silent success.
        assert!(map_revoke(ControlResponse::Capsules(vec![])).is_err());
        assert!(map_revoke(ControlResponse::Revoked).is_ok());
    }
}
