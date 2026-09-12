//! The Physical-World Privacy Sentinel's pure detector logic.
//!
//! The sentinel daemon (`org.arlen.Sentinel1`) drives the BLE/network hardware and
//! the consent/audit boundary; this crate holds the DETERMINISTIC, side-effect-free
//! detector cores it calls, so they are unit-testable without a radio.
//!
//! ## Which detectors have a driver, measured 13 September 2026
//!
//! Two of the seven are reached: `sentineld` calls [`exposure`] and [`readout`] to
//! turn a reading into a posture and compose the line a person reads. **Nothing in
//! the tree calls [`movement`], [`recording`], [`tracker`], [`trigger`] or
//! [`usb`]** - together about 840 lines with fixtures and tests, and no caller.
//!
//! That is deliberate about the order rather than an oversight, and saying which is
//! the point of this note. The cores were written first because they are the half
//! that can be proven on a laptop with no radio in it; what they wait on is
//! `tracker-sentinel-plan.md` §6 jobs 1 and 2 - the GeoClue2 portal bridge that
//! feeds [`movement`] its fixes, and the encrypted sighting store under
//! `~/.local/state/arlen/tracker-sentinel/` that [`trigger`] counts over. Neither
//! exists, so there is nothing for the daemon to drive them with yet.
//!
//! **Keep this list true.** A detector that gains a caller comes out of the second
//! paragraph, and one that is added goes in. A pure core is worth having early only
//! while somebody can still tell, without measuring, which ones are waiting.

pub mod exposure;
pub mod movement;
pub mod readout;
pub mod recording;
pub mod tracker;
pub mod trigger;
pub mod usb;
