//! freedesktop XDG Sound Theme resolution (sound-system-plan.md SO-R1, decided
//! 12 June: "adopt the freedesktop XDG Sound Theme + Sound Naming specs as-is").
//!
//! This is the name-to-file RESOLVER the daemon's playback path consults: given a
//! sound name (a freedesktop Sound Naming-spec name, e.g. `message-new-instant`),
//! it walks the active theme's `index.theme` `Directories`, follows the
//! `Inherits` chain, and ends in the `freedesktop` fallback theme, searching the
//! fixed extension order `.disabled / .oga / .ogg / .wav`. A `.disabled` marker
//! silences the event (terminates lookup); an audio file is the cue to play;
//! nothing found is silent. The decode + PipeWire playback + the DND/Focus/volume
//! policy gate sit on top of this (and need a live audio device, so they verify
//! on metal); the resolution itself is pure filesystem logic, tested here.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

/// Where a sound-name lookup landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SoundResolution {
    /// The playable sound file the name resolved to.
    File(PathBuf),
    /// A `.disabled` marker silenced this event (a deliberate per-event mute).
    Silenced,
    /// No file and no `.disabled` in the theme chain or the freedesktop fallback.
    NotFound,
}

/// The extension lookup order (freedesktop Sound Theme spec). `.disabled` is
/// first: a silence marker wins over any audio file in the same directory.
const EXTENSIONS: [&str; 4] = [".disabled", ".oga", ".ogg", ".wav"];

/// The fallback theme every inheritance chain ends in (freedesktop Sound Theme
/// spec), so a name absent from the active theme still resolves to the system
/// default cue.
const FALLBACK_THEME: &str = "freedesktop";

/// Resolve a sound `name` in `theme` across the `roots` (each a `.../sounds`
/// base directory, in lookup precedence), following the theme's `Inherits` chain
/// and ending in the `freedesktop` fallback. Returns [`SoundResolution::Silenced`]
/// on a `.disabled` marker, [`SoundResolution::File`] on an audio file, and
/// [`SoundResolution::NotFound`] when neither exists anywhere in the chain.
/// Cycle-safe (a theme is searched at most once).
pub fn resolve_sound(roots: &[PathBuf], theme: &str, name: &str) -> SoundResolution {
    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    queue.push_back(theme.to_string());
    let mut appended_fallback = false;

    while let Some(t) = queue.pop_front() {
        if !visited.insert(t.clone()) {
            continue;
        }
        let mut inherits: Vec<String> = Vec::new();
        for root in roots {
            let themedir = root.join(&t);
            let (dirs, inh) = read_theme_index(&themedir);
            inherits.extend(inh);
            for dir in &dirs {
                let base = if dir.is_empty() { themedir.clone() } else { themedir.join(dir) };
                for ext in EXTENSIONS {
                    let cand = base.join(format!("{name}{ext}"));
                    if cand.exists() {
                        return if ext == ".disabled" {
                            SoundResolution::Silenced
                        } else {
                            SoundResolution::File(cand)
                        };
                    }
                }
            }
        }
        for i in inherits {
            queue.push_back(i);
        }
        // The freedesktop fallback is appended once, after the theme + every
        // inherited theme has been searched.
        if queue.is_empty() && !appended_fallback {
            appended_fallback = true;
            queue.push_back(FALLBACK_THEME.to_string());
        }
    }
    SoundResolution::NotFound
}

/// Parse a theme's `index.theme`: the `Directories` to search and the `Inherits`
/// parents. A missing or `Directories`-less index means search the theme dir
/// itself (`[""]`), so a flat theme (sounds directly under the theme dir) works.
fn read_theme_index(themedir: &Path) -> (Vec<String>, Vec<String>) {
    let Ok(text) = std::fs::read_to_string(themedir.join("index.theme")) else {
        return (vec![String::new()], Vec::new());
    };
    let mut dirs = Vec::new();
    let mut inherits = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Directories") {
            if let Some(v) = rest.trim_start().strip_prefix('=') {
                dirs = split_list(v);
            }
        } else if let Some(rest) = line.strip_prefix("Inherits") {
            if let Some(v) = rest.trim_start().strip_prefix('=') {
                inherits = split_list(v);
            }
        }
    }
    if dirs.is_empty() {
        dirs.push(String::new());
    }
    (dirs, inherits)
}

