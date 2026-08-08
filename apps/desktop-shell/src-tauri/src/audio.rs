//! Audio state and control, over the PipeWire CLIs.
//!
//! Nineteen subprocesses: thirteen `pactl` and six `wpctl`. `wpctl` reads and
//! sets the default sink's volume and mute; `pactl` does everything with a
//! shape - the sink and source lists, per-app streams, device descriptions -
//! because it prints something parseable and `wpctl` does not.
//!
//! **This is the last of the desktop backends still driven by a CLI, and unlike
//! its siblings D-Bus is not the way out.** Power reaches logind and UPower on
//! the system bus, and network has NetworkManager there; PipeWire has no D-Bus
//! interface of its own, so replacing these means libpipewire through the
//! `pipewire` crate - a native dependency and an event loop, not a proxy call.
//! Recorded here so the next person does not go looking for a bus name that
//! does not exist.

use serde::{Deserialize, Serialize};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Current audio status.
#[derive(Clone, Serialize, Deserialize)]
pub struct AudioStatus {
    /// Volume level 0-100.
    pub volume: u8,
    /// Whether the sink is muted.
    pub muted: bool,
    /// Output device type: "speakers", "headphones", "bluetooth_headphones",
    /// "bluetooth_speaker", "hdmi", or "unknown".
    #[serde(default)]
    pub output_type: String,
}

/// Short-TTL cache for [`get_audio_status`]. The indicator polls this
/// every 30s (with event-freshness gating) and the popover reads it
/// on open; without caching, opening the popover right after the
/// indicator polled double-counts the wpctl+pactl subprocesses. TTL
/// is deliberately short so volume changes made outside the shell
/// (e.g. via CLI) still surface within ~1s on the next read.
const AUDIO_STATUS_TTL: Duration = Duration::from_millis(800);

static AUDIO_STATUS_CACHE: OnceLock<Mutex<Option<(Instant, AudioStatus)>>> =
    OnceLock::new();

fn audio_status_cache() -> &'static Mutex<Option<(Instant, AudioStatus)>> {
    AUDIO_STATUS_CACHE.get_or_init(|| Mutex::new(None))
}

/// Invalidate the cached `AudioStatus`. Call this after any mutation
/// that changes volume/mute so the next poll reflects the change
/// rather than serving the pre-change value from cache.
fn invalidate_audio_status_cache() {
    if let Ok(mut guard) = audio_status_cache().lock() {
        *guard = None;
    }
}

/// Returns the current volume, mute state, and output device type.
/// Async + `spawn_blocking` so the two subprocess spawns inside don't
/// pin a Tauri worker thread — this command fires on every
/// `audio-changed` event, so the per-call cost adds up under volume
/// scrubbing.
#[tauri::command]
pub async fn get_audio_status() -> Result<AudioStatus, String> {
    tauri::async_runtime::spawn_blocking(get_audio_status_impl)
        .await
        .map_err(|e| format!("audio task join: {e}"))?
}

