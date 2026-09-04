//! What a caller asks the sentinel, and what it answers.
//!
//! One request per connection, JSON, the shape every other Arlen broker socket
//! uses. Settings is the only caller today and reaches it through its Tauri host.
//!
//! THE READOUT CARRIES KEYS, NOT SENTENCES. A posture line names a surface and
//! what was measured there; the app turns that pair into a sentence in the
//! reader's language. A daemon that returned English would be shipping a string
//! no locale can reach, which is the whole reason the born-translatable lint
//! exists, and a privacy page is the last surface that should be readable only
//! in one language.

use arlen_sentinel_detect::exposure::Posture;
use arlen_sentinel_detect::readout::{Line, Surface};
use serde::{Deserialize, Serialize};

use crate::config::{Alerts, Config, Detector, DetectorConfig};

/// What a caller asks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// Everything the Settings page renders.
    GetState,
    /// Turn a detector on or off.
    SetDetector { id: String, on: bool },
    /// Change how a detector speaks up.
    SetAlerts { id: String, mode: String },
    /// Change a watcher's sensitivity.
    SetSensitivity { id: String, level: String },
    /// Apply the one-click remediation behind a posture line.
    ///
    /// Addressed by SURFACE rather than by the line's index in the last readout.
    /// An index means the caller and the daemon have to agree about a list that
    /// is recomputed on every read, and a radio that changed state between the
    /// read and the tap would silently point the fix at the neighbouring line.
    /// A surface names what to act on, so a stale request fixes the thing it said
    /// or nothing at all.
    FixPosture { surface: String },
}

/// What comes back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Response {
    /// The state the page renders.
    ///
    /// Boxed: it is by far the largest variant and every other one is a word or
    /// a sentence, so an unboxed enum would make each `Done` as big as a whole
    /// readout.
    State(Box<State>),
    /// The change was made.
    Done,
    /// The change was refused by a rule, with the sentence to show.
    Refused { message: String },
    /// Something failed underneath.
    Failed { message: String },
}

/// One detector's switches, on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectorWire {
    pub on: bool,
    pub alerts: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sensitivity: Option<String>,
}

impl From<&DetectorConfig> for DetectorWire {
    fn from(c: &DetectorConfig) -> Self {
        DetectorWire {
            on: c.on,
            alerts: c.alerts.as_str().to_string(),
            sensitivity: c.sensitivity.clone(),
        }
    }
}

/// One line of the exposure readout, on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostureWire {
    /// Which surface this is about, as a key the app has a sentence for.
    pub surface: String,
    /// What was measured: `exposed`, `protected` or `unknown`.
    pub posture: String,
    /// Whether a one-click fix is offered for it as it stands.
    pub fix: bool,
}

/// The whole state the page renders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub detectors: Detectors,
    pub posture: Vec<PostureWire>,
    /// Whether anything is using the microphone or camera right now, or absent
    /// when nothing can answer that.
    ///
    /// An `Option` rather than a `bool` because the difference matters more here
    /// than anywhere else on the page. "Nothing is using your microphone" is what
    /// a person opens this page to find out, and there is no microphone or camera
    /// portal in this build to ask, so a `false` would be a claim nobody
    /// measured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_active: Option<bool>,
    /// Whether the tracker still holds the coarse location grant it needs.
    pub tracker_has_location: bool,
    /// Set when the exposure detector could not read a surface at all, so the
    /// page can say the readout is incomplete rather than presenting what it
    /// managed to measure as the whole picture.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub posture_incomplete: Option<bool>,
}

/// The four configurable detectors, named rather than a map, so a missing one is
/// a compile error on both sides instead of an empty card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Detectors {
    pub exposure: DetectorWire,
    pub usb: DetectorWire,
    pub recording: DetectorWire,
    pub tracker: DetectorWire,
}

impl From<&Config> for Detectors {
    fn from(c: &Config) -> Self {
        Detectors {
            exposure: (&c.exposure).into(),
            usb: (&c.usb).into(),
            recording: (&c.recording).into(),
            tracker: (&c.tracker).into(),
        }
    }
}

/// The key the app matches on to pick a sentence for a surface.
pub fn surface_key(s: Surface) -> &'static str {
    match s {
        Surface::WifiMac => "wifi_mac",
        Surface::SavedMacPolicy => "saved_mac_policy",
        Surface::HiddenNetwork => "hidden_network",
        Surface::DhcpHostname => "dhcp_hostname",
        Surface::BluetoothDiscoverable => "bluetooth_discoverable",
        Surface::BlerPrivacy => "ble_privacy",
    }
}

