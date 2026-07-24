//! The Physical-World Privacy Sentinel's pure detector logic.
//!
//! The sentinel daemon (`org.arlen.Sentinel1`) drives the BLE/network hardware and
//! the consent/audit boundary; this crate holds the DETERMINISTIC, side-effect-free
//! detector cores it calls, so they are unit-testable without a radio. The first is
//! SEN-4's per-brand BLE finder-tag classification (`tracker`).

pub mod exposure;
pub mod tracker;
pub mod trigger;
