//! The sentinel's switches on disk: `~/.config/arlen/sentinel.toml`.
//!
//! Four configurable detectors (the mic and camera signal is status-only and
//! belongs to the capture infrastructure, so it has nothing to switch here). Each
//! carries whether it runs, whether it speaks up or stays quiet, and where it
//! means something a sensitivity.
//!
//! THE DEFAULTS ARE THE PLAN'S MATRIX, and which way a missing value falls is a
//! decision rather than a convenience. §6 puts exposure, USB and the capture
//! signal ON by default because they are deterministic and cost nothing, and the
//! two ambient watchers OFF because they listen to the world and a person opts
//! into that. So an absent file, an absent field or a value nobody recognises
//! reads as the default for THAT field: a hand-edited typo leaves the protections
//! running rather than quietly switching them off, and it cannot turn an opt-in
//! watcher on either, because the default there is off.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The detectors this file configures.
///
/// Mic and camera are deliberately absent: that signal is owned by
/// `capture-active-infra-plan.md` and the sentinel only subscribes to it, so
/// there is no switch here that could contradict the one that owns it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Detector {
    /// What this machine broadcasts about itself over Wi-Fi and Bluetooth.
    Exposure,
    /// A USB device that claims to be a keyboard as well as something else.
    Usb,
    /// A camera-bearing wearable advertising nearby.
    Recording,
    /// A tag that keeps turning up wherever you go.
    Tracker,
}

impl Detector {
    /// The name the config file and the wire both use.
    pub fn as_str(self) -> &'static str {
        match self {
            Detector::Exposure => "exposure",
            Detector::Usb => "usb",
            Detector::Recording => "recording",
            Detector::Tracker => "tracker",
        }
    }

    /// Parse the name the surface sends. `None` for anything else, so an unknown
    /// detector is refused rather than silently mapped onto a real one.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "exposure" => Some(Detector::Exposure),
            "usb" => Some(Detector::Usb),
            "recording" => Some(Detector::Recording),
            "tracker" => Some(Detector::Tracker),
            _ => None,
        }
    }

    /// Every detector, in the order the Settings page lays them out: the
    /// always-on three first, then the opt-in watchers.
    pub const ALL: [Detector; 4] = [
        Detector::Exposure,
        Detector::Usb,
        Detector::Recording,
        Detector::Tracker,
    ];
}

/// How a detector speaks when it finds something.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Alerts {
    /// It changes the indicator and says nothing.
    Quiet,
    /// It raises a notification.
    Notify,
}

impl Alerts {
    /// The name the config file and the wire both use.
    pub fn as_str(self) -> &'static str {
        match self {
            Alerts::Quiet => "quiet",
            Alerts::Notify => "notify",
        }
    }

    /// Parse a mode, `None` for anything else.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "quiet" => Some(Alerts::Quiet),
            "notify" => Some(Alerts::Notify),
            _ => None,
        }
    }
}

/// The sensitivity vocabulary the recording indicator uses.
///
/// Named by intent, never by distance. §4.4 is explicit that RSSI cannot be
/// honestly turned into meters: the path-loss exponent swings between 2 and 4,
/// transmit power varies per device, and a body between the two radios dominates
/// both. So the control asks how close something should be before it is worth
/// mentioning, and the dBm floor behind each stop is an implementation detail
/// that never reaches the copy.
pub const PROXIMITY: [&str; 3] = ["near", "room", "anywhere"];

/// The sensitivity vocabulary the tracker uses: how much corroboration it wants
/// before it says a tag is following you.
pub const STRICTNESS: [&str; 3] = ["cautious", "balanced", "strict"];

/// One detector's switches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectorConfig {
    /// Whether the detector runs at all.
    pub on: bool,
    /// Whether it speaks up or stays quiet.
    pub alerts: Alerts,
    /// Set only where sensitivity means something.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sensitivity: Option<String>,
}

/// The whole file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub exposure: DetectorConfig,
    pub usb: DetectorConfig,
    pub recording: DetectorConfig,
    pub tracker: DetectorConfig,
}

