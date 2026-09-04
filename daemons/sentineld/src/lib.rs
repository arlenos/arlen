//! The physical-world privacy sentinel (`privacy-sentinel-plan.md`).
//!
//! Five detectors, one session daemon, `org.arlen.Sentinel1`. This crate holds
//! the daemon's own logic and the protocol its callers speak; the comparisons
//! that turn a reading into a posture live in `arlen-sentinel-detect`, which
//! stays free of I/O so they can be tested without a radio.
//!
//! WHAT THE SURFACE IS WAITING FOR. `apps/settings/src/lib/stores/sentinel.ts`
//! has called `sentinel_get_state` since 26 August and got nothing back, and the
//! page above it was written to say a machine is protected. A privacy page that
//! reports a detector as on when nothing is watching is the one failure this
//! project has said out loud it will not ship, so the order here is: read the
//! real state first, and only then let anything be switched.

pub mod client;
pub mod config;
pub mod host;
pub mod protocol;
pub mod read;
pub mod server;
