//! The broker wire protocol + the pure request dispatch.
//!
//! A caller reads the current AI master switches ([`Request::Get`])
//! or replaces them ([`Request::Set`]). `Set` is the privileged op:
//! only an ADMITTED writer (the apps that legitimately own these
//! settings - `settings`, `ai-daemon`, `ai-agent`) may mutate the
//! canonical state. The caller's app id is resolved by the socket
//! layer from the `SO_PEERPIDFD`-pinned pid
//! ([`arlen_permissions::peer_pidfd`]); this dispatch is pure over
//! `(store, caller_app_id, request)` so the gate is unit-testable
//! without a socket.
//!
//! Frames are length-prefixed JSON (4-byte big-endian length +
//! body), capped at [`MAX_FRAME`] before allocation.

use std::io::{Read, Write};

use serde::{Deserialize, Serialize};

use crate::state::{AiMasterSwitches, StateStore};

/// The largest accepted frame body. The state is a handful of small
/// fields + a bounded app-id set, so 64 KiB is generous; a larger
/// declared length is refused before allocating.
pub const MAX_FRAME: usize = 64 * 1024;

/// The apps allowed to mutate the AI master switches. `settings`
/// writes the user-facing defaults (`enabled` / `access_level` /
/// `provider`); the AI daemon + agent write the live selection +
/// autonomy (`action_mode` / `autonomous_apps` / `executor_live` /
/// the provider live-switch). Every other same-uid caller may read
/// but not write.
///
/// app-id binding is by the resolved binary path (the `SO_PEERPIDFD`
/// pid -> `/proc/<pid>/exe`); a fully same-uid attacker that re-execs
/// an admitted binary is the documented residual that only the
/// SO_PEERSEC/MAC label tier (`same-uid-isolation-plan.md` Tier-A #4)
/// closes - this allowlist is the structure that tier slots into.
const ADMITTED_WRITERS: &[&str] = &["settings", "ai-daemon", "ai-agent"];

/// True iff `app_id` may mutate the master switches. In a debug
/// build the `dev.arlen-*` cargo-run ids of the admitted apps also
/// pass (the resolver yields `dev.<bin>` for an unpackaged binary),
/// matching the knowledge daemon's revoke-admit convention; in a
/// release build only the canonical ids.
pub fn is_admitted_writer(app_id: &str) -> bool {
    ADMITTED_WRITERS.contains(&app_id)
        || (cfg!(debug_assertions)
            && matches!(
                app_id,
                "dev.arlen-settings" | "dev.arlen-ai-daemon" | "dev.arlen-ai-agent"
            ))
}

/// A request to the broker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    /// Read the current master-switch state.
    Get,
    /// Replace the master-switch state (privileged - admitted
    /// writers only).
    Set(AiMasterSwitches),
}

/// The broker's reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Response {
    /// The current state (reply to `Get`).
    State(AiMasterSwitches),
    /// A `Set` was applied + persisted.
    Committed,
    /// A `Set` from a non-admitted caller; nothing was written.
    Refused(String),
    /// The store could not be read or written; the caller must NOT
    /// proceed on a guessed state.
    Error(String),
}

/// Dispatch one request against the store for an authenticated
/// caller. Pure: the socket layer supplies `caller_app_id` (resolved
/// from the pidfd-pinned pid) and frames the result.
pub fn handle_request(
    store: &StateStore,
    caller_app_id: &str,
    request: Request,
) -> Response {
    match request {
        Request::Get => match store.load() {
            Ok(state) => Response::State(state),
            // A corrupt store is an error, not a fabricated default:
            // the caller refuses rather than acting on a guess.
            Err(e) => Response::Error(e.to_string()),
        },
        Request::Set(switches) => {
            if !is_admitted_writer(caller_app_id) {
                return Response::Refused(format!(
                    "caller '{caller_app_id}' may not set the AI master switches"
                ));
            }
            // `store` clamps before persisting; the explicit sanitise
            // here keeps the gate honest even if that changes.
            match store.store(&switches.sanitised()) {
                Ok(()) => Response::Committed,
                Err(e) => Response::Error(e.to_string()),
            }
        }
    }
}

