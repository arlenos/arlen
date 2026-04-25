/// Tauri commands for layout applet communication.
///
/// Reads layout state and writes changes to the compositor TOML config.
/// The current mode is tracked in a shared AtomicU8 that gets updated
/// by the shell_overlay_client when `layout_mode_changed` events arrive.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};

/// Shared layout mode state. Updated by shell_overlay_client events.
///
/// 0 = floating, 1 = tiling, 2 = monocle.
pub static CURRENT_MODE: AtomicU8 = AtomicU8::new(0);

/// Current layout state.
#[derive(Clone, Serialize)]
pub struct LayoutState {
    pub mode: String,
    pub inner_gap: i32,
    pub outer_gap: i32,
    pub smart_gaps: bool,
    pub tiled_headers: bool,
}

/// Get the layout config path.
fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".config/lunaris/compositor.toml")
}

/// Cached layout section from compositor.toml. Invalidated by TTL (5s)
/// to avoid reading disk on every popover open and slider drag tick.
static LAYOUT_CACHE: std::sync::Mutex<Option<(std::time::Instant, toml::Table)>> =
    std::sync::Mutex::new(None);
const LAYOUT_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(5);

fn read_layout_table() -> toml::Table {
    // Return cache if fresh.
    {
        let guard = LAYOUT_CACHE.lock().unwrap();
        if let Some((ts, ref tbl)) = *guard {
            if ts.elapsed() < LAYOUT_CACHE_TTL {
                return tbl.clone();
            }
        }
    }
    // Cache miss — read from disk.
    let path = config_path();
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let table: toml::Table = toml::from_str(&content).unwrap_or_default();
    *LAYOUT_CACHE.lock().unwrap() = Some((std::time::Instant::now(), table.clone()));
    table
}

/// Invalidate the layout cache after a write so the next read picks
/// up the new values immediately.
fn invalidate_layout_cache() {
    *LAYOUT_CACHE.lock().unwrap() = None;
}

/// Read the current layout state from compositor.toml (cached for 5s).
#[tauri::command]
pub fn get_layout_state() -> LayoutState {
    let table = read_layout_table();

    let layout = table.get("layout").and_then(|v| v.as_table());
    let mode = match CURRENT_MODE.load(Ordering::Relaxed) {
        1 => "tiling",
        2 => "monocle",
        _ => "floating",
    };
    LayoutState {
        mode: mode.into(),
        inner_gap: layout
            .and_then(|l| l.get("inner_gap"))
            .and_then(|v| v.as_integer())
            .unwrap_or(8) as i32,
        outer_gap: layout
            .and_then(|l| l.get("outer_gap"))
            .and_then(|v| v.as_integer())
            .unwrap_or(8) as i32,
        smart_gaps: layout
            .and_then(|l| l.get("smart_gaps"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        tiled_headers: layout
            .and_then(|l| l.get("tiled_headers"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    }
}

/// Update the [layout] section in compositor.toml.
///
/// Reads the existing file, updates only layout fields, writes back.
fn update_layout_field(key: &str, value: toml::Value) {
    let path = config_path();
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let mut table: toml::Table = toml::from_str(&content).unwrap_or_default();

    let layout = table
        .entry("layout".to_string())
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    if let toml::Value::Table(ref mut t) = layout {
        t.insert(key.to_string(), value);
    }

    if let Ok(out) = toml::to_string_pretty(&table) {
        let _ = std::fs::write(&path, out);
    }
    invalidate_layout_cache();
}

/// Minimum and maximum accepted gap values (pixels). Matches the slider
/// range in LayoutPopover; this server-side clamp catches malformed or
/// forged invoke payloads so the compositor never sees a nonsense
/// value that would render windows at screen-edge or absurdly spaced.
const GAP_MIN: i32 = 0;
const GAP_MAX: i32 = 100;

fn clamp_gap(name: &str, v: i32) -> i32 {
    let clamped = v.clamp(GAP_MIN, GAP_MAX);
    if clamped != v {
        log::warn!(
            "layout: {name}={v} out of range [{GAP_MIN}, {GAP_MAX}]; clamped to {clamped}"
        );
    }
    clamped
}

/// Set the gaps. Values outside `[GAP_MIN, GAP_MAX]` are clamped with a
/// warning; the command never rejects so the UI doesn't have to handle
/// an error case for a hardware-impossible input.
#[tauri::command]
pub fn set_layout_gaps(inner: i32, outer: i32) {
    let inner = clamp_gap("inner_gap", inner);
    let outer = clamp_gap("outer_gap", outer);
    update_layout_field("inner_gap", toml::Value::Integer(inner as i64));
    update_layout_field("outer_gap", toml::Value::Integer(outer as i64));
    log::info!("layout: gaps set to inner={inner} outer={outer}");
}

/// Set the layout mode for the active workspace.
///
/// Sends a `set_layout_mode` request to the compositor via the
/// shell-overlay protocol.
#[tauri::command]
pub fn set_layout_mode(
    state: tauri::State<std::sync::Arc<crate::shell_overlay_client::ShellOverlaySender>>,
    mode: String,
) {
    let mode_u32 = match mode.as_str() {
        "floating" => 0,
        "tiling" => 1,
        "monocle" => 2,
        _ => {
            log::warn!("set_layout_mode: unknown mode {mode}");
            return;
        }
    };
    state.set_layout_mode(mode_u32);
    log::info!("layout: mode set to {mode}");
}

/// Set smart gaps.
#[tauri::command]
pub fn set_layout_smart_gaps(enabled: bool) {
    update_layout_field("smart_gaps", toml::Value::Boolean(enabled));
    log::info!("layout: smart_gaps={enabled}");
}

/// Toggle compositor-rendered header rendering on tiled SSD windows.
///
/// When `false` (default), tiled windows lose their compositor-rendered
/// 36px header to match tiling-WM convention (i3, sway, hyprland).
/// Stacks always keep their tab-bar header (functional UI), and floating
/// windows are unaffected.
#[tauri::command]
pub fn set_layout_tiled_headers(enabled: bool) {
    update_layout_field("tiled_headers", toml::Value::Boolean(enabled));
    log::info!("layout: tiled_headers={enabled}");
}