fn get_audio_status_impl() -> Result<AudioStatus, String> {
    // Cache hit → return immediately. The event-driven polling pattern
    // in the frontend already throttles to ~one call per 30s, but
    // AudioPopover + AudioIndicator occasionally race.
    if let Ok(guard) = audio_status_cache().lock() {
        if let Some((fetched_at, ref status)) = *guard {
            if fetched_at.elapsed() < AUDIO_STATUS_TTL {
                return Ok(status.clone());
            }
        }
    }

    let output = std::process::Command::new("wpctl")
        .args(["get-volume", "@DEFAULT_AUDIO_SINK@"])
        .output()
        .map_err(|e| format!("wpctl not found: {e}"))?;

    if !output.status.success() {
        return Err("wpctl get-volume failed".into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stdout = stdout.trim();

    let muted = stdout.contains("[MUTED]");
    let volume_str = stdout
        .strip_prefix("Volume: ")
        .unwrap_or("0")
        .split_whitespace()
        .next()
        .unwrap_or("0");

    let volume_f: f64 = volume_str.parse().unwrap_or(0.0);
    let volume = (volume_f * 100.0).round().clamp(0.0, 100.0) as u8;

    let output_type = detect_output_type();

    let status = AudioStatus {
        volume,
        muted,
        output_type,
    };

    if let Ok(mut guard) = audio_status_cache().lock() {
        *guard = Some((Instant::now(), status.clone()));
    }

    Ok(status)
}

/// Detect the type of the default audio output device.
///
/// Checks the default sink's name and properties to determine if it's
/// Bluetooth headphones, Bluetooth speaker, HDMI, or regular speakers.
fn detect_output_type() -> String {
    let default_sink = std::process::Command::new("pactl")
        .args(["get-default-sink"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    if default_sink.is_empty() {
        return "speakers".into();
    }

    let lower = default_sink.to_lowercase();

    // Bluetooth devices have "bluez" in the sink name.
    if lower.contains("bluez") {
        // Try to determine headphones vs speaker from sink properties.
        let props = get_sink_form_factor(&default_sink);
        return match props.as_str() {
            "headphone" | "headset" | "headphones" => "bluetooth_headphones".into(),
            "speaker" => "bluetooth_speaker".into(),
            _ => {
                // Fallback: guess from name.
                if lower.contains("speaker") || lower.contains("boom") {
                    "bluetooth_speaker".into()
                } else {
                    "bluetooth_headphones".into()
                }
            }
        };
    }

    if lower.contains("hdmi") {
        return "hdmi".into();
    }

    "speakers".into()
}

/// The `device.form_factor` of a named sink, which decides the output icon.
///
/// JSON, like the two device lists: the sink's own properties are strings we did
/// not choose, and the text listing's record boundaries can be forged by a value
/// containing a newline. The stake here is only which icon is drawn, but it is
/// the last parse in this file that read the text form, and leaving one behind
/// invites the next reader to copy it.
fn get_sink_form_factor(sink_name: &str) -> String {
    let output = match std::process::Command::new("pactl")
        .args(["-f", "json", "list", "sinks"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return String::new(),
    };

    let sinks: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).unwrap_or_default();
    sinks
        .iter()
        .find(|sink| sink.get("name").and_then(|n| n.as_str()) == Some(sink_name))
        .and_then(|sink| sink.get("properties"))
        .and_then(|props| props.get("device.form_factor"))
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string()
}

/// Sets the volume of the default audio sink (0-100).
#[tauri::command]
pub async fn set_audio_volume(volume: u8) -> Result<(), String> {
    let value = format!("{:.2}", volume as f64 / 100.0);
    let status = tokio::process::Command::new("wpctl")
        .args(["set-volume", "@DEFAULT_AUDIO_SINK@", &value])
        .status()
        .await
        .map_err(|e| format!("wpctl set-volume failed: {e}"))?;

    if !status.success() {
        return Err("wpctl set-volume returned non-zero".into());
    }
    invalidate_audio_status_cache();
    Ok(())
}

/// A single audio output device.
#[derive(Clone, Serialize, Deserialize)]
pub struct AudioOutput {
    /// Pipewire node ID.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Whether this is the current default sink.
    pub is_default: bool,
}

/// Returns available audio output devices with human-readable names.
/// No longer a `#[tauri::command]`: `get_audio_full_state` bundles this and its
/// four siblings into one round-trip and is what the popover calls, so nothing
/// invoked this from a webview. It stays as a function because that bundle calls
/// it. Every registered command is reachable from the renderer, so one nothing
/// needs is IPC surface for free.
pub async fn get_audio_outputs() -> Result<Vec<AudioOutput>, String> {
    tauri::async_runtime::spawn_blocking(get_audio_outputs_impl)
        .await
        .map_err(|e| format!("audio task join: {e}"))?
}

fn get_audio_outputs_impl() -> Result<Vec<AudioOutput>, String> {
    // Get default sink name.
    let default_name = std::process::Command::new("pactl")
        .args(["get-default-sink"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    // JSON, not the text listing, and for the same reason the per-app list uses
    // it: a sink's name and description are strings we did not choose - any
    // client may create one with `module-null-sink` and set them - and libpulse
    // writes property values out raw, newlines included. A line-oriented parse of
    // the text form therefore lets one forge a record, which here would put a
    // device of its choosing in the output picker with an id the next
    // `set-default-sink` would then use. In JSON the record boundaries come from
    // the structure and a newline in a value is escaped.
    let output = std::process::Command::new("pactl")
        .args(["-f", "json", "list", "sinks"])
        .output()
        .map_err(|e| format!("pactl not found: {e}"))?;

    if !output.status.success() {
        return Err("pactl list sinks failed".into());
    }

    let sinks: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).unwrap_or_default();
    let outputs = sinks
        .iter()
        .filter_map(|sink| {
            let id = sink.get("name")?.as_str()?.to_string();
            let name = sink
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or(&id)
                .to_string();
            Some(AudioOutput {
                is_default: id == default_name,
                id,
                name,
            })
        })
        .collect();

    Ok(outputs)
}

/// Sets the default audio output device by sink name.
#[tauri::command]
pub async fn set_audio_output(id: String) -> Result<(), String> {
    let status = tokio::process::Command::new("pactl")
        .args(["set-default-sink", &id])
        .status()
        .await
        .map_err(|e| format!("pactl set-default-sink failed: {e}"))?;

    if !status.success() {
        return Err(format!("pactl set-default-sink {id} failed"));
    }
    Ok(())
}

/// A single audio input device.
#[derive(Clone, Serialize, Deserialize)]
pub struct AudioInput {
    /// PulseAudio source name.
    pub id: String,
    /// Human-readable description.
    pub name: String,
    /// Whether this is the current default source.
    pub is_default: bool,
}

/// Returns available audio input devices (microphones).
/// Filters out monitor sources.
/// No longer a `#[tauri::command]`: `get_audio_full_state` bundles this and its
/// four siblings into one round-trip and is what the popover calls, so nothing
/// invoked this from a webview. It stays as a function because that bundle calls
/// it. Every registered command is reachable from the renderer, so one nothing
/// needs is IPC surface for free.
pub async fn get_audio_inputs() -> Result<Vec<AudioInput>, String> {
    tauri::async_runtime::spawn_blocking(get_audio_inputs_impl)
        .await
        .map_err(|e| format!("audio task join: {e}"))?
}

fn get_audio_inputs_impl() -> Result<Vec<AudioInput>, String> {
    let default_src = std::process::Command::new("pactl")
        .args(["get-default-source"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    // JSON for the same reason as the sink list: a source's name and description
    // are not ours, and the text form's record boundaries can be forged by a
    // value containing a newline.
    let output = std::process::Command::new("pactl")
        .args(["-f", "json", "list", "sources"])
        .output()
        .map_err(|e| format!("pactl not found: {e}"))?;

    if !output.status.success() {
        return Err("pactl list sources failed".into());
    }

    let sources: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).unwrap_or_default();
    let inputs = sources
        .iter()
        .filter_map(|source| {
            let id = source.get("name")?.as_str()?.to_string();
            // Monitor sources are the loopback of an output, not a microphone.
            if id.contains(".monitor") {
                return None;
            }
            let name = source
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or(&id)
                .to_string();
            Some(AudioInput {
                is_default: id == default_src,
                id,
                name,
            })
        })
        .collect();

    Ok(inputs)
}

/// Sets the default audio input device.
#[tauri::command]
pub async fn set_audio_input(id: String) -> Result<(), String> {
    let status = tokio::process::Command::new("pactl")
        .args(["set-default-source", &id])
        .status()
        .await
        .map_err(|e| format!("pactl set-default-source failed: {e}"))?;

    if !status.success() {
        return Err(format!("pactl set-default-source {id} failed"));
    }
    Ok(())
}

/// Returns the current input (microphone) volume and mute state.
/// No longer a `#[tauri::command]`: `get_audio_full_state` bundles this and its
/// four siblings into one round-trip and is what the popover calls, so nothing
/// invoked this from a webview. It stays as a function because that bundle calls
/// it. Every registered command is reachable from the renderer, so one nothing
/// needs is IPC surface for free.
pub async fn get_input_volume() -> Result<AudioStatus, String> {
    tauri::async_runtime::spawn_blocking(get_input_volume_impl)
        .await
        .map_err(|e| format!("audio task join: {e}"))?
}

fn get_input_volume_impl() -> Result<AudioStatus, String> {
    let output = std::process::Command::new("wpctl")
        .args(["get-volume", "@DEFAULT_AUDIO_SOURCE@"])
        .output()
        .map_err(|e| format!("wpctl: {e}"))?;

    if !output.status.success() {
        return Err("wpctl get-volume source failed".into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stdout = stdout.trim();
    let muted = stdout.contains("[MUTED]");
    let volume_str = stdout
        .strip_prefix("Volume: ")
        .unwrap_or("0")
        .split_whitespace()
        .next()
        .unwrap_or("0");
    let volume_f: f64 = volume_str.parse().unwrap_or(0.0);
    let volume = (volume_f * 100.0).round().clamp(0.0, 100.0) as u8;

    Ok(AudioStatus {
        volume,
        muted,
        output_type: String::new(),
    })
}

/// Sets the input (microphone) volume (0-100).
#[tauri::command]
pub async fn set_input_volume(volume: u8) -> Result<(), String> {
    let value = format!("{:.2}", volume as f64 / 100.0);
    let status = tokio::process::Command::new("wpctl")
        .args(["set-volume", "@DEFAULT_AUDIO_SOURCE@", &value])
        .status()
        .await
        .map_err(|e| format!("wpctl: {e}"))?;
    if !status.success() {
        return Err("wpctl set-volume source failed".into());
    }
    Ok(())
}

/// Toggles mute on the default audio source (microphone).
#[tauri::command]
pub async fn toggle_input_mute() -> Result<(), String> {
    let status = tokio::process::Command::new("wpctl")
        .args(["set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"])
        .status()
        .await
        .map_err(|e| format!("wpctl: {e}"))?;
    if !status.success() {
        return Err("wpctl set-mute source failed".into());
    }
    Ok(())
}

/// A running application with audio output.
#[derive(Clone, Serialize, Deserialize)]
pub struct AppVolume {
    /// PulseAudio sink-input index.
    pub id: u32,
    /// Application name.
    pub name: String,
    /// Volume level 0-100.
    pub volume: u8,
    /// Resolved icon as base64 data URL (from Freedesktop icon theme).
    pub icon_data: Option<String>,
}

/// Returns all running applications that are playing audio.
/// No longer a `#[tauri::command]`: `get_audio_full_state` bundles this and its
/// four siblings into one round-trip and is what the popover calls, so nothing
/// invoked this from a webview. It stays as a function because that bundle calls
/// it. Every registered command is reachable from the renderer, so one nothing
/// needs is IPC surface for free.
pub async fn get_app_volumes() -> Result<Vec<AppVolume>, String> {
    tauri::async_runtime::spawn_blocking(get_app_volumes_impl)
        .await
        .map_err(|e| format!("audio task join: {e}"))?
}

fn get_app_volumes_impl() -> Result<Vec<AppVolume>, String> {
    let output = std::process::Command::new("pactl")
        .args(["-f", "json", "list", "sink-inputs"])
        .output()
        .map_err(|e| format!("pactl: {e}"))?;

    if !output.status.success() {
        // No text-format fallback, deliberately. The one that was here parsed
        // `pactl list sink-inputs` line by line, and a line beginning
        // `Sink Input #` started a record - while `application.name` is a string
        // the application itself chooses and libpulse writes it out RAW.
        // Measured in-process against libpulse 17: a value of
        // "Innocent\nSink Input #999" formats as two lines, the second at column
        // zero. Any app could therefore forge an entry in this list, with a name
        // and icon of its choosing and an id naming somebody else's stream -
        // which is what the volume slider then moves.
        //
        // The JSON form has no such seam: record boundaries come from the
        // structure and a newline inside a value is escaped. `-f json` has been
        // in pactl since PulseAudio 15, so that fallback only served systems from
        // before 2021, and carrying a forgeable parser for them is the kind of
        // shim this tree does not keep.
        return Err("pactl does not support -f json".into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let entries: Vec<serde_json::Value> =
        serde_json::from_str(&stdout).unwrap_or_default();

    let mut apps = Vec::new();
    for entry in entries {
        let index = entry.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let name = entry
            .get("properties")
            .and_then(|p| p.get("application.name"))
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string();
        let vol_pct = entry
            .get("volume")
            .and_then(|v| v.as_object())
            .and_then(|obj| obj.values().next())
            .and_then(|ch| ch.get("value_percent"))
            .and_then(|v| v.as_str())
            .and_then(|s| s.trim_end_matches('%').parse::<u8>().ok())
            .unwrap_or(100);

        let props = entry.get("properties");
        let icon_name = props
            .and_then(|p| p.get("application.icon_name"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let binary = props
            .and_then(|p| p.get("application.process.binary"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // Try icon_name, then binary name, then app name as icon lookup.
        let icon_data = [icon_name, binary, &name.to_lowercase()]
            .iter()
            .filter(|s| !s.is_empty())
            .find_map(|s| crate::shell_overlay_client::resolve_app_icon(s.to_string()));

        if !name.is_empty() {
            apps.push(AppVolume {
                id: index,
                name,
                volume: vol_pct,
                icon_data,
            });
        }
    }

    Ok(apps)
}

/// Sets the volume for a specific application (sink-input).
#[tauri::command]
pub async fn set_app_volume(id: u32, volume: u8) -> Result<(), String> {
    let status = tokio::process::Command::new("pactl")
        .args([
            "set-sink-input-volume",
            &id.to_string(),
            &format!("{volume}%"),
        ])
        .status()
        .await
        .map_err(|e| format!("pactl: {e}"))?;
    if !status.success() {
        return Err(format!("pactl set-sink-input-volume {id} failed"));
    }
    Ok(())
}

/// Set Do Not Disturb state. Emits `dnd-changed` Tauri event.
#[tauri::command]
pub fn set_dnd_enabled(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    use tauri::Emitter;
    let _ = app.emit("dnd-changed", enabled);
    log::info!("DND set to {enabled}");
    Ok(())
}

/// Toggles mute on the default audio sink.
#[tauri::command]
pub async fn toggle_audio_mute() -> Result<(), String> {
    let status = tokio::process::Command::new("wpctl")
        .args(["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"])
        .status()
        .await
        .map_err(|e| format!("wpctl set-mute failed: {e}"))?;

    if !status.success() {
        return Err("wpctl set-mute returned non-zero".into());
    }
    invalidate_audio_status_cache();
    Ok(())
}

// ---------------------------------------------------------------------------
// Signal monitor via pactl subscribe
// ---------------------------------------------------------------------------

/// Everything the AudioPopover needs in a single IPC call.
///
/// Replaces the previous pattern of 5 parallel `invoke()` calls from
/// the frontend (get_audio_status + get_input_volume + get_audio_outputs
/// + get_audio_inputs + get_app_volumes), each of which spawned 2-3
/// subprocesses. This batch command still calls the same functions
/// internally but removes 4 IPC round-trips.
#[derive(Clone, Serialize)]
pub struct AudioFullState {
    pub status: AudioStatus,
    pub input_status: AudioStatus,
    pub outputs: Vec<AudioOutput>,
    pub inputs: Vec<AudioInput>,
    pub apps: Vec<AppVolume>,
}

/// Bundle the five AudioPopover reads into one Tauri round-trip.
///
/// Previously this was a sync function calling its five sub-commands
/// sequentially — ~7-8 subprocess spawns × ~15ms each = ~100-150ms
/// of blocking time per popover open. Now every sub-call runs on its
/// own `spawn_blocking` task so the pactl/wpctl subprocesses execute
/// in parallel; total latency drops to the max of the five, roughly
/// 20-30ms.
#[tauri::command]
pub async fn get_audio_full_state() -> Result<AudioFullState, String> {
    let h_status = tauri::async_runtime::spawn_blocking(get_audio_status_impl);
    let h_input = tauri::async_runtime::spawn_blocking(get_input_volume_impl);
    let h_outputs = tauri::async_runtime::spawn_blocking(get_audio_outputs_impl);
    let h_inputs = tauri::async_runtime::spawn_blocking(get_audio_inputs_impl);
    let h_apps = tauri::async_runtime::spawn_blocking(get_app_volumes_impl);

    let status = h_status.await.map_err(|e| format!("audio task join: {e}"))??;
    let input_status = h_input.await.map_err(|e| format!("audio task join: {e}"))??;
    let outputs = h_outputs.await.map_err(|e| format!("audio task join: {e}"))??;
    let inputs = h_inputs.await.map_err(|e| format!("audio task join: {e}"))??;
    let apps = h_apps.await.map_err(|e| format!("audio task join: {e}"))??;

    Ok(AudioFullState {
        status,
        input_status,
        outputs,
        inputs,
        apps,
    })
}

/// Whether a `pactl subscribe` line signals the device SET changed (a sink or
/// source was added or removed) rather than a value change (volume/mute/default).
/// Drives the `audio.state` `device_list_changed` flag so a consumer knows to
/// re-fetch the device list, not just re-read the volume.
fn audio_event_is_device_change(line: &str) -> bool {
    line.contains("'new'") || line.contains("'remove'")
}

/// Read a single trimmed `pactl` value (e.g. `get-default-sink`), or `None`.
fn pactl_value(arg: &str) -> Option<String> {
    let out = std::process::Command::new("pactl").arg(arg).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!v.is_empty()).then_some(v)
}

/// Build the `audio.state` snapshot for the event bus from the current default
/// sink/source. Returns `None` if the core volume/mute read fails (then no
/// snapshot is published, rather than a half-filled one).
fn build_audio_state(device_list_changed: bool) -> Option<crate::projects::proto::AudioStatePayload> {
    let status = get_audio_status_impl().ok()?;
    Some(crate::projects::proto::AudioStatePayload {
        default_sink: pactl_value("get-default-sink").unwrap_or_default(),
        default_source: pactl_value("get-default-source").unwrap_or_default(),
        volume: status.volume as u32,
        muted: status.muted,
        device_list_changed,
    })
}

/// Publish the current audio snapshot on the event bus (SST-R2). Best-effort:
/// a failed read simply skips this publish.
fn emit_audio_state(device_list_changed: bool) {
    use prost::Message;
    if let Some(payload) = build_audio_state(device_list_changed) {
        crate::projects::emit_to_event_bus("audio.state", payload.encode_to_vec());
    }
}

/// Start monitoring PulseAudio/PipeWire events for audio state changes.
///
/// Uses `pactl subscribe` which outputs a line on every sink/source change.
/// Emits an `audio-changed` Tauri event (for the popover) and publishes an
/// `audio.state` snapshot on the event bus (SST-R2, for apps/AI), both
/// debounced to one per change burst.
pub fn start_monitor(app: tauri::AppHandle) {
    std::thread::Builder::new()
        .name("audio-monitor".into())
        .spawn(move || {
            run_audio_monitor(app);
        })
        .expect("failed to spawn audio monitor thread");
}

fn run_audio_monitor(app: tauri::AppHandle) {
    use std::io::BufRead;
    use tauri::Emitter;

    loop {
        let child = match std::process::Command::new("pactl")
            .args(["subscribe"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                log::warn!("audio: pactl subscribe failed: {e}, retrying in 5s");
                std::thread::sleep(std::time::Duration::from_secs(5));
                continue;
            }
        };

        log::info!("audio: pactl subscribe monitor started");

        let Some(stdout) = child.stdout else {
            log::error!("audio: pactl stdout not piped");
            std::thread::sleep(std::time::Duration::from_secs(2));
            continue;
        };
        let reader = std::io::BufReader::new(stdout);

        // Publish the current snapshot once on (re)connect so a consumer that
        // subscribed after the last change still gets the present audio state
        // without waiting for the next one (net/audio have no pull fallback,
        // unlike power's org.arlen.Power1).
        emit_audio_state(false);

        // Debounce: PulseAudio fires bursts of events for a single
        // user action (e.g. a volume change emits 3-5 events in <50ms).
        // Coalesce into one frontend event per 150ms window.
        let mut last_emit = std::time::Instant::now()
            - std::time::Duration::from_secs(1);
        // Accumulated across a debounce window: if any line in it added/removed a
        // device, the published snapshot's device_list_changed is set.
        let mut pending_device_change = false;

        for line in reader.lines() {
            let Ok(line) = line else { break };
            // pactl subscribe outputs lines like:
            // Event 'change' on sink #123
            // Event 'new' on source #456
            if line.contains("sink") || line.contains("source") || line.contains("server") {
                pending_device_change |= audio_event_is_device_change(&line);
                let now = std::time::Instant::now();
                if now.duration_since(last_emit) >= std::time::Duration::from_millis(150) {
                    let _ = app.emit("audio-changed", ());
                    emit_audio_state(pending_device_change);
                    pending_device_change = false;
                    last_emit = now;
                }
            }
        }

        log::warn!("audio: pactl subscribe ended, reconnecting in 2s");
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

#[cfg(test)]
mod tests {
    use super::audio_event_is_device_change;

    #[test]
    fn device_change_is_detected_for_new_and_remove_only() {
        assert!(audio_event_is_device_change("Event 'new' on sink #42"));
        assert!(audio_event_is_device_change("Event 'remove' on source #7"));
        // A value change (volume/mute/default) is not a device-set change.
        assert!(!audio_event_is_device_change("Event 'change' on sink #42"));
        assert!(!audio_event_is_device_change("Event 'change' on server #0"));
    }
}