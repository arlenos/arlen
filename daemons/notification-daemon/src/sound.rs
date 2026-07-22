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
    // A freedesktop sound name is a flat identifier (`message-new-instant`),
    // never a path. The name can originate from a theme's `[sounds]` override,
    // which passes the theme inert floor (that floor allows `/` and `.`), so a
    // hostile theme could otherwise set `notification = "../../etc/foo"` and make
    // `base.join(name)` escape the sound roots to play an arbitrary file. Reject
    // any name carrying a path separator here, at the join site, so every caller
    // is covered regardless of where the name came from.
    if name.is_empty() || name.contains('/') || name.contains('\\') {
        return SoundResolution::NotFound;
    }
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

/// The freedesktop sound name to play for `event`, honouring a per-event override
/// from the sound config (keyed by the event's own default name), else the standard
/// name. [`resolve_sound`] then looks this name up in the active theme.
pub fn cue_name_for_event(
    event: SoundEvent,
    overrides: &std::collections::HashMap<String, String>,
) -> std::borrow::Cow<'_, str> {
    let default = event.sound_name();
    match overrides.get(default) {
        Some(name) => std::borrow::Cow::Borrowed(name.as_str()),
        None => std::borrow::Cow::Borrowed(default),
    }
}

/// Resolve the playable cue for `event` against the daemon's sound config: apply any
/// per-event name override, then look the name up in the configured theme across
/// `roots` (the resolver follows the theme's `Inherits` chain to the `freedesktop`
/// fallback). The complete event-to-file step; whether it actually sounds is the
/// separate [`cue_should_play`] gate.
pub fn resolve_cue(
    event: SoundEvent,
    config: &crate::config::types::SoundConfig,
    roots: &[std::path::PathBuf],
) -> SoundResolution {
    let name = cue_name_for_event(event, &config.overrides);
    resolve_sound(roots, &config.theme, name.as_ref())
}

/// The sound cue (if any) a notification maps to, from its freedesktop urgency
/// (0 low / 1 normal / 2 critical) and category. Restraint by default
/// (sound-system-plan.md): a low-urgency notification stays SILENT, so only normal
/// and critical arrivals sound. The category picks a specific cue when it names one
/// (`device.added`/`device.removed`, an `error` or `warning` class); otherwise a
/// sounding notification is a plain arrival. `None` means no cue for this one.
pub fn sound_event_for_notification(urgency: u8, category: &str) -> Option<SoundEvent> {
    let cat = category.to_ascii_lowercase();
    if cat.starts_with("device.added") {
        return Some(SoundEvent::DeviceAdded);
    }
    if cat.starts_with("device.removed") {
        return Some(SoundEvent::DeviceRemoved);
    }
    if cat.contains("error") {
        return Some(SoundEvent::Error);
    }
    if cat.contains("warning") {
        return Some(SoundEvent::Warning);
    }
    // Low urgency is silent by default; normal and critical arrivals sound.
    if urgency == 0 {
        return None;
    }
    Some(SoundEvent::NotificationArrived)
}

/// Plays a resolved notification cue. This is the playback SEAM: the name->file
/// resolution and the should-play decision are pure (tested above), and the only
/// part that needs a live audio device is the actual output, isolated behind this
/// trait. The headless default ([`NullSoundPlayer`]) runs the whole cue pipeline
/// without touching a sound server; the metal backend (PipeWire decode + output)
/// is a later increment implementing this same `play`.
///
/// `play` must not block the caller (it is called on the notification dispatch
/// path): a real backend submits to the audio server and returns immediately.
/// `volume` is the `0.0..=1.0` master volume (already gated `> 0` by
/// [`cue_should_play`]); a backend that cannot scale volume ignores it.
pub trait SoundPlayer: Send + Sync {
    /// Play `resolution` at `volume`. A [`SoundResolution::Silenced`] or
    /// [`SoundResolution::NotFound`] is a no-op (the cue resolved to silence).
    fn play(&self, resolution: &SoundResolution, volume: f32);
}

/// The headless default player: it logs the resolved cue and plays nothing, so
/// the daemon runs the full resolve + should-play + cue-name pipeline in CI and
/// on a machine with no audio tool. [`SystemSoundPlayer`] replaces it when a play
/// command is present.
pub struct NullSoundPlayer;

impl SoundPlayer for NullSoundPlayer {
    fn play(&self, resolution: &SoundResolution, _volume: f32) {
        match resolution {
            SoundResolution::File(path) => {
                tracing::debug!(path = %path.display(), "sound cue resolved (no audio backend wired)");
            }
            SoundResolution::Silenced => tracing::trace!("sound cue silenced by a .disabled marker"),
            SoundResolution::NotFound => tracing::trace!("sound cue not found in the theme chain"),
        }
    }
}