impl Default for Config {
    /// The plan's §6 matrix, and the reason each one falls the way it does is in
    /// the module header.
    fn default() -> Self {
        Config {
            exposure: DetectorConfig {
                on: true,
                alerts: Alerts::Quiet,
                sensitivity: None,
            },
            usb: DetectorConfig {
                on: true,
                alerts: Alerts::Notify,
                sensitivity: None,
            },
            recording: DetectorConfig {
                on: false,
                alerts: Alerts::Quiet,
                sensitivity: Some("room".to_string()),
            },
            tracker: DetectorConfig {
                on: false,
                alerts: Alerts::Notify,
                sensitivity: Some("balanced".to_string()),
            },
        }
    }
}

impl Config {
    /// One detector's switches.
    pub fn get(&self, d: Detector) -> &DetectorConfig {
        match d {
            Detector::Exposure => &self.exposure,
            Detector::Usb => &self.usb,
            Detector::Recording => &self.recording,
            Detector::Tracker => &self.tracker,
        }
    }

    /// One detector's switches, to change.
    pub fn get_mut(&mut self, d: Detector) -> &mut DetectorConfig {
        match d {
            Detector::Exposure => &mut self.exposure,
            Detector::Usb => &mut self.usb,
            Detector::Recording => &mut self.recording,
            Detector::Tracker => &mut self.tracker,
        }
    }
}

/// Why a change was refused.
///
/// Every one of these is a rule from the plan rather than a parse failure, and
/// each carries the sentence the surface can show, because "that did not work" on
/// a privacy page tells somebody nothing about whether they are protected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// A detector name nothing recognises.
    NoSuchDetector(String),
    /// An alert mode nothing recognises.
    NoSuchMode(String),
    /// The recording indicator may not notify. §7: it is too noisy to push, so
    /// the option is disabled on the page AND refused here, because a page is not
    /// the only thing that can call this.
    RecordingIsAlwaysQuiet,
    /// Sensitivity was set on a detector that has none.
    NoSensitivity(Detector),
    /// A sensitivity outside that detector's vocabulary.
    NoSuchLevel { detector: Detector, level: String },
}

impl Refused {
    /// The sentence to show. Says what did not happen and what still holds,
    /// because a refusal that only names the rule leaves a person guessing about
    /// the state of their own machine.
    pub fn message(&self) -> String {
        match self {
            Refused::NoSuchDetector(name) => {
                format!("There is no detector called {name}. Nothing was changed.")
            }
            Refused::NoSuchMode(name) => {
                format!("{name} is not a way of alerting. Nothing was changed.")
            }
            Refused::RecordingIsAlwaysQuiet => {
                "The recording-device indicator stays quiet. It sees too many passing devices to \
                 be worth a notification, so it only ever changes the indicator."
                    .to_string()
            }
            Refused::NoSensitivity(d) => format!(
                "The {} detector has no sensitivity to set. Nothing was changed.",
                d.as_str()
            ),
            Refused::NoSuchLevel { detector, level } => format!(
                "{level} is not a sensitivity the {} detector has. Nothing was changed.",
                detector.as_str()
            ),
        }
    }
}

/// The vocabulary a detector's sensitivity is drawn from, if it has one.
pub fn levels(d: Detector) -> Option<&'static [&'static str]> {
    match d {
        Detector::Recording => Some(&PROXIMITY),
        Detector::Tracker => Some(&STRICTNESS),
        Detector::Exposure | Detector::Usb => None,
    }
}

/// Turn a detector on or off.
pub fn set_detector(cfg: &mut Config, name: &str, on: bool) -> Result<(), Refused> {
    let d = Detector::parse(name).ok_or_else(|| Refused::NoSuchDetector(name.to_string()))?;
    cfg.get_mut(d).on = on;
    Ok(())
}

