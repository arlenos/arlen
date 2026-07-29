//! PAS-7: turning a declared [`ValueSource`] into the choices this machine
//! actually offers.
//!
//! An enum whose valid values are audio devices or installed themes cannot list
//! them in its recipe: they belong to the user's machine and they change while
//! the page is open. The schema says where to look; this resolves it at render
//! time.
//!
//! **An empty list and a failed lookup are different answers, and the
//! distinction is the whole point of [`Resolution`].** Collapsing "no audio
//! device is plugged in" and "PipeWire did not answer" into an empty dropdown
//! tells the user they have no devices when the truth is that we could not ask.
//! One is a fact about their hardware, the other is our failure, and only one of
//! them is worth them acting on.
//!
//! Resolution happens in Settings, not in the settings broker. The broker holds
//! the authority to write every app's config; teaching it to enumerate audio
//! devices and scan installed applications would widen that process for the sake
//! of a dropdown. Settings already renders the page and already has the access.

use arlen_forage_recipe::settings::{SettingOption, ValueSource};

/// The outcome of consulting a value source.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    /// What the machine currently offers. Legitimately empty when there is
    /// genuinely nothing: no second audio output, no extra themes installed.
    Options(Vec<SettingOption>),
    /// The source could not be consulted at all. Carries a plain-language
    /// reason so the page can say why instead of showing an empty list.
    Unavailable(String),
}

impl Resolution {
    /// Report a source we could not consult.
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Resolution::Unavailable(reason.into())
    }
}

/// The live system state a settings page can ask about.
///
/// A seam rather than direct calls: the sources differ wildly in how they are
/// read (a directory scan, a PipeWire query, a scan of installed `.desktop`
/// files), and every one of them can fail in its own way. Each returns a
/// [`Resolution`] so a provider that cannot answer says so rather than returning
/// an empty list that reads as an answer.
pub trait SystemValues {
    /// Audio sinks currently present.
    fn audio_outputs(&self) -> Resolution;
    /// Audio sources currently present.
    fn audio_inputs(&self) -> Resolution;
    /// Themes installed on this machine.
    fn installed_themes(&self) -> Resolution;
    /// Locales available on this machine.
    fn locales(&self) -> Resolution;
    /// Installed applications that handle `http`/`https`.
    fn browsers(&self) -> Resolution;
}

/// Resolve one declared source against live system state.
///
/// The match is exhaustive on purpose: adding a source to the closed
/// [`ValueSource`] set must not compile until something can answer it, or a
/// package could declare a source that silently resolves to nothing.
pub fn resolve(source: ValueSource, values: &impl SystemValues) -> Resolution {
    match source {
        ValueSource::AudioOutputs => values.audio_outputs(),
        ValueSource::AudioInputs => values.audio_inputs(),
        ValueSource::InstalledThemes => values.installed_themes(),
        ValueSource::Locales => values.locales(),
        ValueSource::Browsers => values.browsers(),
    }
}

/// Themes installed on this machine: the ones shipped with the system plus every
/// `*.toml` in the user's theme directory.
///
/// A missing user directory is not a failure - it just means nothing extra is
/// installed - so it resolves to the bundled themes rather than Unavailable. A
/// directory that exists but cannot be read IS a failure, because something is
/// there and we could not see it.
pub fn themes_in(bundled: &[&str], user_dir: &std::path::Path) -> Resolution {
    let mut options: Vec<SettingOption> = bundled
        .iter()
        .map(|id| SettingOption {
            value: (*id).to_string(),
            label: title_case(id),
            description: "Shipped with the system".to_string(),
        })
        .collect();

    match std::fs::read_dir(user_dir) {
        Ok(entries) => {
            let mut installed: Vec<String> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("toml"))
                .filter_map(|p| {
                    p.file_stem()
                        .and_then(|s| s.to_str())
                        .map(|s| s.to_string())
                })
                .filter(|id| !bundled.contains(&id.as_str()))
                .collect();
            // Directory order is arbitrary; a list that reshuffles between two
            // renders of the same page looks broken.
            installed.sort();
            options.extend(installed.into_iter().map(|id| SettingOption {
                label: title_case(&id),
                value: id,
                description: "Installed by you".to_string(),
            }));
            Resolution::Options(options)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Resolution::Options(options),
        Err(e) => Resolution::unavailable(format!("could not read the theme directory: {e}")),
    }
}