/// How to invoke a discovered play command (the volume argument differs per
/// tool; `Canberra`/`Aplay` cannot scale, so they ignore it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlayerKind {
    /// `pw-play --volume <0.0..1.0> <file>` - the PipeWire CLI (ships with
    /// PipeWire, the project's audio server).
    PwPlay,
    /// `paplay --volume <0..65536> <file>` - the PulseAudio/pipewire-pulse CLI.
    PaPlay,
    /// `canberra-gtk-play -f <file>` - the freedesktop sound CLI (no volume).
    Canberra,
    /// `aplay -q <file>` - ALSA, WAV only, last resort (no volume).
    Aplay,
}

impl PlayerKind {
    /// The candidate commands in preference order (PipeWire-native first).
    const CANDIDATES: [(&'static str, PlayerKind); 4] = [
        ("pw-play", PlayerKind::PwPlay),
        ("paplay", PlayerKind::PaPlay),
        ("canberra-gtk-play", PlayerKind::Canberra),
        ("aplay", PlayerKind::Aplay),
    ];

    /// The argument vector to play `file` at `volume` (`0.0..=1.0`).
    fn args(self, file: &Path, volume: f32) -> Vec<std::ffi::OsString> {
        let v = volume.clamp(0.0, 1.0);
        match self {
            PlayerKind::PwPlay => vec![
                "--volume".into(),
                format!("{v:.3}").into(),
                file.into(),
            ],
            PlayerKind::PaPlay => vec![
                "--volume".into(),
                ((v * 65536.0).round() as u32).to_string().into(),
                file.into(),
            ],
            PlayerKind::Canberra => vec!["-f".into(), file.into()],
            PlayerKind::Aplay => vec!["-q".into(), file.into()],
        }
    }
}

/// Plays a cue by spawning the system audio CLI (`pw-play` first, then
/// `paplay` / `canberra-gtk-play` / `aplay`) with the resolved file - the
/// freedesktop-standard approach, so the daemon needs no in-process decoder or
/// audio-server binding. The spawned child is reaped on a blocking task, so the
/// dispatch path is never blocked and no zombie is left. The actual audio only
/// verifies on a machine with a sound server (metal); the discovery + argument
/// build are unit-tested.
pub struct SystemSoundPlayer {
    command: PathBuf,
    kind: PlayerKind,
}

impl SystemSoundPlayer {
    /// Discover a play command on `$PATH`, or `None` if none is installed (the
    /// daemon then keeps [`NullSoundPlayer`]).
    pub fn discover() -> Option<Self> {
        let path = std::env::var_os("PATH")?;
        Self::select(&PlayerKind::CANDIDATES, |name| find_in_path(name, &path))
    }

