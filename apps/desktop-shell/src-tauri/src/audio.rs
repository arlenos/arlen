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
}

/// Returns the current volume and mute state of the default audio sink.
#[tauri::command]
pub fn get_audio_status() -> Result<AudioStatus, String> {
    let output = std::process::Command::new("wpctl")
        .args(["get-volume", "@DEFAULT_AUDIO_SINK@"])
        .output()
        .map_err(|e| format!("wpctl not found: {e}"))?;

    if !output.status.success() {
        return Err("wpctl get-volume failed".into());
    }

    // Output format: "Volume: 0.75" or "Volume: 0.75 [MUTED]"
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

    Ok(AudioStatus { volume, muted })
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
