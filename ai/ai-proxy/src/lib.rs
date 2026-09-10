//! Outbound proxy library for the Arlen AI layer.
//!
//! Foundation §8.4.6 requires that AI provider traffic exit the host
//! through a dedicated daemon. The proxy is the only component
//! permitted to make outbound HTTPS connections to AI provider
//! endpoints; the AI engine daemon reaches it over the session D-Bus.
//!
//! WHAT FENCES THE OTHER SIDE, since this said the wrong thing. It named
//! `ai-daemon` and `ai-agent` running with `PrivateNetwork=true`, and
//! `PrivateNetwork` appears in none of this tree's units - it cannot usefully,
//! because these are user units and a user unit gets no private netns of its
//! own (`arlen-ai-proxy.service` says the same about its own `IPAddress` lines,
//! which are intent rather than enforcement there). The fence that is real is
//! one line up from that: `arlen-ai-engine-daemon.service` sets
//! `RestrictAddressFamilies=AF_UNIX AF_NETLINK`, so the daemon and everything it
//! spawns cannot open an IP socket at all. Same containment, different
//! mechanism, and it is inherited by children rather than resting on a namespace.
//!
//! This crate exposes the policy core (allowlist + audit emission +
//! forwarder trait) as a reusable library so it can be unit-tested
//! without spinning up the full daemon. The binary in `main.rs`
//! plumbs the policy into a real reqwest-backed outbound layer plus
//! a zbus service.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod allowlist;
pub mod audit;
pub mod catalog;
pub mod connections_client;
pub mod forward;
pub mod models_dev;
pub mod peer_auth;
pub mod service;
pub mod sovereignty;
pub mod transcode;
pub mod usage;
