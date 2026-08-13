//! The session supervisor: the party that asks systemd for the per-user Arlen
//! daemons, and therefore the party that can attest them.
//!
//! Identity in the user session cannot come from a unit's name - the user owns a
//! directory that outranks the shipped one, so a name is something they choose.
//! It comes from the registration, and a registration is only worth anything if
//! it is renewed when systemd replaces the process. That is this component's
//! whole job; see [`supervise`] for the decision and why one registration is not
//! enough.

pub mod broker;
pub mod supervise;
pub mod systemd;
