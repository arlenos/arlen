//! SC-3 (Flatpak half): the capability footprint for a Flathub app, read from
//! its Flatpak `metadata` file (store-app.md section 9.2, "capability panel:
//! `finish-args`").
//!
//! The composed AppStream catalog Flathub publishes carries display metadata
//! only, never the sandbox permissions, so the footprint cannot come from the
//! same document `flathub_entries` parses. It comes from the app's `metadata`
//! file, whose `[Context]` section is the persisted form of the `finish-args`
//! the build declared (`shared=network;ipc;`, `filesystems=host;`, ...).
//!
//! The labels emitted here are the SAME vocabulary the forage path emits
//! (`network`, `filesystem`, `notifications`, `clipboard`, `audio`, `system`),
//! per section 8.1's binding rule: align to the real capability-token grant
//! classes, never mint a store-only taxonomy. A permission with no honest
//! counterpart is left out rather than given an invented name.

use std::collections::BTreeSet;

/// Parse the `[Context]` section of a Flatpak `metadata` file into capability
/// labels, sorted and deduped. Text outside `[Context]` is ignored; an absent or
/// empty section yields no labels (an app that asks for nothing).
pub fn context_labels(metadata: &str) -> Vec<String> {
    let mut labels = BTreeSet::new();
    for (key, value) in context_entries(metadata) {
        match key {
            // `shared=network` is the whole-namespace network share: the app can
            // reach the internet.
            "shared" if list_has(value, "network") => {
                labels.insert("network");
            }
            // Any filesystem grant at all (`host`, `home`, an explicit path)
            // means the app reaches files outside its own sandbox.
            "filesystems" if !value.trim().is_empty() => {
                labels.insert("filesystem");
            }
            "sockets" => {
                if list_has(value, "pulseaudio") {
                    labels.insert("audio");
                }
                // X11 has no isolation between clients: a socket grant lets the
                // app read other windows' input and the clipboard. It is a
                // system-wide grant, which is how Flatpak's own docs and Flatseal
                // present it, so it is not softened here.
                if list_has(value, "x11") || list_has(value, "fallback-x11") {
                    labels.insert("system");
                }
            }
            // `devices=all` hands over /dev (camera, mic, everything). `dri` is
            // just the GPU and is not a privacy grant, so it is NOT labelled.
            "devices" if list_has(value, "all") => {
                labels.insert("system");
            }
            // A bus name the app may talk to. The notification service is the one
            // with a grant class of its own.
            "talk-name" | "system-talk-name"
                if value.contains("org.freedesktop.Notifications") =>
            {
                labels.insert("notifications");
            }
            _ => {}
        }
    }
    labels.into_iter().map(str::to_string).collect()
}

/// The `key=value` pairs of the `[Context]` section, in file order. A metadata
/// file is INI-shaped; only that one section is read, and a line without `=` is
/// skipped rather than guessed at.
fn context_entries(metadata: &str) -> Vec<(&str, &str)> {
    let mut entries = Vec::new();
    let mut in_context = false;
    for line in metadata.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_context = line == "[Context]";
            continue;
        }
        if !in_context || line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            entries.push((key.trim(), value.trim()));
        }
    }
    entries
}

/// Whether a `;`-separated Flatpak list contains `needle`. A leading `!` negates
/// an entry (`shared=!network`), so a negated match does not count as a grant.
fn list_has(value: &str, needle: &str) -> bool {
    value
        .split(';')
        .map(str::trim)
        .any(|item| item == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = "\
[Application]
name=org.demo.App

[Context]
shared=network;ipc;
sockets=x11;wayland;pulseaudio;
filesystems=host;
devices=dri;

[Session Bus Policy]
org.freedesktop.Notifications=talk
";

    #[test]
    fn a_broad_app_reports_each_grant_class() {
        let labels = context_labels(FULL);
        assert!(labels.contains(&"network".to_string()));
        assert!(labels.contains(&"filesystem".to_string()));
        assert!(labels.contains(&"audio".to_string()));
        // x11 is a system-wide grant.
        assert!(labels.contains(&"system".to_string()));
    }

    #[test]
    fn a_sandboxed_app_reports_nothing() {
        let m = "[Context]\nshared=ipc;\nsockets=wayland;\ndevices=dri;\n";
        assert!(context_labels(m).is_empty());
    }

    /// `devices=dri` is only the GPU. Labelling it `system` would overstate the
    /// grant on nearly every graphical app, so it must not.
    #[test]
    fn the_gpu_device_is_not_a_system_grant() {
        let m = "[Context]\ndevices=dri;\n";
        assert!(context_labels(m).is_empty());
        let all = "[Context]\ndevices=all;\n";
        assert_eq!(context_labels(all), vec!["system".to_string()]);
    }

    /// A negated share is not a grant: `shared=!network` must not read as network.
    #[test]
    fn a_negated_share_is_not_a_grant() {
        let m = "[Context]\nshared=!network;ipc;\n";
        assert!(context_labels(m).is_empty());
    }

    /// Only `[Context]` is read: a `filesystems=` line under another section must
    /// not leak in.
    #[test]
    fn only_the_context_section_is_read() {
        let m = "[Build]\nfilesystems=host;\n\n[Context]\nshared=ipc;\n";
        assert!(context_labels(m).is_empty());
    }

    #[test]
    fn an_empty_filesystem_value_is_not_a_grant() {
        let m = "[Context]\nfilesystems=\n";
        assert!(context_labels(m).is_empty());
    }

    #[test]
    fn a_notification_talk_name_is_a_notifications_grant() {
        let m = "[Context]\ntalk-name=org.freedesktop.Notifications;\n";
        assert_eq!(context_labels(m), vec!["notifications".to_string()]);
    }

    #[test]
    fn labels_are_sorted_and_deduped() {
        let m = "[Context]\nshared=network;\nsockets=x11;fallback-x11;\ndevices=all;\n";
        // x11, fallback-x11 and devices=all all map to `system`: one label.
        assert_eq!(context_labels(m), vec!["network".to_string(), "system".to_string()]);
    }
}
