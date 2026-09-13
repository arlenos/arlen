//! What the watches have seen lately, for the page and the shell badge
//! (`privacy-sentinel-plan.md` §7).
//!
//! The two BLE watches are independent tasks and the socket that answers the page
//! is a third. This is the only thing they share: the latest finding of each, with
//! the moment it was made.
//!
//! **A finding expires, and that is the whole reason this is not a plain field.**
//! §7 mounts the shell's privacy badge only while a warn condition HOLDS and
//! collapses it to nothing when clear, because a permanently-lit shield nags. A
//! camera that was near you an hour ago is not near you, so a reading carries its
//! timestamp and goes quiet on its own rather than waiting for something to come
//! and clear it. Nothing has to remember to.
//!
//! **It keeps the latest, not a history.** One slot per detector, overwritten. A
//! list of everything that passed would be the proximity log the whole detector
//! exists to warn people about, and it would be in memory on the page's read path
//! rather than in the sealed store.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a finding stays current.
///
/// Two minutes is a compromise with a reason on each side: shorter and a device
/// that adverts every 30 seconds would flicker in and out of the badge; longer and
/// the page would say something is nearby after its owner has walked off. The
/// monitor's own release timeout is 30 seconds, so this is four of those - long
/// enough that a missed advert does not clear it, short enough to be about now.
pub const FRESH_FOR: Duration = Duration::from_secs(120);

/// One finding and when it was made.
#[derive(Debug, Clone)]
struct Seen<T> {
    what: T,
    at: Instant,
}

/// The latest finding of each watch.
#[derive(Debug, Default)]
pub struct Live {
    recording: Mutex<Option<Seen<(String, String)>>>,
    tracker: Mutex<Option<Seen<(String, String)>>>,
}

impl Live {
    /// Record that a recording device of `class_label` is nearby.
    pub fn saw_recording(&self, class_label: &str, confidence: &str) {
        if let Ok(mut slot) = self.recording.lock() {
            *slot = Some(Seen {
                what: (class_label.to_string(), confidence.to_string()),
                at: Instant::now(),
            });
        }
    }

    /// Record a tracker verdict.
    pub fn saw_tracker(&self, brand: &str, verdict: &str) {
        if let Ok(mut slot) = self.tracker.lock() {
            *slot = Some(Seen {
                what: (brand.to_string(), verdict.to_string()),
                at: Instant::now(),
            });
        }
    }

    /// The recording class currently nearby, or `None` when the last one has gone
    /// stale.
    pub fn recording_nearby(&self) -> Option<(String, String)> {
        fresh(&self.recording)
    }

    /// The current tracker verdict, or `None` when the last one has gone stale.
    pub fn tracker_suspected(&self) -> Option<(String, String)> {
        fresh(&self.tracker)
    }
}

/// The slot's value if it is still current.
///
/// A poisoned lock answers `None`: a watch that panicked mid-write has left a
/// reading nobody should trust, and "nothing is nearby" is the safe direction for
/// a badge that would otherwise assert something about the room.
fn fresh(slot: &Mutex<Option<Seen<(String, String)>>>) -> Option<(String, String)> {
    let guard = slot.lock().ok()?;
    let seen = guard.as_ref()?;
    (seen.at.elapsed() < FRESH_FOR).then(|| seen.what.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_seen_is_nothing_reported() {
        let live = Live::default();
        assert!(live.recording_nearby().is_none());
        assert!(live.tracker_suspected().is_none());
    }

    #[test]
    fn the_latest_finding_is_the_one_reported() {
        let live = Live::default();
        live.saw_recording("Snap Spectacles", "medium");
        live.saw_recording("Meta smart glasses", "high");
        assert_eq!(
            live.recording_nearby(),
            Some(("Meta smart glasses".into(), "high".into())),
            "one slot, overwritten - not a history of who passed"
        );
    }

    /// The two watches do not overwrite each other.
    #[test]
    fn each_watch_has_its_own_slot() {
        let live = Live::default();
        live.saw_recording("Meta smart glasses", "high");
        live.saw_tracker("apple-find-my", "alert");
        assert!(live.recording_nearby().is_some());
        assert_eq!(live.tracker_suspected(), Some(("apple-find-my".into(), "alert".into())));
    }

    /// A finding goes quiet on its own. Faked by writing a stale timestamp, which
    /// is the one thing worth reaching inside the type for: the alternative is a
    /// two-minute test.
    #[test]
    fn a_stale_finding_is_not_reported() {
        let live = Live::default();
        live.saw_recording("Meta smart glasses", "high");
        {
            let mut slot = live.recording.lock().unwrap();
            let seen = slot.as_mut().unwrap();
            seen.at = Instant::now() - FRESH_FOR - Duration::from_secs(1);
        }
        assert!(live.recording_nearby().is_none(), "an hour-old camera is not nearby");
    }
}