/// Read a length-prefixed JSON frame. Refuses a declared length
/// above [`MAX_FRAME`] before allocating.
pub fn read_frame<R: Read, T: for<'de> Deserialize<'de>>(reader: &mut R) -> std::io::Result<T> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame exceeds MAX_FRAME",
        ));
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Write a length-prefixed JSON frame. Refuses a body above
/// [`MAX_FRAME`].
pub fn write_frame<W: Write, T: Serialize>(writer: &mut W, value: &T) -> std::io::Result<()> {
    let body = serde_json::to_vec(value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    if body.len() > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame exceeds MAX_FRAME",
        ));
    }
    writer.write_all(&(body.len() as u32).to_be_bytes())?;
    writer.write_all(&body)?;
    writer.flush()
}

/// Async read of a length-prefixed JSON frame (the server + client
/// share this over their tokio streams). Refuses an oversized
/// declared length before allocating.
pub async fn read_frame_async<R, T>(reader: &mut R) -> std::io::Result<T>
where
    R: tokio::io::AsyncRead + Unpin,
    T: for<'de> Deserialize<'de>,
{
    use tokio::io::AsyncReadExt;
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame exceeds MAX_FRAME",
        ));
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await?;
    serde_json::from_slice(&body)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Async write of a length-prefixed JSON frame.
pub async fn write_frame_async<W, T>(writer: &mut W, value: &T) -> std::io::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
    T: Serialize,
{
    use tokio::io::AsyncWriteExt;
    let body = serde_json::to_vec(value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    if body.len() > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame exceeds MAX_FRAME",
        ));
    }
    writer.write_all(&(body.len() as u32).to_be_bytes()).await?;
    writer.write_all(&body).await?;
    writer.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ActionMode;
    use std::collections::BTreeSet;

    fn store(dir: &std::path::Path) -> StateStore {
        StateStore::open(dir).expect("open")
    }

    #[test]
    fn get_returns_the_stored_state() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path());
        let want = AiMasterSwitches {
            enabled: true,
            access_level: 3,
            ..Default::default()
        };
        s.store(&want).unwrap();
        match handle_request(&s, "anyone", Request::Get) {
            Response::State(got) => assert_eq!(got, want),
            other => panic!("expected State, got {other:?}"),
        }
    }

    #[test]
    fn an_admitted_writer_commits_and_persists() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path());
        let mut want = AiMasterSwitches {
            enabled: true,
            action_mode: ActionMode::Supervised,
            ..Default::default()
        };
        want.autonomous_apps.insert("org.arlen.files".to_string());
        assert_eq!(
            handle_request(&s, "settings", Request::Set(want.clone())),
            Response::Committed
        );
        // a fresh store sees the persisted state
        assert_eq!(store(tmp.path()).load().unwrap(), want);
    }

    #[test]
    fn a_non_admitted_writer_is_refused_and_nothing_is_written() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path());
        let hostile = AiMasterSwitches {
            executor_live: true,
            access_level: 4,
            ..Default::default()
        };
        match handle_request(&s, "org.evil.app", Request::Set(hostile)) {
            Response::Refused(_) => {}
            other => panic!("expected Refused, got {other:?}"),
        }
        // the store stayed at the floor - the hostile set never landed
        assert_eq!(s.load().unwrap(), AiMasterSwitches::default());
    }

    #[test]
    fn a_set_clamps_an_out_of_range_access_level_before_persisting() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path());
        let bad = AiMasterSwitches {
            access_level: 9,
            ..Default::default()
        };
        assert_eq!(
            handle_request(&s, "ai-daemon", Request::Set(bad)),
            Response::Committed
        );
        assert_eq!(s.load().unwrap().access_level, 0);
    }

    #[test]
    fn frames_round_trip() {
        let req = Request::Set(AiMasterSwitches {
            enabled: true,
            autonomous_apps: BTreeSet::from(["a".to_string(), "b".to_string()]),
            ..Default::default()
        });
        let mut buf = Vec::new();
        write_frame(&mut buf, &req).unwrap();
        let got: Request = read_frame(&mut buf.as_slice()).unwrap();
        assert_eq!(got, req);
    }

    #[test]
    fn an_oversized_declared_frame_is_refused_before_allocating() {
        let mut framed = (MAX_FRAME as u32 + 1).to_be_bytes().to_vec();
        framed.extend_from_slice(b"{}");
        let r: std::io::Result<Request> = read_frame(&mut framed.as_slice());
        assert!(r.is_err());
    }

    #[test]
    fn debug_dev_ids_are_admitted_only_in_debug() {
        assert!(is_admitted_writer("settings"));
        assert!(is_admitted_writer("ai-agent"));
        assert!(!is_admitted_writer("org.evil.app"));
        assert_eq!(is_admitted_writer("dev.arlen-settings"), cfg!(debug_assertions));
    }
}
