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
//! reached, and so are [`sighting`], [`home_anchor`], [`epoch`] and [`ble_scan`]:
//! `arlen-sentineld` keeps tags and the learned resting place between sessions in
//! its encrypted store, stamps each sighting with the awake-session it fell in,
//! prunes the tags on start, and since 13 September registers a real BlueZ
//! advertisement monitor from [`ble_scan`]'s patterns and drives the whole chain
//! from what it finds. [`recording`] joined it the same day - its own monitor, its
//! own switch, and no history kept, because a stranger's device near you is not
//! your context. **[`usb`] has no caller anywhere**: that detector is USBGuard's
//! hands and this machine has no USBGuard on it yet.
//!
//! That is deliberate about the order rather than an oversight, and saying which is
//! the point of this note. The cores were written first because they are the half
//! that can be proven on a laptop with no radio in it. What SEN-4 still waits on is
//! `tracker-sentinel-plan.md` §6 job 3's substrate - a BlueZ `AdvertisementMonitor`
//! and a live adapter. The classifier, the geometry, the accumulation, the criteria,
//! the coarse location feed and the sealed store are all there and joined in
//! `arlen-sentineld`'s run loop; nothing hands that loop a real advertisement.
//!
//! **Keep this list true.** A detector that gains a caller comes out of the second
//! paragraph, and one that is added goes in. A pure core is worth having early only
//! while somebody can still tell, without measuring, which ones are waiting.

pub mod ble_scan;
pub mod epoch;
pub mod exposure;
pub mod home_anchor;
pub mod movement;
pub mod readout;
pub mod sighting;
pub mod recording;
pub mod tracker;
pub mod trigger;
pub mod usb;
