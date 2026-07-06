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
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

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

// ── D-Bus session-bus client ──

use std::sync::Mutex;

use tauri::{AppHandle, Emitter};
use zbus::Connection;

const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";
const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";
const ROOT_IFACE: &str = "org.mpris.MediaPlayer2";

/// The user's pinned active player (manual override of auto-follow), or `None`
/// for auto. Set by `mpris_pin`; read when composing the now-playing state.
static PINNED: Mutex<Option<String>> = Mutex::new(None);

/// Remote (`https://`) album art is default-off (the ambient-leak rule): only a
/// `file://` art URL is passed through; a remote one falls back to the app icon.
fn art_for(art_url: Option<String>) -> Option<String> {
    art_url.filter(|u| u.starts_with("file://"))
}

/// List the MPRIS player bus names currently on the session bus.
async fn list_players(conn: &Connection) -> Result<Vec<String>, String> {
    let proxy = zbus::fdo::DBusProxy::new(conn)
        .await
        .map_err(|e| format!("DBus proxy: {e}"))?;
    let names = proxy
        .list_names()
        .await
        .map_err(|e| format!("ListNames: {e}"))?;
    Ok(names
        .into_iter()
        .map(|n| n.as_str().to_string())
        .filter(|n| n.starts_with(MPRIS_PREFIX))
        .collect())
}

/// One player's read state: the switcher entry plus the active-view details.
struct PlayerRead {
    player: MprisPlayer,
    track: TrackMeta,
    position: f64,
    can_seek: bool,
    can_prev: bool,
    can_next: bool,
    can_pause: bool,
    can_control: bool,
}

/// Read a single player's state. Missing properties degrade to defaults rather
/// than dropping the player.
async fn read_player(conn: &Connection, bus: &str) -> Option<PlayerRead> {
    let player = zbus::Proxy::new(conn, bus.to_owned(), MPRIS_PATH, PLAYER_IFACE)
        .await
        .ok()?;
    let root = zbus::Proxy::new(conn, bus.to_owned(), MPRIS_PATH, ROOT_IFACE)
        .await
        .ok()?;

    let status = PlaybackStatus::parse(
        &player
            .get_property::<String>("PlaybackStatus")
            .await
            .unwrap_or_default(),
    );
    let meta = player
        .get_property::<HashMap<String, OwnedValue>>("Metadata")
        .await
        .unwrap_or_default();
    let track = parse_track(&meta);
    let position = micros_to_seconds(player.get_property::<i64>("Position").await.unwrap_or(0));
    let app = root
        .get_property::<String>("Identity")
        .await
        .unwrap_or_else(|_| bus.trim_start_matches(MPRIS_PREFIX).to_string());

    Some(PlayerRead {
        player: MprisPlayer {
            id: bus.to_string(),
            app,
            icon: None,
            status,
        },
        track,
        position,
        can_seek: player.get_property::<bool>("CanSeek").await.unwrap_or(false),
        can_prev: player
            .get_property::<bool>("CanGoPrevious")
            .await
            .unwrap_or(false),
        can_next: player
            .get_property::<bool>("CanGoNext")
            .await
            .unwrap_or(false),
        can_pause: player.get_property::<bool>("CanPause").await.unwrap_or(false),
        can_control: player
            .get_property::<bool>("CanControl")
            .await
            .unwrap_or(false),
    })
}

/// Compose the now-playing state from every registered player, or `None` when no
/// player is present (the applet hides).
async fn build_now_playing(conn: &Connection) -> Option<NowPlaying> {
    let buses = list_players(conn).await.ok()?;
    let mut reads = Vec::new();
    for bus in buses {
        if let Some(r) = read_player(conn, &bus).await {
            reads.push(r);
        }
    }
    if reads.is_empty() {
        return None;
    }
    let players: Vec<MprisPlayer> = reads.iter().map(|r| r.player.clone()).collect();
    let pinned = PINNED.lock().ok().and_then(|p| p.clone());
    let active_id = rank_active(&players, pinned.as_deref())?;
    let active = reads.iter().find(|r| r.player.id == active_id)?;

    Some(NowPlaying {
        title: active.track.title.clone(),
        artist: active.track.artist.clone(),
        album: active.track.album.clone(),
        art_url: art_for(active.track.art_url.clone()),
        status: active.player.status,
        position: active.position,
        length: active.track.length,
        can_seek: active.can_seek,
        can_prev: active.can_prev,
        can_next: active.can_next,
        can_pause: active.can_pause,
        can_control: active.can_control,
        players,
        active_id,
    })
}