/// Split a freedesktop key list (`a,b,c`) into trimmed, non-empty entries.
fn split_list(v: &str) -> Vec<String> {
    v.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// The sound-theme base directories in freedesktop precedence:
/// `$XDG_DATA_HOME/sounds` (or `~/.local/share/sounds`) first, then each
/// `$XDG_DATA_DIRS/sounds`. The production roots for [`resolve_sound`].
pub fn default_sound_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = std::env::var_os("XDG_DATA_HOME") {
        roots.push(PathBuf::from(home).join("sounds"));
    } else if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".local/share/sounds"));
    }
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".to_string());
    for d in data_dirs.split(':').filter(|s| !s.is_empty()) {
        roots.push(PathBuf::from(d).join("sounds"));
    }
    roots
}

/// An Arlen sound event in the restrained default-on set (sound-system-plan.md:
/// ~5-8 cues that sound out of the box; everything else is silent-by-default via
/// the `.disabled` mechanism). Each maps to a freedesktop Sound Naming-spec name
/// so the cue resolves in any freedesktop sound theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundEvent {
    /// A notification arrived.
    NotificationArrived,
    /// An error / failure was surfaced.
    Error,
    /// A warning was surfaced.
    Warning,
    /// A user-requested action finished successfully.
    ActionCompleted,
    /// A removable device was attached (mount).
    DeviceAdded,
    /// A removable device was detached (unmount).
    DeviceRemoved,
}

impl SoundEvent {
    /// The freedesktop Sound Naming-spec name this event maps to - the `name`
    /// [`resolve_sound`] looks up in the active theme. Arlen adopts the standard
    /// vocabulary (notification -> `message-new-instant`, error -> `dialog-error`,
    /// warning -> `dialog-warning`, action-completion -> `complete`, device add/
    /// remove -> `device-added`/`device-removed`) so the cues resolve in any
    /// freedesktop theme; an Arlen-specific cue would carry an `x-arlen-` prefix.
    pub fn sound_name(self) -> &'static str {
        match self {
            SoundEvent::NotificationArrived => "message-new-instant",
            SoundEvent::Error => "dialog-error",
            SoundEvent::Warning => "dialog-warning",
            SoundEvent::ActionCompleted => "complete",
            SoundEvent::DeviceAdded => "device-added",
            SoundEvent::DeviceRemoved => "device-removed",
        }
    }
}

