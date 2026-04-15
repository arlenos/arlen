/// System toggle commands: Caffeine (idle inhibit) and Screen Recording.

use std::process::{Child, Command};
use std::sync::Mutex;

use serde::Serialize;

/// Runtime state for system toggles (not persisted).
pub struct ToggleState {
    caffeine: Mutex<Option<Child>>,
    recording: Mutex<Option<Child>>,
    recording_path: Mutex<Option<String>>,
}

impl ToggleState {
    /// Create with all toggles off.
    pub fn new() -> Self {
        Self {
            caffeine: Mutex::new(None),
            recording: Mutex::new(None),
            recording_path: Mutex::new(None),
        }
    }
}

/// Current state of all system toggles.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToggleStatus {
    pub caffeine_active: bool,
    pub recording_active: bool,
    pub recording_path: Option<String>,
}

/// Get current toggle state.
#[tauri::command]
pub fn get_toggle_status(state: tauri::State<'_, ToggleState>) -> ToggleStatus {
    let caffeine = state.caffeine.lock().unwrap().is_some();
    let recording = state.recording.lock().unwrap().is_some();
    let path = state.recording_path.lock().unwrap().clone();
    ToggleStatus {
        caffeine_active: caffeine,
        recording_active: recording,
        recording_path: path,
    }
}

/// Toggle idle/sleep inhibit (Caffeine mode).
///
/// Uses `systemd-inhibit` to prevent the system from going idle or
/// sleeping. Killing the child process releases the inhibit.
#[tauri::command]
pub fn toggle_caffeine(state: tauri::State<'_, ToggleState>) -> Result<bool, String> {
    let mut guard = state.caffeine.lock().unwrap();
    if let Some(ref mut child) = *guard {
        // Deactivate: kill the inhibitor process.
        let _ = child.kill();
        let _ = child.wait();
        *guard = None;
        log::info!("caffeine: deactivated");
        Ok(false)
    } else {
        // Activate: spawn systemd-inhibit.
        let child = Command::new("systemd-inhibit")
            .args([
                "--what=idle:sleep",
                "--who=lunaris-shell",
                "--why=Caffeine mode",
                "sleep",
                "infinity",
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("failed to start systemd-inhibit: {e}"))?;
        *guard = Some(child);
        log::info!("caffeine: activated");
        Ok(true)
    }
}

/// Toggle screen recording via wf-recorder.
///
/// Starts recording to `~/Videos/lunaris-{timestamp}.mp4`.
/// Stops by sending SIGINT to the process.
#[tauri::command]
pub fn toggle_recording(state: tauri::State<'_, ToggleState>) -> Result<bool, String> {
    let mut rec_guard = state.recording.lock().unwrap();
    let mut path_guard = state.recording_path.lock().unwrap();

    if let Some(ref mut child) = *rec_guard {
        // Stop: send SIGINT (graceful stop for wf-recorder).
        unsafe {
            libc::kill(child.id() as i32, libc::SIGINT);
        }
        let _ = child.wait();
        let path = path_guard.take();
        *rec_guard = None;
        log::info!("recording: stopped ({})", path.as_deref().unwrap_or("?"));
        Ok(false)
    } else {
        // Start: create output path and spawn wf-recorder.
        let videos_dir = dirs::video_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join("Videos")))
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
        let _ = std::fs::create_dir_all(&videos_dir);

        let timestamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
        let filename = format!("lunaris-{timestamp}.mp4");
        let output = videos_dir.join(&filename);
        let output_str = output.to_string_lossy().to_string();

        let child = Command::new("wf-recorder")
            .args(["-f", &output_str])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("failed to start wf-recorder: {e}"))?;

        *rec_guard = Some(child);
        *path_guard = Some(output_str.clone());
        log::info!("recording: started -> {output_str}");
        Ok(true)
    }
}
