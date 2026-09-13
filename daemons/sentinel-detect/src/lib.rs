//! The Physical-World Privacy Sentinel's pure detector logic.
//!
//! The sentinel daemon (`org.arlen.Sentinel1`) drives the BLE/network hardware and
//! the consent/audit boundary; this crate holds the DETERMINISTIC, side-effect-free
//! detector cores it calls, so they are unit-testable without a radio.
//!
//! ## Which detectors have a driver, measured 13 September 2026
//!
//! `sentineld` calls [`exposure`] and [`readout`] to turn a reading into a posture
//! and compose the line a person reads. The SEN-4 chain is joined inside this crate:
//! [`sighting`] accumulates a tag's separated adverts and reduces them through
//! [`movement`] into the summary [`trigger`] decides on, so four of the eight are
//! reached. **[`recording`] and [`usb`] have no caller anywhere**, and neither does
//! [`sighting`] itself - it is the top of the SEN-4 chain and what the daemon will
//! call.
//!
//! That is deliberate about the order rather than an oversight, and saying which is
//! the point of this note. The cores were written first because they are the half
//! that can be proven on a laptop with no radio in it; what SEN-4 waits on is
//! `tracker-sentinel-plan.md` §6 jobs 1 and 2 - the GeoClue2 portal bridge that
//! feeds a sighting its fix, and the encrypted store under
//! `~/.local/state/arlen/tracker-sentinel/` that keeps the tags between sessions.
//! Neither exists, so there is nothing for the daemon to drive the chain with yet.
//!
//! **Keep this list true.** A detector that gains a caller comes out of the second
//! paragraph, and one that is added goes in. A pure core is worth having early only
//! while somebody can still tell, without measuring, which ones are waiting.

pub mod exposure;
pub mod movement;
pub mod readout;
pub mod sighting;
pub mod recording;
pub mod tracker;
pub mod trigger;
pub mod usb;
