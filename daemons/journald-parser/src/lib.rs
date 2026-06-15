//! `arlen-journald-parser` - the Tier-2 journal ingestion library.
//!
//! The systemd journal is the second of the three named ingestion tiers (the
//! eBPF kernel-layer is Tier-1). This crate carries the pure, fully-tested
//! [`classify`](classify::classify) core that turns a `journalctl --output=json`
//! line into a coarse, non-sensitive [`ServiceEvent`](classify::ServiceEvent);
//! the daemon binary (`main.rs`) is a thin host that follows the journal and
//! emits the classified events onto the event bus.

pub mod classify;