/// Call a no-argument transport method on a player.
async fn transport(bus: &str, method: &str) -> Result<(), String> {
    let conn = Connection::session()
        .await
        .map_err(|e| format!("session bus: {e}"))?;
    let player = zbus::Proxy::new(&conn, bus.to_owned(), MPRIS_PATH, PLAYER_IFACE)
        .await
        .map_err(|e| format!("player proxy: {e}"))?;
    player
        .call_method(method, &())
        .await
        .map_err(|e| format!("{method}: {e}"))?;
    Ok(())
}

/// Fetch the current now-playing state (the pull path the applet reads on mount).
#[tauri::command]
pub async fn mpris_now_playing() -> Result<Option<NowPlaying>, String> {
    let conn = Connection::session()
        .await
        .map_err(|e| format!("session bus: {e}"))?;
    Ok(build_now_playing(&conn).await)
}

/// Toggle play/pause on a player.
#[tauri::command]
pub async fn mpris_play_pause(id: String) -> Result<(), String> {
    transport(&id, "PlayPause").await
}

/// Skip to the next track.
#[tauri::command]
pub async fn mpris_next(id: String) -> Result<(), String> {
    transport(&id, "Next").await
}

/// Skip to the previous track.
#[tauri::command]
pub async fn mpris_previous(id: String) -> Result<(), String> {
    transport(&id, "Previous").await
}

/// Extract the current track's id (`mpris:trackid`, an object path) from a
/// Metadata map - required by MPRIS `SetPosition`. Tolerates a player that (out
/// of spec) sends the trackid as a string.
fn meta_trackid(meta: &HashMap<String, OwnedValue>) -> Option<OwnedObjectPath> {
    match Value::try_from(meta.get("mpris:trackid")?.clone()).ok()? {
        Value::ObjectPath(p) => Some(p.into()),
        Value::Str(s) => ObjectPath::try_from(s.as_str())
            .ok()
            .map(|p| p.into_owned().into()),
        _ => None,
    }
}

/// Seek the active player to `seconds`. MPRIS `SetPosition` is absolute and keyed
/// on the current track's id, read from the player's Metadata; a player that
/// reports no trackid (or `can_seek=false`) cannot be sought.
#[tauri::command]
pub async fn mpris_set_position(id: String, seconds: f64) -> Result<(), String> {
    let conn = Connection::session()
        .await
        .map_err(|e| format!("session bus: {e}"))?;
    let player = zbus::Proxy::new(&conn, id, MPRIS_PATH, PLAYER_IFACE)
        .await
        .map_err(|e| format!("player proxy: {e}"))?;
    let meta = player
        .get_property::<HashMap<String, OwnedValue>>("Metadata")
        .await
        .map_err(|e| format!("Metadata: {e}"))?;
    let trackid = meta_trackid(&meta).ok_or_else(|| "no mpris:trackid; cannot seek".to_string())?;
    let micros = (seconds.max(0.0) * 1_000_000.0) as i64;
    player
        .call_method("SetPosition", &(trackid, micros))
        .await
        .map_err(|e| format!("SetPosition: {e}"))?;
    Ok(())
}

/// Pin (or, with `None`, un-pin) a player as the active one.
#[tauri::command]
pub fn mpris_pin(id: Option<String>) {
    if let Ok(mut p) = PINNED.lock() {
        *p = id;
    }
}

/// Poll the session bus and emit `mpris://now-playing` so the applet tracks the
/// live state. MPRIS emits `PropertiesChanged`, but a short poll is simpler and
/// robust across players that under-report; the emit carries the full payload.
pub fn start_monitor(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let Ok(conn) = Connection::session().await else {
            return;
        };
        loop {
            let state = build_now_playing(&conn).await;
            let _ = app.emit("mpris://now-playing", state);
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
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

    #[test]
    fn meta_trackid_reads_the_object_path() {
        let mut meta: HashMap<String, OwnedValue> = HashMap::new();
        meta.insert(
            "mpris:trackid".into(),
            owned(Value::ObjectPath(
                ObjectPath::try_from("/org/mpris/track/1").unwrap(),
            )),
        );
        assert_eq!(meta_trackid(&meta).unwrap().as_str(), "/org/mpris/track/1");
        // A player that sends no trackid cannot be sought.
        assert!(meta_trackid(&HashMap::new()).is_none());
    }

    #[test]
    fn remote_art_is_dropped_local_art_is_kept() {
        // The ambient-leak rule: only file:// art is passed through.
        assert_eq!(
            art_for(Some("file:///home/u/art.png".into())).as_deref(),
            Some("file:///home/u/art.png")
        );
        assert_eq!(art_for(Some("https://cdn/art.png".into())), None);
        assert_eq!(art_for(None), None);
    }
}
