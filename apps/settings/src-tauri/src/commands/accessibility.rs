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

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const STATE_DIR: &str = "cosmic-comp";
const STATE_FILE: &str = "a11y_screen_filter.ron";

/// Mirrors `compositor::config::ColorFilter` — the discriminant
/// values matter (`offscreen.frag` reads them) but for write-back
/// we only need to round-trip the variant names through RON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorFilter {
    Greyscale,
    Protanopia,
    Deuteranopia,
    Tritanopia,
}

impl ColorFilter {
    fn from_label(s: &str) -> Option<Self> {
        match s {
            "Greyscale" => Some(Self::Greyscale),
            "Protanopia" => Some(Self::Protanopia),
            "Deuteranopia" => Some(Self::Deuteranopia),
            "Tritanopia" => Some(Self::Tritanopia),
            _ => None,
        }
    }
}

/// On-disk schema. Mirrors the compositor's `ScreenFilter` minus
/// the `night_light_tint` field, which is `#[serde(skip)]` there
/// (computed live, never persisted) — we omit it on write so the
/// compositor's parser sees the same shape it always has.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenFilter {
    pub inverted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_filter: Option<ColorFilter>,
}

/// Frontend-friendly view: `null` for "no filter" instead of the
/// Rust `Option`. Frontend sends `null` from the PopoverSelect
/// when the user picks "None".
///
/// `rename_all = "camelCase"` keeps the JSON contract stable in
/// the convention Svelte stores expect — without it, `color_filter`
/// silently dropped to `None` on every save and reads came back
/// as a missing field (Codex Sprint C review HIGH 1).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenFilterDto {
    pub inverted: bool,
    /// `null` ⇒ no colour filter. Otherwise one of the
    /// `ColorFilter` variant names ("Greyscale", "Protanopia",
    /// "Deuteranopia", "Tritanopia"). Unknown labels are rejected
    /// at parse time.
    #[serde(default)]
    pub color_filter: Option<String>,
}

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

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip through serde with the camelCase rename ensures
    /// the JSON contract matches the frontend expectation. Without
    /// `rename_all = "camelCase"`, color_filter would silently drop
    /// across the Tauri boundary (Codex Sprint C review HIGH 1).
    #[test]
    fn dto_serialises_as_camel_case() {
        let dto = ScreenFilterDto {
            inverted: true,
            color_filter: Some("Protanopia".into()),
        };
        let json = serde_json::to_string(&dto).unwrap();
        assert!(
            json.contains("colorFilter"),
            "DTO must serialise as camelCase, got: {json}"
        );
        assert!(
            !json.contains("color_filter"),
            "snake_case key leaked through: {json}"
        );

        // And it must deserialise from the same shape.
        let back: ScreenFilterDto = serde_json::from_str(&json).unwrap();
        assert!(back.inverted);
        assert_eq!(back.color_filter, Some("Protanopia".to_string()));
    }

    /// The full set->get round-trip: sending a non-null filter
    /// from the frontend (camelCase) must come back the same way.
    #[test]
    fn dto_round_trips_non_null_filter() {
        let payload = serde_json::json!({
            "inverted": false,
            "colorFilter": "Deuteranopia"
        });
        let dto: ScreenFilterDto = serde_json::from_value(payload).unwrap();
        assert_eq!(dto.color_filter, Some("Deuteranopia".to_string()));

        let json = serde_json::to_value(&dto).unwrap();
        assert_eq!(json["colorFilter"], "Deuteranopia");
    }

    /// Frontend "None"/"none"/empty/missing all map to `Option::None`
    /// on disk so the compositor reads "no filter applied".
    #[test]
    fn from_label_handles_none_sentinels() {
        assert_eq!(ColorFilter::from_label("Greyscale"), Some(ColorFilter::Greyscale));
        assert_eq!(ColorFilter::from_label("Protanopia"), Some(ColorFilter::Protanopia));
        assert_eq!(ColorFilter::from_label("Deuteranopia"), Some(ColorFilter::Deuteranopia));
        assert_eq!(ColorFilter::from_label("Tritanopia"), Some(ColorFilter::Tritanopia));

        // Anything else returns None — the set command then maps
        // that to "no filter" for the recognised None-sentinels.
        assert_eq!(ColorFilter::from_label("None"), None);
        assert_eq!(ColorFilter::from_label(""), None);
        assert_eq!(ColorFilter::from_label("garbage"), None);
    }

    /// Round-trip through the on-disk RON schema matches the
    /// compositor's `ScreenFilter` shape (inverted + color_filter
    /// only — no night_light_tint).
    #[test]
    fn ron_roundtrip_matches_compositor_shape() {
        let state = ScreenFilter {
            inverted: true,
            color_filter: Some(ColorFilter::Greyscale),
        };
        let s = ron::ser::to_string(&state).unwrap();
        // Sanity: variant name should be in serialised output —
        // that's what the compositor parser keys on.
        assert!(s.contains("Greyscale"), "missing variant name: {s}");
        assert!(s.contains("inverted"));

        // Empty-filter shape: color_filter omitted thanks to
        // skip_serializing_if. The compositor's serde-default
        // for missing fields then resolves to None on read.
        let empty = ScreenFilter {
            inverted: false,
            color_filter: None,
        };
        let s2 = ron::ser::to_string(&empty).unwrap();
        assert!(!s2.contains("color_filter"), "should omit None: {s2}");
    }

    #[test]
    fn dto_default_color_filter_is_none() {
        let json = r#"{"inverted":false}"#;
        let dto: ScreenFilterDto = serde_json::from_str(json).unwrap();
        assert!(!dto.inverted);
        assert!(dto.color_filter.is_none());
    }
}
