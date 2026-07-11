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
use crate::mint::MintParams;
use crate::revocation::CapsuleListEntry;
use crate::slice::FrozenSlice;

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

    /// Mint a capsule from an already-materialized frozen slice. Returns the new
    /// capsule's revocation handle and the slice content hash. Fails closed if the
    /// caller is not a mint-admitted (human-UI) peer (the daemon withholds the signing
    /// key), surfaced as the daemon's coarse `Error` reply.
    pub fn mint(&self, slice: FrozenSlice, params: MintParams) -> io::Result<MintReceipt> {
        map_mint(self.round_trip(&ControlRequest::Mint { slice, params })?)
    }
}

/// The result of a successful mint: the revocation handle the owner keeps to revoke
/// the capsule, and the slice content hash (the capsule identity).
pub struct MintReceipt {
    /// The revocation handle for a later revoke.
    pub handle: String,
    /// The slice content hash (hex), the capsule's identity.
    pub slice_hash: String,
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

/// Interpret a reply to `Mint`.
fn map_mint(resp: ControlResponse) -> io::Result<MintReceipt> {
    match resp {
        ControlResponse::Minted { handle, slice_hash } => Ok(MintReceipt { handle, slice_hash }),
        ControlResponse::Error(e) => Err(io::Error::other(e)),
        other => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unexpected reply to mint: {other:?}"),
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
                meta: None,
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
        assert!(map_mint(ControlResponse::Error("mint not available".into())).is_err());
        // A wrong-variant reply is also an error, not a silent success.
        assert!(map_revoke(ControlResponse::Capsules(vec![])).is_err());
        assert!(map_revoke(ControlResponse::Revoked).is_ok());
        assert!(map_mint(ControlResponse::Revoked).is_err());
    }

    #[test]
    fn a_minted_reply_maps_to_the_receipt() {
        let r = map_mint(ControlResponse::Minted {
            handle: "h-9".into(),
            slice_hash: "abcd".into(),
        })
        .unwrap();
        assert_eq!(r.handle, "h-9");
        assert_eq!(r.slice_hash, "abcd");
    }

    // The sync client's `mint` drives the async serve loop over a real bound socket:
    // the harness materializes a slice and hands it to `capsuled` to sign + register.
    // This proves the cross-runtime wire path (std client <-> tokio serve, identical
    // 4-byte BE length framing) end to end, not just the reply mapping.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_sync_client_mints_over_a_real_control_socket() {
        use crate::control_server::serve_control_connection;
        use crate::mint::MintParams;
        use crate::revocation::RevocationFile;
        use crate::scope::CapsuleScope;
        use crate::slice::{FrozenSlice, SliceNode, SliceValue};
        use arlen_forage_store::Store;
        use ed25519_dalek::SigningKey;
        use std::collections::BTreeMap;
        use std::sync::atomic::{AtomicU64, Ordering};
        use tokio::net::UnixListener;

        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("capsule-ctl-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("control.sock");
        let store = Store::open(dir.join("store")).unwrap();
        let ledger = RevocationFile::open(dir.join("ledger")).unwrap();
        let key = SigningKey::from_bytes(&[7u8; 32]);

        let listener = UnixListener::bind(&sock).unwrap();
        // Serve exactly one connection (the mint), with the key present (the admission
        // gate is tested separately; here the wire round-trip is under test).
        let srv = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            serve_control_connection(stream, &ledger, &store, Some(&key)).await.unwrap();
        });

        let mut fields = BTreeMap::new();
        fields.insert("path".to_string(), SliceValue::Text("/a".to_string()));
        let slice = FrozenSlice {
            nodes: vec![SliceNode { id: "f1".into(), label: "File".into(), fields }],
            relations: vec![],
        };
        let params = MintParams {
            scope: CapsuleScope { roots: vec!["p1".into()], expand_hops: 1 },
            audience_hex: "00".repeat(32),
            expires_at_micros: i64::MAX,
            max_ops: 5,
            originating_user: "tim".into(),
            label: "Reading list".into(),
            scope_summary: "1 file in this project (FILE_PART_OF)".into(),
        };

        let sock_path = sock.clone();
        let receipt = tokio::task::spawn_blocking(move || {
            CapsuleControlClient::new(sock_path).mint(slice, params)
        })
        .await
        .unwrap()
        .expect("mint over the real socket succeeds");

        srv.await.unwrap();
        assert!(!receipt.handle.is_empty());
        assert!(!receipt.slice_hash.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