/// Change how a detector speaks up.
pub fn set_alerts(cfg: &mut Config, name: &str, mode: &str) -> Result<(), Refused> {
    let d = Detector::parse(name).ok_or_else(|| Refused::NoSuchDetector(name.to_string()))?;
    let m = Alerts::parse(mode).ok_or_else(|| Refused::NoSuchMode(mode.to_string()))?;
    if d == Detector::Recording && m == Alerts::Notify {
        return Err(Refused::RecordingIsAlwaysQuiet);
    }
    cfg.get_mut(d).alerts = m;
    Ok(())
}

/// Change a watcher's sensitivity.
pub fn set_sensitivity(cfg: &mut Config, name: &str, level: &str) -> Result<(), Refused> {
    let d = Detector::parse(name).ok_or_else(|| Refused::NoSuchDetector(name.to_string()))?;
    let allowed = levels(d).ok_or(Refused::NoSensitivity(d))?;
    if !allowed.contains(&level) {
        return Err(Refused::NoSuchLevel {
            detector: d,
            level: level.to_string(),
        });
    }
    cfg.get_mut(d).sensitivity = Some(level.to_string());
    Ok(())
}

/// Where the file lives. `$XDG_CONFIG_HOME` when set, else `~/.config`.
pub fn config_path() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".config"),
    };
    Some(base.join("arlen/sentinel.toml"))
}

/// Read the file, falling back per field.
///
/// A missing file is the ordinary first-run case and reads as the defaults. A
/// file that will not parse is NOT: the whole document is one unit, so a broken
/// one reads as the defaults too, which leaves the protections on. The caller
/// gets told which happened so it can say so rather than silently rewriting
/// somebody's file over a typo.
pub fn load(path: &Path) -> (Config, Option<String>) {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Config::default(), None),
        Err(e) => {
            return (
                Config::default(),
                Some(format!("could not read {}: {e}", path.display())),
            )
        }
    };
    match parse(&text) {
        Ok(cfg) => (cfg, None),
        Err(e) => (
            Config::default(),
            Some(format!("could not parse {}: {e}", path.display())),
        ),
    }
}

/// Parse the document, filling each absent or unrecognised field from the
/// default rather than failing the whole read.
///
/// Serde would reject the document on the first bad enum value, which for this
/// file is the wrong shape of strictness: one mistyped `alerts` should not decide
/// whether the other three detectors run. So the document is read as a table and
/// each field is taken only when it is one this crate knows.
pub fn parse(text: &str) -> Result<Config, toml::de::Error> {
    let raw: toml::Value = toml::from_str(text)?;
    let mut cfg = Config::default();
    for d in Detector::ALL {
        let Some(table) = raw.get(d.as_str()).and_then(|v| v.as_table()) else {
            continue;
        };
        let allowed = levels(d);
        let slot = cfg.get_mut(d);
        if let Some(on) = table.get("on").and_then(|v| v.as_bool()) {
            slot.on = on;
        }
        if let Some(mode) = table
            .get("alerts")
            .and_then(|v| v.as_str())
            .and_then(Alerts::parse)
        {
            // The recording indicator's quiet is not a preference, so a file that
            // says otherwise is read the same way the setter refuses it.
            if !(d == Detector::Recording && mode == Alerts::Notify) {
                slot.alerts = mode;
            }
        }
        if let (Some(allowed), Some(level)) =
            (allowed, table.get("sensitivity").and_then(|v| v.as_str()))
        {
            if allowed.contains(&level) {
                slot.sensitivity = Some(level.to_string());
            }
        }
    }
    Ok(cfg)
}