/// Parse the locales `locale -a` lists.
///
/// `C` and `POSIX` are dropped: they are the no-locale fallbacks, and offering
/// them as a language is how a user ends up with a machine that displays dates
/// and sorting in a way nobody chose. `.utf8` and `.UTF-8` name the same locale,
/// so they are folded together rather than shown twice.
pub fn locales_from(listing: &str) -> Resolution {
    let mut seen: Vec<String> = Vec::new();
    for line in listing.lines() {
        let name = line.trim();
        if name.is_empty() || name == "C" || name == "POSIX" || name.starts_with("C.") {
            continue;
        }
        let canonical = canonical_locale(name);
        if !seen.contains(&canonical) {
            seen.push(canonical);
        }
    }
    seen.sort();
    Resolution::Options(
        seen.into_iter()
            .map(|name| SettingOption {
                label: name.clone(),
                description: String::new(),
                value: name,
            })
            .collect(),
    )
}

/// Parse the devices `pactl list short sinks` (or `sources`) prints: one
/// tab-separated row per device, `id  name  driver  spec  state`.
///
/// The stored value is the device NAME, not the numeric id: ids are handed out
/// per session, so a config holding `47` would point at a different device after
/// a reboot, or at nothing.
///
/// Monitor sources are dropped from the input list. They are loopbacks of an
/// output, they appear next to real microphones, and picking one as your
/// recording input records the machine's own playback instead of you.
pub fn pactl_devices_from(listing: &str) -> Resolution {
    let mut names: Vec<String> = Vec::new();
    for line in listing.lines() {
        let mut fields = line.split('\t');
        let (Some(_id), Some(name)) = (fields.next(), fields.next()) else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() || name.ends_with(".monitor") {
            continue;
        }
        names.push(name.to_string());
    }

    // Label each device, then widen any label two devices share. A machine with
    // an onboard card and a loopback has two `analog-stereo` outputs, and naming
    // them both "Analog stereo" leaves the user picking blind. Verified against a
    // real host, where exactly that happened.
    let short: Vec<String> = names.iter().map(|n| device_label(n)).collect();
    let options = names
        .iter()
        .zip(&short)
        .map(|(name, label)| {
            let unique = short.iter().filter(|l| *l == label).count() == 1;
            SettingOption {
                label: if unique {
                    label.clone()
                } else {
                    device_label_wide(name)
                },
                description: String::new(),
                value: name.clone(),
            }
        })
        .collect();
    Resolution::Options(options)
}

/// A device name is a long identifier (`alsa_output.pci-0000_00_1f.3.analog-stereo`).
/// The tail after the last `.` is usually what distinguishes one from another.
fn device_label(name: &str) -> String {
    let tail = name.rsplit('.').next().unwrap_or(name);
    if tail.len() < 3 {
        return name.to_string();
    }
    title_case(tail)
}

/// The label for a device whose short one is ambiguous: everything after the
/// `alsa_output.`-style prefix, which is the part that actually differs.
fn device_label_wide(name: &str) -> String {
    match name.split_once('.') {
        Some((_prefix, rest)) if !rest.is_empty() => title_case(rest),
        _ => title_case(name),
    }
}

/// Applications that can open a web address, from the installed desktop entries.
///
/// An entry qualifies by declaring `x-scheme-handler/http` or `https` in its
/// `MimeType`, which is how the desktop-entry spec says a browser announces
/// itself. `NoDisplay=true` entries are skipped: they are deliberately hidden
/// from menus and offering one as a choice contradicts that.
pub fn browsers_in(dirs: &[std::path::PathBuf]) -> Resolution {
    let mut options: Vec<SettingOption> = Vec::new();
    let mut any_dir_read = false;

    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        any_dir_read = true;
        for path in entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("desktop"))
        {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Some(entry) = browser_entry(&text) else {
                continue;
            };
            let Some(id) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            // Earlier directories win: a user's own entry shadows the system
            // one of the same name, matching how desktop entries resolve.
            if options.iter().any(|o| o.value == id) {
                continue;
            }
            options.push(SettingOption {
                value: id.to_string(),
                label: entry,
                description: String::new(),
            });
        }
    }

    if !any_dir_read {
        return Resolution::unavailable("no application directory could be read");
    }
    options.sort_by(|a, b| a.label.cmp(&b.label));
    Resolution::Options(options)
}

