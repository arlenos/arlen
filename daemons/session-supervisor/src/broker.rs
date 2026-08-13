//! The identity-broker seam, over the same client every other registrar uses.
//!
//! The supervisor's whole reason to exist is this call: a user unit's identity
//! cannot be read from its `/proc/<pid>/exe` once the unit is hardened, so
//! something outside has to say what it is while the pid is known. The launcher
//! does that for apps at spawn; nothing did it for units systemd starts, and this
//! is that something.
//!
//! `registrar: false` on every registration, and the broker would refuse it
//! otherwise: the supervisor may register because the session root registered IT,
//! and only what the root started directly may pass that right on. A supervised
//! daemon gets an identity and no power to hand identities out.

use std::os::fd::AsFd;
use std::path::PathBuf;

use crate::supervise::Registrar;

/// A registrar that stamps supervised units at the session's identity broker.
pub struct BrokerRegistrar {
    socket: PathBuf,
}

impl BrokerRegistrar {
    /// Point at wherever the broker is listening for this session.
    pub fn at_default_socket() -> Self {
        Self {
            socket: arlen_permissions::identity_wire::identity_broker_connect_path(),
        }
    }
}

impl Registrar for BrokerRegistrar {
    fn register(&self, pid: u32, app_id: &str) -> Result<(), String> {
        // A pidfd rather than the bare pid, so the registration cannot land on a
        // recycled pid if the unit dies between the MainPID read and this call.
        // The broker holds the fd open for the record's life, which is what makes
        // a later lookup race-free too.
        let pidfd = arlen_permissions::peer_pidfd::pidfd_open(pid)
            .ok_or_else(|| format!("pid {pid} was gone before it could be registered"))?;
        arlen_permissions::identity_wire::register_identity(
            &self.socket,
            pidfd.as_fd(),
            app_id,
            // Nothing onward: a supervised daemon gets an identity and no
            // authority. The rights this supervisor holds were granted to IT.
            arlen_permissions::identity_store::Grants::default(),
        )
        .map_err(|e| format!("registering {app_id} (pid {pid}): {e}"))
    }
}
