//! Trust-on-first-use for `.lenv` publishers (profile-system-plan.md, "Trust =
//! TOFU by default ... enrollment link for managed onboarding").
//!
//! [`verify_signature`](crate::verify_signature) proves a `.lenv` was signed by
//! the key it carries; TOFU decides whether to TRUST that key. The first time a
//! publisher is seen its fingerprint is confirmed by the user and pinned; every
//! later package from that publisher must present the SAME key, or it is refused
//! (a different key for a known publisher is impersonation, never silently
//! accepted). The enrollment link is just pinning the org key ahead of the first
//! package, so the first install is already trusted with no prompt. This is the
//! pin store plus the verdict; the persistence path is the installer's.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::key_fingerprint;

/// The trust decision for a publisher's key against the pinned store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TofuVerdict {
    /// No pin yet: trust on first use - the user confirms the fingerprint, then
    /// it is pinned via [`Pins::pin`].
    NewPublisher,
    /// The presented key matches the pinned fingerprint: proceed.
    Trusted,
    /// A DIFFERENT key for a pinned publisher: refuse. Never re-pin to silence
    /// this - it is the impersonation/tamper signal TOFU exists to catch.
    KeyChanged,
}

/// The pinned publisher keys: publisher name to pinned key fingerprint.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Pins {
    #[serde(default)]
    pins: BTreeMap<String, String>,
}

impl Pins {
    /// An empty pin store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse a pins file (TOML).
    pub fn parse(toml_str: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(toml_str)
    }

    /// Serialize the pin store to TOML for persistence.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string(self)
    }

    /// The TOFU verdict for `publisher` presenting `verifying_key`.
    pub fn verdict(&self, publisher: &str, verifying_key: &[u8]) -> TofuVerdict {
        let fingerprint = key_fingerprint(verifying_key);
        match self.pins.get(publisher) {
            None => TofuVerdict::NewPublisher,
            Some(pinned) if *pinned == fingerprint => TofuVerdict::Trusted,
            Some(_) => TofuVerdict::KeyChanged,
        }
    }

    /// Pin `publisher`'s key - after a [`TofuVerdict::NewPublisher`] confirmation
    /// or via the enrollment link. Overwrites any existing pin, so callers must
    /// never call this to accept a [`TofuVerdict::KeyChanged`]; only a deliberate
    /// re-enrollment legitimately replaces a pin.
    pub fn pin(&mut self, publisher: &str, verifying_key: &[u8]) {
        self.pins
            .insert(publisher.to_string(), key_fingerprint(verifying_key));
    }

    /// Whether `publisher` already has a pinned key.
    pub fn is_pinned(&self, publisher: &str) -> bool {
        self.pins.contains_key(publisher)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Two distinct 32-byte keys.
    const KEY_A: [u8; 32] = [1u8; 32];
    const KEY_B: [u8; 32] = [2u8; 32];

    #[test]
    fn first_use_is_new_then_trusted_after_pinning() {
        let mut pins = Pins::new();
        assert_eq!(pins.verdict("Acme", &KEY_A), TofuVerdict::NewPublisher);
        assert!(
            !pins.is_pinned("Acme"),
            "an unpinned publisher must read as not pinned"
        );
        pins.pin("Acme", &KEY_A);
        assert!(pins.is_pinned("Acme"));
        assert_eq!(pins.verdict("Acme", &KEY_A), TofuVerdict::Trusted);
    }

    #[test]
    fn a_different_key_for_a_pinned_publisher_is_a_key_change() {
        let mut pins = Pins::new();
        pins.pin("Acme", &KEY_A);
        assert_eq!(pins.verdict("Acme", &KEY_B), TofuVerdict::KeyChanged);
    }

    #[test]
    fn an_enrolled_key_is_trusted_with_no_prompt() {
        // The enrollment link pins the org key before the first package.
        let mut pins = Pins::new();
        pins.pin("Acme", &KEY_A);
        assert_eq!(pins.verdict("Acme", &KEY_A), TofuVerdict::Trusted);
    }

    #[test]
    fn pins_round_trip_through_toml() {
        let mut pins = Pins::new();
        pins.pin("Acme", &KEY_A);
        pins.pin("Globex", &KEY_B);
        let toml = pins.to_toml().unwrap();
        let restored = Pins::parse(&toml).unwrap();
        assert_eq!(restored.verdict("Acme", &KEY_A), TofuVerdict::Trusted);
        assert_eq!(restored.verdict("Globex", &KEY_B), TofuVerdict::Trusted);
        assert_eq!(restored.verdict("Globex", &KEY_A), TofuVerdict::KeyChanged);
    }

    #[test]
    fn an_unknown_publisher_stays_new() {
        let mut pins = Pins::new();
        pins.pin("Acme", &KEY_A);
        assert_eq!(pins.verdict("Stranger", &KEY_A), TofuVerdict::NewPublisher);
    }
}