/// Parse a surface key back, for a fix request naming one.
pub fn parse_surface(key: &str) -> Option<Surface> {
    Surface::ALL.into_iter().find(|s| surface_key(*s) == key)
}

/// The key for a posture.
pub fn posture_key(p: Posture) -> &'static str {
    match p {
        Posture::Exposed => "exposed",
        Posture::Protected => "protected",
        Posture::Unknown => "unknown",
    }
}

/// The readout, on the wire.
pub fn posture_wire(lines: &[Line]) -> Vec<PostureWire> {
    lines
        .iter()
        .map(|l| PostureWire {
            surface: surface_key(l.surface).to_string(),
            posture: posture_key(l.posture).to_string(),
            fix: l.fixable,
        })
        .collect()
}

/// Whether any line is a surface nobody could read.
pub fn readout_incomplete(lines: &[Line]) -> bool {
    lines.iter().any(|l| l.posture == Posture::Unknown)
}

/// Which detector a request names, without acting on it.
pub fn requested_detector(r: &Request) -> Option<Detector> {
    match r {
        Request::SetDetector { id, .. }
        | Request::SetAlerts { id, .. }
        | Request::SetSensitivity { id, .. } => Detector::parse(id),
        Request::GetState | Request::FixPosture { .. } => None,
    }
}

/// Whether a request changes anything, which is what decides if it needs a write.
pub fn is_write(r: &Request) -> bool {
    !matches!(r, Request::GetState)
}

/// The alert mode a request asks for, if it is one.
pub fn requested_alerts(r: &Request) -> Option<Alerts> {
    match r {
        Request::SetAlerts { mode, .. } => Alerts::parse(mode),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_surface_has_a_key_and_it_parses_back() {
        for s in Surface::ALL {
            assert_eq!(parse_surface(surface_key(s)), Some(s));
        }
    }

    #[test]
    fn a_key_nothing_knows_is_refused_rather_than_matched() {
        assert_eq!(parse_surface("wifi"), None);
        assert_eq!(parse_surface(""), None);
    }

    #[test]
    fn a_surface_nobody_could_read_still_makes_a_line() {
        let lines = [Line {
            surface: Surface::BluetoothDiscoverable,
            posture: Posture::Unknown,
            fixable: false,
        }];
        let wire = posture_wire(&lines);
        assert_eq!(wire[0].posture, "unknown");
        assert!(readout_incomplete(&lines), "the page is told it is partial");
    }

    #[test]
    fn a_fully_measured_readout_is_not_incomplete() {
        let lines = [Line {
            surface: Surface::WifiMac,
            posture: Posture::Protected,
            fixable: false,
        }];
        assert!(!readout_incomplete(&lines));
    }

    #[test]
    fn the_request_shape_is_the_one_the_surface_sends() {
        let r: Request =
            serde_json::from_str(r#"{"op":"set_detector","id":"tracker","on":true}"#).unwrap();
        assert_eq!(
            r,
            Request::SetDetector {
                id: "tracker".into(),
                on: true
            }
        );
        assert_eq!(requested_detector(&r), Some(Detector::Tracker));
        assert!(is_write(&r));
    }

    #[test]
    fn a_read_is_not_a_write() {
        assert!(!is_write(&Request::GetState));
    }

    #[test]
    fn the_state_serialises_in_the_shape_the_store_declares() {
        let cfg = Config::default();
        let state = State {
            detectors: (&cfg).into(),
            posture: posture_wire(&[Line {
                surface: Surface::BluetoothDiscoverable,
                posture: Posture::Exposed,
                fixable: true,
            }]),
            capture_active: None,
            tracker_has_location: false,
            posture_incomplete: None,
        };
        let json = serde_json::to_string(&state).unwrap();
        assert!(!json.contains("captureActive"), "unmeasured is absent, not false: {json}");
        assert!(json.contains("\"trackerHasLocation\":false"), "{json}");
        assert!(json.contains("\"surface\":\"bluetooth_discoverable\""), "{json}");
        assert!(!json.contains("postureIncomplete"), "absent when complete");
    }

    #[test]
    fn a_refusal_travels_with_its_sentence() {
        let r = Response::Refused {
            message: "nope".into(),
        };
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<Response>(&json).unwrap(), r);
    }
}