/// The display name of a desktop entry that handles web addresses, or `None` if
/// it does not handle them or is hidden.
fn browser_entry(text: &str) -> Option<String> {
    let mut name = None;
    let mut handles_web = false;
    let mut hidden = false;

    for line in text.lines() {
        let line = line.trim();
        // Only the main section describes the application itself; the actions
        // below it have their own Name= lines ("Open a New Window").
        if line.starts_with('[') && line != "[Desktop Entry]" {
            break;
        }
        if let Some(v) = line.strip_prefix("Name=") {
            name.get_or_insert_with(|| v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("MimeType=") {
            handles_web = v.split(';').any(|m| {
                let m = m.trim();
                m == "x-scheme-handler/http" || m == "x-scheme-handler/https"
            });
        } else if let Some(v) = line.strip_prefix("NoDisplay=") {
            hidden = v.trim() == "true";
        }
    }

    if handles_web && !hidden {
        name
    } else {
        None
    }
}

/// Fold the charset suffix to one spelling so `de_AT.utf8` and `de_AT.UTF-8` are
/// one entry.
fn canonical_locale(name: &str) -> String {
    match name.split_once('.') {
        Some((base, charset)) => {
            let normalised = charset.replace('-', "").to_ascii_lowercase();
            if normalised == "utf8" {
                format!("{base}.UTF-8")
            } else {
                name.to_string()
            }
        }
        None => name.to_string(),
    }
}

/// A readable label from an id: `solarized-dark` becomes `Solarized dark`.
fn title_case(id: &str) -> String {
    let spaced = id.replace(['-', '_'], " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => spaced,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Nothing;
    impl SystemValues for Nothing {
        fn audio_outputs(&self) -> Resolution {
            Resolution::unavailable("no audio server")
        }
        fn audio_inputs(&self) -> Resolution {
            Resolution::unavailable("no audio server")
        }
        fn installed_themes(&self) -> Resolution {
            Resolution::Options(Vec::new())
        }
        fn locales(&self) -> Resolution {
            Resolution::Options(Vec::new())
        }
        fn browsers(&self) -> Resolution {
            Resolution::Options(Vec::new())
        }
    }

    /// The distinction the whole module exists for: a source that could not be
    /// consulted must not look like a source that answered "none".
    #[test]
    fn a_failed_lookup_is_not_an_empty_list() {
        let unavailable = resolve(ValueSource::AudioOutputs, &Nothing);
        let empty = resolve(ValueSource::Locales, &Nothing);

        assert!(matches!(unavailable, Resolution::Unavailable(_)));
        assert_eq!(empty, Resolution::Options(Vec::new()));
        assert_ne!(unavailable, empty);
    }

    #[test]
    fn each_source_reaches_its_own_provider() {
        struct Naming;
        impl SystemValues for Naming {
            fn audio_outputs(&self) -> Resolution {
                Resolution::unavailable("outputs")
            }
            fn audio_inputs(&self) -> Resolution {
                Resolution::unavailable("inputs")
            }
            fn installed_themes(&self) -> Resolution {
                Resolution::unavailable("themes")
            }
            fn locales(&self) -> Resolution {
                Resolution::unavailable("locales")
            }
            fn browsers(&self) -> Resolution {
                Resolution::unavailable("browsers")
            }
        }
        for (source, expected) in [
            (ValueSource::AudioOutputs, "outputs"),
            (ValueSource::AudioInputs, "inputs"),
            (ValueSource::InstalledThemes, "themes"),
            (ValueSource::Locales, "locales"),
            (ValueSource::Browsers, "browsers"),
        ] {
            assert_eq!(
                resolve(source, &Naming),
                Resolution::unavailable(expected),
                "{source:?} reached the wrong provider"
            );
        }
    }

    fn theme_dir(files: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for f in files {
            std::fs::write(dir.path().join(f), "").unwrap();
        }
        dir
    }

    #[test]
    fn installed_themes_follow_the_bundled_ones() {
        let dir = theme_dir(&["solarized-dark.toml", "nord.toml", "notes.txt"]);
        let Resolution::Options(options) = themes_in(&["dark", "light"], dir.path()) else {
            panic!("should resolve");
        };
        let values: Vec<&str> = options.iter().map(|o| o.value.as_str()).collect();
        assert_eq!(values, vec!["dark", "light", "nord", "solarized-dark"]);
        assert_eq!(options[2].label, "Nord");
        assert_eq!(options[3].label, "Solarized dark");
    }

    /// A theme directory the user never created means no extra themes, not a
    /// broken lookup: the bundled ones are still the honest answer.
    #[test]
    fn a_missing_theme_directory_still_offers_the_bundled_themes() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("never-created");
        let Resolution::Options(options) = themes_in(&["dark"], &missing) else {
            panic!("a missing directory is not a failure");
        };
        assert_eq!(options.len(), 1);
        assert_eq!(options[0].value, "dark");
    }

    /// A user theme sharing a bundled id must not appear twice.
    #[test]
    fn a_user_theme_does_not_duplicate_a_bundled_one() {
        let dir = theme_dir(&["dark.toml"]);
        let Resolution::Options(options) = themes_in(&["dark", "light"], dir.path()) else {
            panic!("should resolve");
        };
        assert_eq!(options.len(), 2, "{options:?}");
    }

    #[test]
    fn the_no_locale_fallbacks_are_not_offered_as_languages() {
        let Resolution::Options(options) =
            locales_from("C\nPOSIX\nC.UTF-8\nde_AT.UTF-8\nen_GB.UTF-8\n")
        else {
            panic!("should resolve");
        };
        let values: Vec<&str> = options.iter().map(|o| o.value.as_str()).collect();
        assert_eq!(values, vec!["de_AT.UTF-8", "en_GB.UTF-8"]);
    }

    /// `locale -a` lists both spellings of the same locale on most machines.
    #[test]
    fn the_two_spellings_of_utf8_are_one_locale() {
        let Resolution::Options(options) = locales_from("de_AT.utf8\nde_AT.UTF-8\n") else {
            panic!("should resolve");
        };
        assert_eq!(options.len(), 1, "{options:?}");
        assert_eq!(options[0].value, "de_AT.UTF-8");
    }

    /// The stored value must be the device name. Session-assigned ids point at a
    /// different device after a reboot.
    #[test]
    fn a_device_is_stored_by_name_not_by_session_id() {
        let Resolution::Options(options) = pactl_devices_from(
            "47\talsa_output.pci-0000_00_1f.3.analog-stereo\tmodule\ts16le\tRUNNING\n",
        ) else {
            panic!("should resolve");
        };
        assert_eq!(options[0].value, "alsa_output.pci-0000_00_1f.3.analog-stereo");
        assert_eq!(options[0].label, "Analog stereo");
    }

    /// Found on a real host: an onboard card and a loopback both end in
    /// `analog-stereo`, so the short label named them identically and the user
    /// had two indistinguishable entries to choose between.
    #[test]
    fn two_devices_never_share_a_label() {
        let Resolution::Options(options) = pactl_devices_from(
            "62\talsa_output.platform-snd_aloop.0.analog-stereo\tPipeWire\ts32le\tSUSPENDED\n\
             64\talsa_output.pci-0000_c1_00.6.analog-stereo\tPipeWire\ts32le\tSUSPENDED\n",
        ) else {
            panic!("should resolve");
        };
        assert_eq!(options.len(), 2);
        assert_ne!(
            options[0].label, options[1].label,
            "the user cannot pick between two identical names"
        );
        assert_eq!(options[0].label, "Platform snd aloop.0.analog stereo");
        assert_eq!(options[1].label, "Pci 0000 c1 00.6.analog stereo");
    }

    /// Widening only happens where it is needed: a device with no twin keeps its
    /// short, readable name.
    #[test]
    fn an_unambiguous_device_keeps_its_short_label() {
        let Resolution::Options(options) = pactl_devices_from(
            "1\talsa_output.pci-0000_00_1f.3.analog-stereo\tPipeWire\ts16le\tRUNNING\n\
             2\tbluez_output.AC_12_34.1.headset-head-unit\tPipeWire\ts16le\tIDLE\n",
        ) else {
            panic!("should resolve");
        };
        assert_eq!(options[0].label, "Analog stereo");
        assert_eq!(options[1].label, "Headset head unit");
    }

    /// A monitor source records the machine's own playback, so offering it beside
    /// the microphones is how someone ends up recording the wrong thing.
    #[test]
    fn a_monitor_source_is_not_offered_as_an_input() {
        let Resolution::Options(options) = pactl_devices_from(
            "1\talsa_input.usb-mic\tmodule\ts16le\tRUNNING\n\
             2\talsa_output.hdmi.monitor\tmodule\ts16le\tIDLE\n",
        ) else {
            panic!("should resolve");
        };
        assert_eq!(options.len(), 1, "{options:?}");
        assert_eq!(options[0].value, "alsa_input.usb-mic");
    }

    fn app_dir(entries: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, body) in entries {
            std::fs::write(dir.path().join(name), body).unwrap();
        }
        dir
    }

    const FIREFOX: &str = "[Desktop Entry]\nName=Firefox\nMimeType=text/html;x-scheme-handler/http;x-scheme-handler/https;\n";

    #[test]
    fn only_entries_that_handle_web_addresses_are_browsers() {
        let dir = app_dir(&[
            ("firefox.desktop", FIREFOX),
            (
                "calc.desktop",
                "[Desktop Entry]\nName=Calculator\nMimeType=application/x-calc;\n",
            ),
        ]);
        let Resolution::Options(options) = browsers_in(&[dir.path().to_path_buf()]) else {
            panic!("should resolve");
        };
        assert_eq!(options.len(), 1, "{options:?}");
        assert_eq!(options[0].label, "Firefox");
        assert_eq!(options[0].value, "firefox.desktop");
    }

    /// An entry that asked not to be shown must not be offered as a choice.
    #[test]
    fn a_hidden_entry_is_not_offered() {
        let dir = app_dir(&[(
            "hidden.desktop",
            "[Desktop Entry]\nName=Hidden\nNoDisplay=true\nMimeType=x-scheme-handler/https;\n",
        )]);
        let Resolution::Options(options) = browsers_in(&[dir.path().to_path_buf()]) else {
            panic!("should resolve");
        };
        assert!(options.is_empty(), "{options:?}");
    }

    /// The actions below the main section carry their own Name= lines, and the
    /// first one would otherwise win.
    #[test]
    fn an_action_name_is_not_mistaken_for_the_application_name() {
        let dir = app_dir(&[(
            "b.desktop",
            "[Desktop Entry]\nName=Chromium\nMimeType=x-scheme-handler/http;\n\
             [Desktop Action new-window]\nName=New Window\n",
        )]);
        let Resolution::Options(options) = browsers_in(&[dir.path().to_path_buf()]) else {
            panic!("should resolve");
        };
        assert_eq!(options[0].label, "Chromium");
    }

    /// Reading no directory at all is a failed lookup, not "you have no browser".
    #[test]
    fn browsers_are_unavailable_when_no_directory_can_be_read() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope");
        assert!(matches!(
            browsers_in(&[missing]),
            Resolution::Unavailable(_)
        ));
    }

    #[test]
    fn a_locale_with_another_charset_is_left_alone() {
        let Resolution::Options(options) = locales_from("ja_JP.eucjp\n") else {
            panic!("should resolve");
        };
        assert_eq!(options[0].value, "ja_JP.eucjp");
    }
}