    /// Pick the first candidate that `lookup` resolves to a binary. Pure, so the
    /// preference order is unit-tested with a fake lookup.
    fn select(
        candidates: &[(&str, PlayerKind)],
        lookup: impl Fn(&str) -> Option<PathBuf>,
    ) -> Option<Self> {
        candidates.iter().find_map(|(name, kind)| {
            lookup(name).map(|command| Self { command, kind: *kind })
        })
    }
}

impl SoundPlayer for SystemSoundPlayer {
    fn play(&self, resolution: &SoundResolution, volume: f32) {
        let SoundResolution::File(file) = resolution else {
            return;
        };
        let mut cmd = std::process::Command::new(&self.command);
        cmd.args(self.kind.args(file, volume))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        match cmd.spawn() {
            Ok(mut child) => {
                // Reap on a blocking task so the dispatch path is not blocked by
                // playback and the daemon leaves no zombie. `play` is always
                // called from the tokio dispatch path, so a runtime is present.
                tokio::task::spawn_blocking(move || {
                    let _ = child.wait();
                });
            }
            Err(e) => tracing::debug!("sound play spawn failed: {e}"),
        }
    }
}

/// Find an executable `name` on the `$PATH`-shaped `path` value, returning the
/// first `dir/name` that exists. (A plain existence check, not a full
/// executable-bit test; a non-executable shadow would surface as a spawn error.)
fn find_in_path(name: &str, path: &std::ffi::OsStr) -> Option<PathBuf> {
    std::env::split_paths(path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn the_system_player_prefers_pw_play_then_falls_down_the_chain() {
        // Only paplay + aplay present: paplay (higher in the order) wins.
        let present = ["paplay", "aplay"];
        let player = SystemSoundPlayer::select(&PlayerKind::CANDIDATES, |name| {
            present.contains(&name).then(|| PathBuf::from(format!("/usr/bin/{name}")))
        })
        .expect("a player is selected");
        assert_eq!(player.kind, PlayerKind::PaPlay);
        assert_eq!(player.command, PathBuf::from("/usr/bin/paplay"));

        // pw-play present: it wins outright.
        let player = SystemSoundPlayer::select(&PlayerKind::CANDIDATES, |name| {
            (name == "pw-play").then(|| PathBuf::from("/usr/bin/pw-play"))
        })
        .unwrap();
        assert_eq!(player.kind, PlayerKind::PwPlay);

        // Nothing installed: no player.
        assert!(SystemSoundPlayer::select(&PlayerKind::CANDIDATES, |_| None).is_none());
    }

    #[test]
    fn the_play_args_carry_the_file_and_per_tool_volume() {
        let file = Path::new("/usr/share/sounds/freedesktop/stereo/bell.oga");
        // pw-play takes a 0..1 float; full volume.
        let a = PlayerKind::PwPlay.args(file, 1.0);
        assert_eq!(a[0], "--volume");
        assert_eq!(a[1], "1.000");
        assert_eq!(a[2], file);
        // paplay takes 0..65536; half volume rounds to 32768.
        let a = PlayerKind::PaPlay.args(file, 0.5);
        assert_eq!(a[0], "--volume");
        assert_eq!(a[1], "32768");
        // canberra/aplay carry no volume, just the file.
        assert_eq!(PlayerKind::Canberra.args(file, 0.5), vec![OsString::from("-f"), file.into()]);
        assert_eq!(PlayerKind::Aplay.args(file, 0.5), vec![OsString::from("-q"), file.into()]);
    }

    #[test]
    fn find_in_path_locates_a_binary_in_a_path_dir() {
        let dir = std::env::temp_dir().join(format!("arlen-sound-path-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("pw-play");
        std::fs::write(&bin, b"#!/bin/true\n").unwrap();
        let path = std::env::join_paths([PathBuf::from("/nonexistent-xyz"), dir.clone()]).unwrap();
        assert_eq!(find_in_path("pw-play", &path), Some(bin));
        assert_eq!(find_in_path("nope-not-here", &path), None);
        std::fs::remove_dir_all(&dir).ok();
    }

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
    fn a_traversing_sound_name_cannot_escape_the_roots() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("sounds");
        put(&root, "arlen", Some("[Sound Theme]\nDirectories=stereo\n"), "stereo", None);
        // A real target OUTSIDE the sound root that a `../` name would reach:
        // sounds/arlen/stereo/ + "../../../outside/evil" + ".oga" = <tmp>/outside/evil.oga.
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("evil.oga"), b"x").unwrap();
        // The guard refuses the traversing name, so the existing outside file is
        // never resolved (without the guard this would return File(evil.oga)).
        assert_eq!(
            resolve_sound(&[root.clone()], "arlen", "../../../outside/evil"),
            SoundResolution::NotFound
        );
        // Any separator-bearing or empty name is not a valid flat sound name.
        assert_eq!(resolve_sound(&[root.clone()], "arlen", "a/b"), SoundResolution::NotFound);
        assert_eq!(resolve_sound(&[root], "arlen", ""), SoundResolution::NotFound);
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

    #[test]
    fn cue_name_honours_the_per_event_override() {
        let mut overrides = std::collections::HashMap::new();
        // No override -> the event's standard freedesktop name.
        assert_eq!(cue_name_for_event(SoundEvent::Error, &overrides), "dialog-error");
        // An override keyed on the default name redirects the cue.
        overrides.insert("dialog-error".to_string(), "x-arlen-custom-error".to_string());
        assert_eq!(cue_name_for_event(SoundEvent::Error, &overrides), "x-arlen-custom-error");
    }

    #[test]
    fn resolve_cue_uses_the_config_theme_and_override() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        // A flat theme "mytheme" with a cue file for the overridden name.
        put(&root, "mytheme", None, "", Some("custom-bell.oga"));
        let mut config = crate::config::types::SoundConfig {
            theme: "mytheme".to_string(),
            ..Default::default()
        };
        config
            .overrides
            .insert("message-new-instant".to_string(), "custom-bell".to_string());
        assert_eq!(
            resolve_cue(SoundEvent::NotificationArrived, &config, &[root.clone()]),
            SoundResolution::File(root.join("mytheme/custom-bell.oga")),
        );
    }

    #[test]
    fn cue_selection_respects_urgency_and_category() {
        use SoundEvent::*;
        // Category-specific cues win.
        assert_eq!(sound_event_for_notification(1, "device.added"), Some(DeviceAdded));
        assert_eq!(sound_event_for_notification(1, "device.removed"), Some(DeviceRemoved));
        assert_eq!(sound_event_for_notification(1, "email.error"), Some(Error));
        assert_eq!(sound_event_for_notification(1, "transfer.warning"), Some(Warning));
        // Low urgency with no special category stays silent (restraint).
        assert_eq!(sound_event_for_notification(0, ""), None);
        assert_eq!(sound_event_for_notification(0, "im.received"), None);
        // Normal and critical arrivals sound as a plain notification.
        assert_eq!(sound_event_for_notification(1, "im.received"), Some(NotificationArrived));
        assert_eq!(sound_event_for_notification(2, ""), Some(NotificationArrived));
        // A device cue sounds even at low urgency (the category is explicit).
        assert_eq!(sound_event_for_notification(0, "device.added"), Some(DeviceAdded));
    }
}