/// Write the file, creating `~/.config/arlen` if this is the first switch.
///
/// Written to a sibling temp file and renamed, so a person who opens their config
/// mid-write never sees half a document, and a crash leaves the previous switches
/// rather than an empty file that would read as the defaults.
pub fn save(path: &Path, cfg: &Config) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(cfg)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_plans_matrix() {
        let c = Config::default();
        assert!(c.exposure.on && c.usb.on, "the deterministic pair runs");
        assert!(
            !c.recording.on && !c.tracker.on,
            "the ambient watchers are opted into"
        );
        assert_eq!(c.usb.alerts, Alerts::Notify);
        assert_eq!(c.recording.alerts, Alerts::Quiet);
    }

    #[test]
    fn an_absent_file_reads_as_the_defaults_and_says_nothing_went_wrong() {
        let dir = tempfile::tempdir().unwrap();
        let (cfg, problem) = load(&dir.path().join("nothing-here.toml"));
        assert_eq!(cfg, Config::default());
        assert!(problem.is_none());
    }

    #[test]
    fn a_broken_file_leaves_the_protections_on_and_reports_it() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sentinel.toml");
        std::fs::write(&p, "this is not toml {{{").unwrap();
        let (cfg, problem) = load(&p);
        assert!(cfg.exposure.on && cfg.usb.on);
        assert!(problem.unwrap().contains("could not parse"));
    }

    #[test]
    fn one_bad_field_does_not_decide_the_other_detectors() {
        let cfg = parse(
            "[exposure]\nalerts = \"shout\"\non = false\n\n[usb]\non = true\nalerts = \"quiet\"\n",
        )
        .unwrap();
        assert!(!cfg.exposure.on, "the field that parsed was taken");
        assert_eq!(
            cfg.exposure.alerts,
            Alerts::Quiet,
            "the one that did not fell back"
        );
        assert_eq!(cfg.usb.alerts, Alerts::Quiet, "and its neighbour was read");
    }

    #[test]
    fn a_file_that_asks_the_recording_indicator_to_notify_is_read_as_quiet() {
        let cfg = parse("[recording]\non = true\nalerts = \"notify\"\n").unwrap();
        assert!(cfg.recording.on);
        assert_eq!(cfg.recording.alerts, Alerts::Quiet);
    }

    #[test]
    fn switches_round_trip_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("arlen/sentinel.toml");
        let mut cfg = Config::default();
        set_detector(&mut cfg, "tracker", true).unwrap();
        set_sensitivity(&mut cfg, "tracker", "strict").unwrap();
        set_alerts(&mut cfg, "exposure", "notify").unwrap();
        save(&p, &cfg).unwrap();
        let (back, problem) = load(&p);
        assert!(problem.is_none());
        assert_eq!(back, cfg);
    }

    #[test]
    fn the_recording_indicator_may_not_be_told_to_notify() {
        let mut cfg = Config::default();
        let e = set_alerts(&mut cfg, "recording", "notify").unwrap_err();
        assert_eq!(e, Refused::RecordingIsAlwaysQuiet);
        assert!(e.message().contains("stays quiet"));
        assert_eq!(cfg.recording.alerts, Alerts::Quiet);
    }

    #[test]
    fn a_detector_with_no_sensitivity_refuses_one() {
        let mut cfg = Config::default();
        let e = set_sensitivity(&mut cfg, "exposure", "near").unwrap_err();
        assert_eq!(e, Refused::NoSensitivity(Detector::Exposure));
    }

    #[test]
    fn each_watcher_keeps_to_its_own_vocabulary() {
        let mut cfg = Config::default();
        assert!(set_sensitivity(&mut cfg, "recording", "strict").is_err());
        assert!(set_sensitivity(&mut cfg, "tracker", "room").is_err());
        assert!(set_sensitivity(&mut cfg, "recording", "near").is_ok());
        assert!(set_sensitivity(&mut cfg, "tracker", "cautious").is_ok());
    }

    #[test]
    fn an_unknown_detector_is_refused_rather_than_matched_to_a_real_one() {
        let mut cfg = Config::default();
        let e = set_detector(&mut cfg, "expsoure", true).unwrap_err();
        assert_eq!(e, Refused::NoSuchDetector("expsoure".to_string()));
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn a_refusal_says_what_still_holds() {
        for r in [
            Refused::NoSuchDetector("x".into()),
            Refused::NoSuchMode("x".into()),
            Refused::NoSensitivity(Detector::Usb),
            Refused::NoSuchLevel {
                detector: Detector::Tracker,
                level: "x".into(),
            },
        ] {
            assert!(
                r.message().contains("Nothing was changed"),
                "{r:?} leaves a person guessing"
            );
        }
    }
}
