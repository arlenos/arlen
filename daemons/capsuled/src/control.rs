//! The owner-facing control protocol for `capsuled`: the management ops the
//! "share a slice" surface drives - list the active capsules and revoke one by
//! handle - distinct from the recipient grant-read serve loop (`server.rs`), where
//! a reader presents a signed grant to fetch a slice.
//!
//! These ops belong to the capsule OWNER (the harness / settings), not a recipient,
//! so they run over a separate control socket the owner connects to (SO_PEERCRED
//! same-uid admission, like the consent broker's control socket). Framed the same
//! length-prefixed JSON way as the serve loop. `mint` is a later op (it composes the
//! knowledge daemon's slice materialization with the local mint, and is a checked
//! human action never reachable by the agent), so it is not in this first cut.

use serde::{Deserialize, Serialize};

use crate::mint::MintParams;
use crate::revocation::CapsuleListEntry;
use crate::slice::FrozenSlice;

/// A control request from the capsule owner's surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlRequest {
    /// List the registered capsules (the active-capsules surface: handle + state).
    List,
    /// Revoke a capsule by its handle (the one-gesture revoke). Idempotent: revoking
    /// an unknown or already-revoked handle still leaves it terminally revoked.
    Revoke {
        /// The revocation handle to revoke.
        handle: String,
    },
    /// Mint a capsule from an already-materialized slice (the caller materialized it
    /// via the knowledge daemon's `0x07` op): the daemon signs the grant, stores the
    /// slice blob and registers the handle with its mint metadata. This is the
    /// deliberate human "share a slice" action; the serve side gates it so it is
    /// never reachable by the agent.
    Mint {
        /// The frozen slice to freeze into a capsule.
        slice: FrozenSlice,
        /// The mint terms + the human label/scope summary for the list surface.
        params: MintParams,
    },
}

impl ControlRequest {
    /// Reject a structurally invalid request before it reaches the ledger/mint: a
    /// revoke needs a non-empty handle; a mint needs a non-empty label.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            ControlRequest::List => Ok(()),
            ControlRequest::Revoke { handle } if handle.trim().is_empty() => {
                Err("revoke requires a non-empty handle".to_string())
            }
            ControlRequest::Revoke { .. } => Ok(()),
            ControlRequest::Mint { params, .. } if params.label.trim().is_empty() => {
                Err("mint requires a non-empty label".to_string())
            }
            ControlRequest::Mint { .. } => Ok(()),
        }
    }
}

/// The control reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlResponse {
    /// The registered capsules, for [`ControlRequest::List`].
    Capsules(Vec<CapsuleListEntry>),
    /// A [`ControlRequest::Revoke`] completed (idempotent).
    Revoked,
    /// A [`ControlRequest::Mint`] completed: the new capsule's revocation handle and
    /// the slice content hash (the capsule identity).
    Minted {
        /// The revocation handle the owner uses to revoke it.
        handle: String,
        /// The slice content hash (hex), the capsule's identity.
        slice_hash: String,
    },
    /// The request was rejected or failed; the message is a coarse reason.
    Error(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip_through_json() {
        for req in [
            ControlRequest::List,
            ControlRequest::Revoke { handle: "h-1".into() },
        ] {
            let json = serde_json::to_string(&req).unwrap();
            assert_eq!(serde_json::from_str::<ControlRequest>(&json).unwrap(), req);
        }
    }

    #[test]
    fn responses_round_trip_through_json() {
        let resp = ControlResponse::Capsules(vec![CapsuleListEntry {
            handle: "h-1".into(),
            revoked: false,
            ops_used: 3,
            meta: None,
        }]);
        let json = serde_json::to_string(&resp).unwrap();
        assert_eq!(serde_json::from_str::<ControlResponse>(&json).unwrap(), resp);
    }

    #[test]
    fn a_revoke_with_a_blank_handle_is_rejected() {
        assert!(ControlRequest::Revoke { handle: "  ".into() }.validate().is_err());
        assert!(ControlRequest::Revoke { handle: "h".into() }.validate().is_ok());
        assert!(ControlRequest::List.validate().is_ok());
    }
}
