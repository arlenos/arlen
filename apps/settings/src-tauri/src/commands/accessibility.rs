//! Screen-filter (invert + colour-blindness modes) commands.
//!
//! The compositor stores the filter state in
//! `~/.local/state/cosmic-comp/a11y_screen_filter.ron` and watches
//! the file for live updates. We round-trip through the same RON
//! schema so a Settings save flips the screen within ~100 ms via
//! the existing notify-watcher path — no Tauri/D-Bus IPC needed
//! to talk to the compositor.
//!
//! `compositor.toml [accessibility_zoom]` (the magnifier settings)
//! goes through the regular `config_set` Tauri command — no
//! special handling here.
//!
//! The filter types + the variant<->label mapping (round-trip-tested in CI) live
//! in `arlen-settings-core::accessibility`; this file is the file read/write.

use std::path::PathBuf;

use arlen_settings_core::accessibility::{ColorFilter, ScreenFilter, ScreenFilterDto};

const STATE_DIR: &str = "cosmic-comp";
const STATE_FILE: &str = "a11y_screen_filter.ron";

fn state_path() -> Option<PathBuf> {
    dirs::state_dir().map(|p| p.join(STATE_DIR).join(STATE_FILE))
}

/// Read the current filter state, or defaults if the file is missing.
#[tauri::command]
pub fn accessibility_filter_get() -> Result<ScreenFilterDto, String> {
    let Some(path) = state_path() else {
        return Ok(ScreenFilterDto {
            inverted: false,
            color_filter: None,
        });
    };
    if !path.exists() {
        return Ok(ScreenFilterDto {
            inverted: false,
            color_filter: None,
        });
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    let parsed: ScreenFilter = ron::de::from_str(&content)
        .map_err(|e| format!("parse {}: {e}", path.display()))?;
    Ok(ScreenFilterDto {
        inverted: parsed.inverted,
        color_filter: parsed.color_filter.map(|cf| match cf {
            ColorFilter::Greyscale => "Greyscale".to_string(),
            ColorFilter::Protanopia => "Protanopia".to_string(),
            ColorFilter::Deuteranopia => "Deuteranopia".to_string(),
            ColorFilter::Tritanopia => "Tritanopia".to_string(),
        }),
    })
}

/// Write a new filter state. Atomic tmp+rename so the compositor's
/// notify-watcher never sees a half-written file.
#[tauri::command]
pub fn accessibility_filter_set(dto: ScreenFilterDto) -> Result<(), String> {
    let path = state_path()
        .ok_or_else(|| "could not resolve XDG state dir".to_string())?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }

    let color_filter = match dto.color_filter.as_deref() {
        None | Some("") | Some("None") | Some("none") => None,
        Some(label) => Some(
            ColorFilter::from_label(label)
                .ok_or_else(|| format!("unknown colour filter: {label}"))?,
        ),
    };

    let state = ScreenFilter {
        inverted: dto.inverted,
        color_filter,
    };

    let serialised = ron::ser::to_string_pretty(&state, Default::default())
        .map_err(|e| format!("serialise: {e}"))?;

    let tmp = path.with_extension("ron.tmp");
    std::fs::write(&tmp, serialised.as_bytes())
        .map_err(|e| format!("write tmp {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path)
        .map_err(|e| format!("rename {} -> {}: {e}", tmp.display(), path.display()))?;

    Ok(())
}
