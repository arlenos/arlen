//! MPRIS "Now Playing" backend for the top-bar applet (orphan-recovery item 5,
//! `mpris-applet-plan.md`). The shell is the universal MPRIS consumer: any app
//! exposing `org.mpris.MediaPlayer2.*` on the session bus is a producer.
//!
//! This module is the PURE core the D-Bus client feeds: the payload types the
//! `nowPlaying` store mirrors, the **active-player ranking owned by us** (not
//! `playerctld`), and the MPRIS value parsing (PlaybackStatus, the microsecond
//! time base, the `a{sv}` metadata map). The session-bus discovery
//! (`ListNames` + `NameOwnerChanged`), the per-player `PropertiesChanged`
//! subscription, the transport calls and `Position` interpolation are the D-Bus
//! wiring that consumes these (built on top, following the `bluetooth.rs`
//! session-bus pattern).
// The pure core is consumed by the D-Bus client + Tauri commands (the next
// increment); until that lands its items read as unused in the bin tree.
#![allow(dead_code)]

use std::collections::HashMap;

use serde::Serialize;
use zbus::zvariant::{OwnedValue, Value};

/// Playback status, mirroring the three MPRIS `PlaybackStatus` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackStatus {
    Playing,
    Paused,
    Stopped,
}

impl PlaybackStatus {
    /// Parse the MPRIS `PlaybackStatus` string; anything unrecognised is treated
    /// as `Stopped` (fail-safe: an unknown state never reads as actively playing).
    pub fn parse(s: &str) -> PlaybackStatus {
        match s {
            "Playing" => PlaybackStatus::Playing,
            "Paused" => PlaybackStatus::Paused,
            _ => PlaybackStatus::Stopped,
        }
    }

    /// Ranking weight for the active-player pick: playing outranks paused, paused
    /// outranks stopped.
    fn rank(self) -> u8 {
        match self {
            PlaybackStatus::Playing => 2,
            PlaybackStatus::Paused => 1,
            PlaybackStatus::Stopped => 0,
        }
    }
}

/// A registered media player, for the switcher row (mirrors `MprisPlayer` in
/// `nowPlaying.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MprisPlayer {
    /// The bus name (`org.mpris.MediaPlayer2.<player>`), the stable id.
    pub id: String,
    /// The app name (from `Identity` / `DesktopEntry`).
    pub app: String,
    /// The app icon data-URI, or null (the frontend falls back to a note glyph).
    pub icon: Option<String>,
    pub status: PlaybackStatus,
}

/// The active player's track + transport state (mirrors `NowPlaying` in
/// `nowPlaying.ts`; `camelCase` for the Tauri boundary).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub album: String,
    /// The album-art URL (`file://` loads directly; remote `https://` art is
    /// default-off per the ambient-leak rule), or null.
    pub art_url: Option<String>,
    pub status: PlaybackStatus,
    /// Elapsed + total, in SECONDS (MPRIS is microseconds; the client converts).
    pub position: f64,
    pub length: f64,
    pub can_seek: bool,
    pub can_prev: bool,
    pub can_next: bool,
    pub can_pause: bool,
    /// Transport is read-only when false.
    pub can_control: bool,
    /// All registered players, including the active one.
    pub players: Vec<MprisPlayer>,
    /// The active (or pinned) player's id.
    pub active_id: String,
}

/// Convert an MPRIS microsecond time value to seconds (the frontend's unit). A
/// negative value (some players report `-1` for unknown) clamps to `0`.
pub fn micros_to_seconds(micros: i64) -> f64 {
    if micros <= 0 {
        0.0
    } else {
        micros as f64 / 1_000_000.0
    }
}

/// Pick the active player from the registered set: the highest-ranked status
/// wins (playing > paused > stopped), ties broken deterministically by the bus id
/// so the active pick does not flicker between two equally-ranked players. A
/// `pinned` id, if still present, always wins (the user's manual override of
/// auto-follow). Returns `None` for an empty set (the applet hides).
pub fn rank_active(players: &[MprisPlayer], pinned: Option<&str>) -> Option<String> {
    if let Some(pin) = pinned {
        if players.iter().any(|p| p.id == pin) {
            return Some(pin.to_string());
        }
    }
    players
        .iter()
        .max_by(|a, b| {
            a.status
                .rank()
                .cmp(&b.status.rank())
                // A later id loses the tie so the winner is the smallest id at the
                // top rank (stable, order-independent).
                .then_with(|| b.id.cmp(&a.id))
        })
        .map(|p| p.id.clone())
}

/// The track fields parsed from an MPRIS `Metadata` (`a{sv}`) map.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackMeta {
    pub title: String,
    pub artist: String,
    pub album: String,
    /// `mpris:artUrl` (the client applies the `file://`-direct / remote-off rule).
    pub art_url: Option<String>,
    /// `mpris:length` converted from microseconds to seconds.
    pub length: f64,
}

/// Extract a string metadata field (`xesam:title`, `xesam:album`, `mpris:artUrl`).
/// Mirrors the `bluetooth.rs` `OwnedValue` -> `Value` match pattern.
fn meta_str(meta: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    match Value::try_from(meta.get(key)?.clone()).ok()? {
        Value::Str(s) => Some(s.to_string()),
        _ => None,
    }
}

