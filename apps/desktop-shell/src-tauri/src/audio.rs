/// Audio volume control via wpctl (wireplumber).
///
/// Reads and sets the default audio sink volume using the `wpctl` CLI.

use serde::{Deserialize, Serialize};

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

// TODO: Batch refactor — combine get_audio_status + get_audio_outputs +
// get_audio_inputs + get_app_volumes into a single get_audio_full_state()
// command that runs 3-4 subprocesses instead of the current 10-15 when
// AudioPopover opens. Each command currently spawns its own wpctl/pactl.

/// Returns the current volume, mute state, and output device type.
#[tauri::command]
pub fn get_audio_status() -> Result<AudioStatus, String> {
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

    Ok(AudioStatus {
        volume,
        muted,
        output_type,
    })
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

/// Get the form_factor property of a PulseAudio sink.
fn get_sink_form_factor(sink_name: &str) -> String {
    let output = match std::process::Command::new("pactl")
        .args(["list", "sinks"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return String::new(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut in_target_sink = false;

    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(name) = trimmed.strip_prefix("Name: ") {
            in_target_sink = name == sink_name;
        }
        if in_target_sink {
            if let Some(val) = trimmed.strip_prefix("device.form_factor = ") {
                return val.trim_matches('"').to_string();
            }
        }
        // Stop at next sink.
        if trimmed.starts_with("Sink #") && in_target_sink {
            break;
        }
    }

    String::new()
}

/// Sets the volume of the default audio sink (0-100).
#[tauri::command]
pub fn set_audio_volume(volume: u8) -> Result<(), String> {
    let value = format!("{:.2}", volume as f64 / 100.0);
    let status = std::process::Command::new("wpctl")
        .args(["set-volume", "@DEFAULT_AUDIO_SINK@", &value])
        .status()
        .map_err(|e| format!("wpctl set-volume failed: {e}"))?;

    if !status.success() {
        return Err("wpctl set-volume returned non-zero".into());
    }
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
#[tauri::command]
pub fn get_audio_outputs() -> Result<Vec<AudioOutput>, String> {
    // Get default sink name.
    let default_name = std::process::Command::new("pactl")
        .args(["get-default-sink"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    // Parse `pactl list sinks` for Name + Description per sink.
    let output = std::process::Command::new("pactl")
        .args(["list", "sinks"])
        .output()
        .map_err(|e| format!("pactl not found: {e}"))?;

    if !output.status.success() {
        return Err("pactl list sinks failed".into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut outputs = Vec::new();
    let mut current_name = String::new();
    let mut current_desc = String::new();

    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(name) = trimmed.strip_prefix("Name: ") {
            current_name = name.to_string();
        } else if let Some(desc) = trimmed.strip_prefix("Description: ") {
            current_desc = desc.to_string();
            // We have both Name and Description, emit the entry.
            if !current_name.is_empty() {
                outputs.push(AudioOutput {
                    id: current_name.clone(),
                    name: current_desc.clone(),
                    is_default: current_name == default_name,
                });
            }
        } else if trimmed.starts_with("Sink #") {
            current_name.clear();
            current_desc.clear();
        }
    }

    Ok(outputs)
}

/// Sets the default audio output device by sink name.
#[tauri::command]
pub fn set_audio_output(id: String) -> Result<(), String> {
    let status = std::process::Command::new("pactl")
        .args(["set-default-sink", &id])
        .status()
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
#[tauri::command]
pub fn get_audio_inputs() -> Result<Vec<AudioInput>, String> {
    let default_src = std::process::Command::new("pactl")
        .args(["get-default-source"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    let output = std::process::Command::new("pactl")
        .args(["list", "sources"])
        .output()
        .map_err(|e| format!("pactl not found: {e}"))?;

    if !output.status.success() {
        return Err("pactl list sources failed".into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut inputs = Vec::new();
    let mut current_name = String::new();
    let mut current_desc = String::new();

    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(name) = trimmed.strip_prefix("Name: ") {
            current_name = name.to_string();
        } else if let Some(desc) = trimmed.strip_prefix("Description: ") {
            current_desc = desc.to_string();
            // Filter out monitor sources (they contain ".monitor").
            if !current_name.contains(".monitor") && !current_name.is_empty() {
                inputs.push(AudioInput {
                    id: current_name.clone(),
                    name: current_desc.clone(),
                    is_default: current_name == default_src,
                });
            }
        } else if trimmed.starts_with("Source #") {
            current_name.clear();
            current_desc.clear();
        }
    }

    Ok(inputs)
}

/// Sets the default audio input device.
#[tauri::command]
pub fn set_audio_input(id: String) -> Result<(), String> {
    let status = std::process::Command::new("pactl")
        .args(["set-default-source", &id])
        .status()
        .map_err(|e| format!("pactl set-default-source failed: {e}"))?;

    if !status.success() {
        return Err(format!("pactl set-default-source {id} failed"));
    }
    Ok(())
}

/// Returns the current input (microphone) volume and mute state.
#[tauri::command]
pub fn get_input_volume() -> Result<AudioStatus, String> {
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
pub fn set_input_volume(volume: u8) -> Result<(), String> {
    let value = format!("{:.2}", volume as f64 / 100.0);
    let status = std::process::Command::new("wpctl")
        .args(["set-volume", "@DEFAULT_AUDIO_SOURCE@", &value])
        .status()
        .map_err(|e| format!("wpctl: {e}"))?;
    if !status.success() {
        return Err("wpctl set-volume source failed".into());
    }
    Ok(())
}

/// Toggles mute on the default audio source (microphone).
#[tauri::command]
pub fn toggle_input_mute() -> Result<(), String> {
    let status = std::process::Command::new("wpctl")
        .args(["set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"])
        .status()
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
#[tauri::command]
pub fn get_app_volumes() -> Result<Vec<AppVolume>, String> {
    let output = std::process::Command::new("pactl")
        .args(["-f", "json", "list", "sink-inputs"])
        .output()
        .map_err(|e| format!("pactl: {e}"))?;

    if !output.status.success() {
        // Fallback: pactl without JSON (older versions).
        return get_app_volumes_legacy();
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

/// Legacy fallback for pactl without JSON support.
fn get_app_volumes_legacy() -> Result<Vec<AppVolume>, String> {
    let output = std::process::Command::new("pactl")
        .args(["list", "sink-inputs"])
        .output()
        .map_err(|e| format!("pactl: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut apps = Vec::new();
    let mut current_id: Option<u32> = None;
    let mut current_name = String::new();
    let mut current_vol: u8 = 100;
    let mut current_icon: Option<String> = None;
    let mut current_binary: Option<String> = None;

    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Sink Input #") {
            if let Some(id) = current_id {
                if !current_name.is_empty() {
                    let icon_data = [
                        current_icon.as_deref(),
                        current_binary.as_deref(),
                        Some(current_name.to_lowercase().as_str()),
                    ]
                    .iter()
                    .copied()
                    .flatten()
                    .find_map(|s| {
                        crate::shell_overlay_client::resolve_app_icon(s.to_string())
                    });
                    apps.push(AppVolume {
                        id,
                        name: current_name.clone(),
                        volume: current_vol,
                        icon_data,
                    });
                }
            }
            current_id = rest.parse().ok();
            current_name.clear();
            current_vol = 100;
            current_icon = None;
            current_binary = None;
        } else if let Some(val) = trimmed.strip_prefix("application.name = ") {
            current_name = val.trim_matches('"').to_string();
        } else if let Some(val) = trimmed.strip_prefix("application.icon_name = ") {
            current_icon = Some(val.trim_matches('"').to_string());
        } else if let Some(val) = trimmed.strip_prefix("application.process.binary = ") {
            current_binary = Some(val.trim_matches('"').to_string());
        } else if trimmed.starts_with("Volume:") {
            if let Some(pct_start) = trimmed.find("/ ") {
                let rest = &trimmed[pct_start + 2..];
                if let Some(pct_end) = rest.find('%') {
                    current_vol = rest[..pct_end].trim().parse().unwrap_or(100);
                }
            }
        }
    }
    // Last entry.
    if let Some(id) = current_id {
        if !current_name.is_empty() {
            let icon_data = [
                current_icon.as_deref(),
                current_binary.as_deref(),
                Some(current_name.to_lowercase().as_str()),
            ]
            .iter()
            .copied()
            .flatten()
            .find_map(|s| crate::shell_overlay_client::resolve_app_icon(s.to_string()));
            apps.push(AppVolume {
                id,
                name: current_name,
                volume: current_vol,
                icon_data,
            });
        }
    }

    Ok(apps)
}

/// Sets the volume for a specific application (sink-input).
#[tauri::command]
pub fn set_app_volume(id: u32, volume: u8) -> Result<(), String> {
    let status = std::process::Command::new("pactl")
        .args([
            "set-sink-input-volume",
            &id.to_string(),
            &format!("{volume}%"),
        ])
        .status()
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
pub fn toggle_audio_mute() -> Result<(), String> {
    let status = std::process::Command::new("wpctl")
        .args(["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"])
        .status()
        .map_err(|e| format!("wpctl set-mute failed: {e}"))?;

    if !status.success() {
        return Err("wpctl set-mute returned non-zero".into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Signal monitor via pactl subscribe
// ---------------------------------------------------------------------------

/// Start monitoring PulseAudio/PipeWire events for audio state changes.
///
/// Uses `pactl subscribe` which outputs a line on every sink/source change.
/// Emits `audio-changed` Tauri events.
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

        // Debounce: PulseAudio fires bursts of events for a single
        // user action (e.g. a volume change emits 3-5 events in <50ms).
        // Coalesce into one frontend event per 150ms window.
        let mut last_emit = std::time::Instant::now()
            - std::time::Duration::from_secs(1);

        for line in reader.lines() {
            let Ok(line) = line else { break };
            // pactl subscribe outputs lines like:
            // Event 'change' on sink #123
            // Event 'change' on source #456
            if line.contains("sink") || line.contains("source") || line.contains("server") {
                let now = std::time::Instant::now();
                if now.duration_since(last_emit) >= std::time::Duration::from_millis(150) {
                    let _ = app.emit("audio-changed", ());
                    last_emit = now;
                }
            }
        }

        log::warn!("audio: pactl subscribe ended, reconnecting in 2s");
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}
