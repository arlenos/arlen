/// System toggle commands: Caffeine (idle inhibit) and Screen Recording.

use std::process::{Child, Command};
use std::sync::Mutex;

use serde::Serialize;

/// Runtime state for system toggles (not persisted).
pub struct ToggleState {
    caffeine: Mutex<Option<Child>>,
    recording: Mutex<Option<Child>>,
    recording_path: Mutex<Option<String>>,
    /// UNIX-millis timestamp when the active recording started. The
    /// top-bar Recording badge consumes this to render a live elapsed
    /// counter.
    recording_started_at: Mutex<Option<u64>>,
}

impl ToggleState {
    /// Create with all toggles off.
    pub fn new() -> Self {
        Self {
            caffeine: Mutex::new(None),
            recording: Mutex::new(None),
            recording_path: Mutex::new(None),
            recording_started_at: Mutex::new(None),
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
    /// UNIX-millis timestamp when the recording started, or None.
    pub recording_started_at: Option<u64>,
}

/// Whether a helper is still running, clearing it if it has exited.
///
/// `is_some()` only says a child was once spawned. `systemd-inhibit` can be
/// refused by logind and a recorder can die on its own, and both leave a process
/// slot that reads as active forever - the panel would show a machine kept awake
/// by an inhibitor that is not there.
///
/// Reaping here rather than in a watchdog: the badges already poll this every
/// four seconds, so the poll they were doing anyway becomes the supervision.
fn still_running(slot: &mut Option<std::process::Child>) -> bool {
    let Some(child) = slot.as_mut() else { return false };
    match child.try_wait() {
        Ok(Some(status)) => {
            log::info!("toggle: helper exited on its own ({status})");
            *slot = None;
            false
        }
        // Still running, or a wait we cannot make sense of. Keeping it is the
        // conservative reading: the process may well be alive.
        _ => true,
    }
}

/// Get current toggle state.
#[tauri::command]
pub fn get_toggle_status(state: tauri::State<'_, ToggleState>) -> ToggleStatus {
    let caffeine = still_running(&mut state.caffeine.lock().unwrap());
    let mut rec_guard = state.recording.lock().unwrap();
    let recording = still_running(&mut rec_guard);
    let mut path_guard = state.recording_path.lock().unwrap();
    let mut started_guard = state.recording_started_at.lock().unwrap();
    if !recording {
        // The path and the clock belong to a recording that is over.
        path_guard.take();
        started_guard.take();
    }
    let path = path_guard.clone();
    let started = *started_guard;
    ToggleStatus {
        caffeine_active: caffeine,
        recording_active: recording,
        recording_path: path,
        recording_started_at: started,
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
                "--who=arlen-shell",
                "--why=Caffeine mode",
                "sleep",
                "infinity",
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("failed to start systemd-inhibit: {e}"))?;

        // Same reason as the recorder below: a spawn proves the binary exists,
        // not that logind granted the inhibit. Saying "the machine will stay
        // awake" when it will not is the one thing this toggle must not do.
        std::thread::sleep(std::time::Duration::from_millis(250));
        let mut child = child;
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!("the inhibitor stopped immediately ({status})"));
        }
        *guard = Some(child);
        log::info!("caffeine: activated");
        Ok(true)
    }
}

/// Toggle screen recording via wf-recorder.
///
/// Starts recording to `~/Videos/arlen-{timestamp}.mp4`.
/// Stops by sending SIGINT to the process.
#[tauri::command]
pub fn toggle_recording(state: tauri::State<'_, ToggleState>) -> Result<bool, String> {
    let mut rec_guard = state.recording.lock().unwrap();
    let mut path_guard = state.recording_path.lock().unwrap();
    let mut started_guard = state.recording_started_at.lock().unwrap();

    if let Some(ref mut child) = *rec_guard {
        // Stop: send SIGINT (graceful stop for wf-recorder).
        unsafe {
            libc::kill(child.id() as i32, libc::SIGINT);
        }
        let _ = child.wait();
        let path = path_guard.take();
        *rec_guard = None;
        *started_guard = None;
        log::info!("recording: stopped ({})", path.as_deref().unwrap_or("?"));
        Ok(false)
    } else {
        // Start: create output path and spawn wf-recorder.
        let videos_dir = dirs::video_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join("Videos")))
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
        let _ = std::fs::create_dir_all(&videos_dir);

        let timestamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
        let filename = format!("arlen-{timestamp}.mp4");
        let output = videos_dir.join(&filename);
        let output_str = output.to_string_lossy().to_string();

        let mut child = Command::new("wf-recorder")
            .args(["-f", &output_str])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("failed to start wf-recorder: {e}"))?;

        // A spawn only proves the binary exists. `wf-recorder` speaks
        // `zwlr_screencopy_v1`, which this compositor does not implement - it
        // serves `ext_image_copy_capture_v1` instead, which is also where the
        // screen-capture master switch is enforced. So the recorder exits almost
        // immediately, and without this the toggle would report success and the
        // panel would show a recording that never started.
        //
        // This catches the immediate failure so the click itself can report it.
        // A recorder that dies later is caught by `get_toggle_status`, which the
        // badges poll every four seconds and which reaps a helper that has gone.
        std::thread::sleep(std::time::Duration::from_millis(250));
        if let Ok(Some(status)) = child.try_wait() {
            let _ = std::fs::remove_file(&output);
            return Err(format!("the recorder stopped immediately ({status})"));
        }

        *rec_guard = Some(child);
        *path_guard = Some(output_str.clone());
        *started_guard = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        );
        log::info!("recording: started -> {output_str}");
        Ok(true)
    }
}