/// Extract the `xesam:artist` list (`as`), joined with `", "`. Tolerates a lone
/// string (some players send `xesam:artist` as a single string).
fn meta_artist(meta: &HashMap<String, OwnedValue>) -> String {
    let Some(v) = meta
        .get("xesam:artist")
        .and_then(|v| Value::try_from(v.clone()).ok())
    else {
        return String::new();
    };
    match v {
        Value::Array(arr) => arr
            .iter()
            .filter_map(|e| match e {
                Value::Str(s) => Some(s.to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(", "),
        Value::Str(s) => s.to_string(),
        _ => String::new(),
    }
}

/// Extract `mpris:length` (microseconds, signed or unsigned) as seconds.
fn meta_length_seconds(meta: &HashMap<String, OwnedValue>) -> f64 {
    let Some(v) = meta
        .get("mpris:length")
        .and_then(|v| Value::try_from(v.clone()).ok())
    else {
        return 0.0;
    };
    match v {
        Value::I64(n) => micros_to_seconds(n),
        Value::U64(n) => micros_to_seconds(n as i64),
        Value::I32(n) => micros_to_seconds(n as i64),
        Value::U32(n) => micros_to_seconds(n as i64),
        _ => 0.0,
    }
}

/// Parse an MPRIS `Metadata` (`a{sv}`) map into the track fields the applet
/// renders. Missing fields degrade gracefully (empty title/artist/album, no art,
/// zero length) rather than failing the whole player.
pub fn parse_track(meta: &HashMap<String, OwnedValue>) -> TrackMeta {
    TrackMeta {
        title: meta_str(meta, "xesam:title").unwrap_or_default(),
        artist: meta_artist(meta),
        album: meta_str(meta, "xesam:album").unwrap_or_default(),
        art_url: meta_str(meta, "mpris:artUrl"),
        length: meta_length_seconds(meta),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(id: &str, status: PlaybackStatus) -> MprisPlayer {
        MprisPlayer {
            id: id.to_string(),
            app: id.to_string(),
            icon: None,
            status,
        }
    }

    #[test]
    fn playback_status_parses_fail_safe() {
        assert_eq!(PlaybackStatus::parse("Playing"), PlaybackStatus::Playing);
        assert_eq!(PlaybackStatus::parse("Paused"), PlaybackStatus::Paused);
        assert_eq!(PlaybackStatus::parse("Stopped"), PlaybackStatus::Stopped);
        // An unknown status is never "playing".
        assert_eq!(PlaybackStatus::parse("Buffering"), PlaybackStatus::Stopped);
    }

    #[test]
    fn micros_convert_to_seconds_and_clamp() {
        assert_eq!(micros_to_seconds(1_500_000), 1.5);
        assert_eq!(micros_to_seconds(0), 0.0);
        assert_eq!(micros_to_seconds(-1), 0.0); // unknown-position sentinel
    }

    #[test]
    fn ranking_prefers_playing_then_paused() {
        let players = vec![
            player("org.mpris.MediaPlayer2.vlc", PlaybackStatus::Paused),
            player("org.mpris.MediaPlayer2.spotify", PlaybackStatus::Playing),
            player("org.mpris.MediaPlayer2.mpv", PlaybackStatus::Stopped),
        ];
        assert_eq!(
            rank_active(&players, None).as_deref(),
            Some("org.mpris.MediaPlayer2.spotify")
        );
    }

    #[test]
    fn ranking_is_deterministic_on_a_tie() {
        // Two playing players: the smallest id wins, order-independently.
        let a = vec![
            player("org.mpris.MediaPlayer2.b", PlaybackStatus::Playing),
            player("org.mpris.MediaPlayer2.a", PlaybackStatus::Playing),
        ];
        let mut b = a.clone();
        b.reverse();
        assert_eq!(rank_active(&a, None), rank_active(&b, None));
        assert_eq!(rank_active(&a, None).as_deref(), Some("org.mpris.MediaPlayer2.a"));
    }

    #[test]
    fn a_present_pin_overrides_the_ranking() {
        let players = vec![
            player("org.mpris.MediaPlayer2.spotify", PlaybackStatus::Playing),
            player("org.mpris.MediaPlayer2.mpv", PlaybackStatus::Paused),
        ];
        // The user pinned mpv even though spotify is playing.
        assert_eq!(
            rank_active(&players, Some("org.mpris.MediaPlayer2.mpv")).as_deref(),
            Some("org.mpris.MediaPlayer2.mpv")
        );
        // A pin that is no longer registered falls back to the ranking.
        assert_eq!(
            rank_active(&players, Some("org.mpris.MediaPlayer2.gone")).as_deref(),
            Some("org.mpris.MediaPlayer2.spotify")
        );
    }

    #[test]
    fn no_players_hides_the_applet() {
        assert_eq!(rank_active(&[], None), None);
    }

    fn owned(v: Value<'static>) -> OwnedValue {
        OwnedValue::try_from(v).unwrap()
    }

    #[test]
    fn parse_track_extracts_the_metadata_fields() {
        let mut meta: HashMap<String, OwnedValue> = HashMap::new();
        meta.insert("xesam:title".into(), owned(Value::from("Song")));
        meta.insert(
            "xesam:artist".into(),
            owned(Value::from(vec!["A".to_string(), "B".to_string()])),
        );
        meta.insert("xesam:album".into(), owned(Value::from("Album")));
        meta.insert("mpris:artUrl".into(), owned(Value::from("file:///art.png")));
        meta.insert("mpris:length".into(), owned(Value::from(90_000_000i64)));
        let t = parse_track(&meta);
        assert_eq!(t.title, "Song");
        assert_eq!(t.artist, "A, B"); // the `as` list joined
        assert_eq!(t.album, "Album");
        assert_eq!(t.art_url.as_deref(), Some("file:///art.png"));
        assert_eq!(t.length, 90.0); // 90s from 90_000_000 us
    }

    #[test]
    fn parse_track_degrades_gracefully_on_missing_fields() {
        let meta: HashMap<String, OwnedValue> = HashMap::new();
        let t = parse_track(&meta);
        assert_eq!(t.title, "");
        assert_eq!(t.artist, "");
        assert_eq!(t.art_url, None);
        assert_eq!(t.length, 0.0);
    }
}