/// The central-playback gate (`sound-system-plan.md`): whether a cue tied to a
/// notification should actually sound. A sound FOLLOWS the notification's own
/// visibility decision - if DND, Focus Mode or a per-app rule suppressed, queued or
/// dropped the notification, its sound is silenced too, so an app can never force
/// attention through sound past the gate the notification itself was held by. On top
/// of that, a global sound mute or a zero master volume silences every cue
/// regardless of source. Only an `Allow`ed notification, unmuted, at a positive
/// volume, sounds. Pure, so the policy is tested without an audio device.
pub fn cue_should_play(
    suppress: crate::dnd::state::SuppressResult,
    sound_muted: bool,
    master_volume: f32,
) -> bool {
    use crate::dnd::state::SuppressResult;
    matches!(suppress, SuppressResult::Allow) && !sound_muted && master_volume > 0.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dnd::state::SuppressResult;
    use std::fs;
    use tempfile::TempDir;

    /// Create `<root>/<theme>/<dir>/<file>` (dir empty = the theme dir), with an
    /// optional `index.theme` body for the theme.
    fn put(root: &Path, theme: &str, index: Option<&str>, dir: &str, file: Option<&str>) {
        let themedir = root.join(theme);
        if let Some(body) = index {
            fs::create_dir_all(&themedir).unwrap();
            fs::write(themedir.join("index.theme"), body).unwrap();
        }
        if let Some(f) = file {
            let d = if dir.is_empty() { themedir.clone() } else { themedir.join(dir) };
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join(f), b"x").unwrap();
        }
    }

    #[test]
    fn an_audio_file_in_a_theme_directory_resolves() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        put(&root, "arlen", Some("[Sound Theme]\nDirectories=stereo\n"), "stereo", Some("bell.oga"));
        let r = resolve_sound(&[root.clone()], "arlen", "bell");
        assert_eq!(r, SoundResolution::File(root.join("arlen/stereo/bell.oga")));
    }

    #[test]
    fn a_disabled_marker_silences_and_wins_over_audio() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        put(&root, "arlen", Some("[Sound Theme]\nDirectories=stereo\n"), "stereo", Some("bell.oga"));
        // Same name, .disabled marker in the same dir: silence wins.
        put(&root, "arlen", None, "stereo", Some("bell.disabled"));
        assert_eq!(resolve_sound(&[root], "arlen", "bell"), SoundResolution::Silenced);
    }

    #[test]
    fn a_name_absent_from_the_theme_falls_through_to_an_inherited_theme() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        put(&root, "arlen", Some("[Sound Theme]\nDirectories=stereo\nInherits=base\n"), "stereo", None);
        put(&root, "base", Some("[Sound Theme]\nDirectories=stereo\n"), "stereo", Some("error.oga"));
        let r = resolve_sound(&[root.clone()], "arlen", "error");
        assert_eq!(r, SoundResolution::File(root.join("base/stereo/error.oga")));
    }

    #[test]
    fn a_name_in_no_theme_falls_through_to_freedesktop() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        put(&root, "arlen", Some("[Sound Theme]\nDirectories=stereo\n"), "stereo", None);
        // No Inherits, but freedesktop is the implicit final fallback.
        put(&root, "freedesktop", Some("[Sound Theme]\nDirectories=stereo\n"), "stereo", Some("complete.oga"));
        let r = resolve_sound(&[root.clone()], "arlen", "complete");
        assert_eq!(r, SoundResolution::File(root.join("freedesktop/stereo/complete.oga")));
    }

    #[test]
    fn a_name_nowhere_is_not_found() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        put(&root, "arlen", Some("[Sound Theme]\nDirectories=stereo\n"), "stereo", None);
        assert_eq!(resolve_sound(&[root], "arlen", "no-such-cue"), SoundResolution::NotFound);
    }

    #[test]
    fn an_inheritance_cycle_terminates() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        put(&root, "a", Some("[Sound Theme]\nInherits=b\n"), "", None);
        put(&root, "b", Some("[Sound Theme]\nInherits=a\n"), "", None);
        // Must not loop forever; the cue is simply not found.
        assert_eq!(resolve_sound(&[root], "a", "x"), SoundResolution::NotFound);
    }

    #[test]
    fn a_flat_theme_without_an_index_searches_the_theme_dir() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        // No index.theme: the sound lives directly under the theme dir.
        put(&root, "flat", None, "", Some("warning.ogg"));
        let r = resolve_sound(&[root.clone()], "flat", "warning");
        assert_eq!(r, SoundResolution::File(root.join("flat/warning.ogg")));
    }

    #[test]
    fn each_event_maps_to_its_standard_freedesktop_name() {
        // The adopted vocabulary (sound-system-plan.md); these are the canonical
        // freedesktop Sound Naming-spec names, so they resolve in any theme.
        assert_eq!(SoundEvent::NotificationArrived.sound_name(), "message-new-instant");
        assert_eq!(SoundEvent::Error.sound_name(), "dialog-error");
        assert_eq!(SoundEvent::Warning.sound_name(), "dialog-warning");
        assert_eq!(SoundEvent::ActionCompleted.sound_name(), "complete");
        assert_eq!(SoundEvent::DeviceAdded.sound_name(), "device-added");
        assert_eq!(SoundEvent::DeviceRemoved.sound_name(), "device-removed");
    }

    #[test]
    fn an_events_name_resolves_through_the_theme() {
        // The event -> name map threads into the resolver: a theme cue for the
        // event's standard name is found via SoundEvent::sound_name.
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        put(&root, "arlen", Some("[Sound Theme]\nDirectories=stereo\n"), "stereo", Some("dialog-error.oga"));
        let name = SoundEvent::Error.sound_name();
        assert_eq!(
            resolve_sound(&[root.clone()], "arlen", name),
            SoundResolution::File(root.join("arlen/stereo/dialog-error.oga")),
        );
    }

    #[test]
    fn the_cue_gate_follows_the_notification_and_the_global_state() {
        // An allowed, unmuted, audible notification sounds.
        assert!(cue_should_play(SuppressResult::Allow, false, 1.0));
        // A suppressed / queued / dropped notification never sounds (no force-past-DND).
        assert!(!cue_should_play(SuppressResult::Suppress, false, 1.0));
        assert!(!cue_should_play(SuppressResult::Queue, false, 1.0));
        assert!(!cue_should_play(SuppressResult::Drop, false, 1.0));
        // The global mute and a zero master volume silence even an allowed cue.
        assert!(!cue_should_play(SuppressResult::Allow, true, 1.0));
        assert!(!cue_should_play(SuppressResult::Allow, false, 0.0));
    }
}
