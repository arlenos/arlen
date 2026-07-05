//! Theme-specific Tauri commands.
//!
//! These are thin convenience wrappers around the generic `config_*`
//! commands, kept separate so the frontend can call them without
//! building dot-notation keys itself.

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use super::config::{config_get, config_set, ConfigFile};

/// Return the current appearance.toml as a JSON object.
#[tauri::command]
pub fn theme_get() -> Result<serde_json::Value, String> {
    config_get(ConfigFile::Appearance, None)
}

/// One resolved colour role for the Appearance preview and per-field override
/// rows.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaletteRole {
    /// The semantic role key (`bg_app`, `accent`, `fg_primary`, ...).
    pub role: String,
    /// The resolved colour as `#RRGGBB[AA]`.
    pub hex: String,
}

/// The TOML content of the active theme's base: the bundled dark/light default,
/// or a user theme read from `~/.local/share/arlen/themes/{id}.toml`. `None` if
/// a named user theme file is missing.
fn active_theme_content(id: &str) -> Option<String> {
    match id {
        "dark" => Some(arlen_theme::DARK_TOML.to_string()),
        "light" => Some(arlen_theme::LIGHT_TOML.to_string()),
        _ => {
            let path = arlen_theme::ArlenTheme::user_themes_dir().join(format!("{id}.toml"));
            std::fs::read_to_string(path).ok()
        }
    }
}

/// Every resolved [`ColorTokens`] role as an ordered role/hex list (backgrounds,
/// foregrounds, accent and its states, status, borders) so the preview and
/// override rows render real values.
fn palette_of(theme: &arlen_theme::ArlenTheme) -> Vec<PaletteRole> {
    use arlen_theme::gtk::rgba_to_hex;
    let c = &theme.color;
    let pair = |role: &str, rgba| PaletteRole {
        role: role.to_string(),
        hex: rgba_to_hex(rgba),
    };
    vec![
        pair("bg_shell", c.bg_shell),
        pair("bg_app", c.bg_app),
        pair("bg_card", c.bg_card),
        pair("bg_overlay", c.bg_overlay),
        pair("bg_input", c.bg_input),
        pair("fg_primary", c.fg_primary),
        pair("fg_secondary", c.fg_secondary),
        pair("fg_disabled", c.fg_disabled),
        pair("fg_inverse", c.fg_inverse),
        pair("accent", c.accent),
        pair("accent_hover", c.accent_hover),
        pair("accent_pressed", c.accent_pressed),
        pair("success", c.success),
        pair("warning", c.warning),
        pair("error", c.error),
        pair("info", c.info),
        pair("border_default", c.border_default),
        pair("border_strong", c.border_strong),
    ]
}

/// The resolved colour palette of the active appearance: the active theme's base
/// merged with the `theme.toml` customization layer (the per-field overrides the
/// Appearance suite writes via `config_set(Customization, ...)`), resolved
/// through `sdk/theme`. Returns every semantic role's hex so the preview renders
/// the real theme instead of a fixture. NB the legacy `appearance.toml
/// [overrides]` (accent/radius/font) are a separate, superseded path and are not
/// folded in here; the suite writes overrides to `theme.toml`.
#[tauri::command]
pub fn theme_resolved_palette() -> Result<Vec<PaletteRole>, String> {
    let id = get_active_theme_id()?;
    let base =
        active_theme_content(&id).ok_or_else(|| format!("active theme '{id}' not found"))?;
    let customization =
        std::fs::read_to_string(arlen_theme::ArlenTheme::user_customization_path()).ok();
    let theme = arlen_theme::ArlenTheme::resolve(&base, None, customization.as_deref())
        .map_err(|e| format!("resolve: {e}"))?;
    Ok(palette_of(&theme))
}

/// The system's installed font families via `fc-list`, deduplicated and sorted,
/// for the Appearance font pickers (replacing the fixed short list). Each
/// `fc-list` line is one font file's family names; the primary (first
/// comma-separated) name is taken and a `BTreeSet` dedupes and sorts. Returns an
/// empty list if fontconfig is unavailable, so the picker degrades to whatever
/// the frontend defaults to rather than erroring.
#[tauri::command]
pub async fn theme_list_fonts() -> Vec<String> {
    let Ok(output) = tokio::process::Command::new("fc-list")
        .args([":", "family"])
        .output()
        .await
    else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let mut families: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for line in text.lines() {
        // Localised or aliased families are comma-separated on one line; the
        // first entry is the primary family name the picker wants.
        let primary = line.split(',').next().unwrap_or(line).trim();
        if !primary.is_empty() {
            families.insert(primary.to_string());
        }
    }
    families.into_iter().collect()
}

/// The XDG icon-theme search directories: `/usr/share/icons`, the user data
/// dir's `icons/`, and legacy `~/.icons`. A missing directory is simply skipped
/// by the readers below.
fn icon_search_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = vec![std::path::PathBuf::from("/usr/share/icons")];
    if let Some(data) = dirs::data_dir() {
        dirs.push(data.join("icons"));
    }
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".icons"));
    }
    dirs
}

/// The installed icon themes for the Appearance icon picker: directories under
/// the XDG icon paths that carry an `index.theme` and at least one icon
/// directory (any subdirectory other than `cursors`), which excludes
/// pure-cursor themes that also ship an `index.theme`. Deduplicated and sorted;
/// empty if none are found.
#[tauri::command]
pub fn theme_list_icon_themes() -> Vec<String> {
    let mut themes = std::collections::BTreeSet::new();
    for base in icon_search_dirs() {
        let Ok(entries) = std::fs::read_dir(&base) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.join("index.theme").is_file() {
                continue;
            }
            let has_icon_dir = std::fs::read_dir(&path)
                .into_iter()
                .flatten()
                .flatten()
                .any(|e| e.path().is_dir() && e.file_name().to_string_lossy() != "cursors");
            if has_icon_dir {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    themes.insert(name.to_string());
                }
            }
        }
    }
    themes.into_iter().collect()
}

/// The installed cursor themes for the Appearance cursor picker: directories
/// under the XDG icon paths that contain a `cursors/` subdirectory (the
/// definitive cursor-theme marker). Deduplicated and sorted.
#[tauri::command]
pub fn theme_list_cursor_themes() -> Vec<String> {
    let mut themes = std::collections::BTreeSet::new();
    for base in icon_search_dirs() {
        let Ok(entries) = std::fs::read_dir(&base) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.join("cursors").is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    themes.insert(name.to_string());
                }
            }
        }
    }
    themes.into_iter().collect()
}

/// Set the accent color (hex string like `#3b82f6`).
#[tauri::command]
pub async fn theme_set_accent(color: String) -> Result<(), String> {
    config_set(
        ConfigFile::Appearance,
        "overrides.accent".into(),
        serde_json::Value::String(color),
    )
    .await
}

/// A theme as the gallery lists it: identity + a resolved preview swatch.
/// Mirrors the desktop-shell `ThemeInfo` and adds `swatch` so the gallery
/// renders real colours instead of a fixture.
#[derive(Debug, Clone, Serialize)]
pub struct ThemeSummary {
    /// Theme id (the `[meta].id`, also the `theme.active` value).
    pub id: String,
    /// Display name.
    pub name: String,
    /// `"dark"` or `"light"`.
    pub variant: String,
    /// True for the built-in dark/light themes, false for user-installed.
    pub is_builtin: bool,
    /// Five representative resolved colours, hex: background, surface,
    /// accent, a structural mid-tone, foreground. The gallery paints these
    /// as the preview strip.
    pub swatch: Vec<String>,
}

/// The five preview colours for a resolved theme, in the gallery's order
/// (bg / surface / accent / secondary-structural / fg).
fn swatch_of(theme: &arlen_theme::ArlenTheme) -> Vec<String> {
    use arlen_theme::gtk::rgba_to_hex;
    let c = &theme.color;
    vec![
        rgba_to_hex(c.bg_app),
        rgba_to_hex(c.bg_card),
        rgba_to_hex(c.accent),
        rgba_to_hex(c.border_strong),
        rgba_to_hex(c.fg_primary),
    ]
}

/// Resolve a theme file's TOML into a gallery summary. Returns `None` if the
/// content does not resolve (a malformed user theme is skipped, not fatal).
fn summary_of(content: &str, is_builtin: bool) -> Option<ThemeSummary> {
    let theme = arlen_theme::ArlenTheme::from_bundled(content).ok()?;
    Some(ThemeSummary {
        id: theme.meta.id.clone(),
        name: theme.meta.name.clone(),
        variant: if theme.is_dark() { "dark" } else { "light" }.to_string(),
        is_builtin,
        swatch: swatch_of(&theme),
    })
}

/// List every available theme (built-in dark/light + user-installed under
/// `~/.local/share/arlen/themes/`), each resolved through `sdk/theme` so the
/// gallery previews are real. A user theme that fails to resolve is skipped.
#[tauri::command]
pub fn get_available_themes() -> Vec<ThemeSummary> {
    let mut out = Vec::new();
    for content in [arlen_theme::DARK_TOML, arlen_theme::LIGHT_TOML] {
        if let Some(summary) = summary_of(content, true) {
            out.push(summary);
        }
    }
    if let Ok(entries) = std::fs::read_dir(arlen_theme::ArlenTheme::user_themes_dir()) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Some(summary) = summary_of(&content, false) {
                        out.push(summary);
                    }
                }
            }
        }
    }
    out
}

/// Switch the active theme: persist `appearance.toml [theme].active = id` and
/// emit `config:appearance:changed` so listeners re-resolve immediately. The
/// file watcher fires on the write too, but emitting directly makes the switch
/// feel instant instead of waiting on the debounce.
#[tauri::command]
pub async fn set_theme(id: String, app: AppHandle) -> Result<(), String> {
    config_set(
        ConfigFile::Appearance,
        "theme.active".into(),
        serde_json::Value::String(id),
    )
    .await?;
    if let Err(e) = app.emit("config:appearance:changed", ()) {
        log::warn!("set_theme: emit config:appearance:changed failed: {e}");
    }
    Ok(())
}

/// The currently active theme id (`appearance.toml [theme].active`), defaulting
/// to `"dark"` when the key is unset so the gallery always has a selection.
#[tauri::command]
pub fn get_active_theme_id() -> Result<String, String> {
    let value = config_get(ConfigFile::Appearance, Some("theme.active".into()))?;
    Ok(value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| "dark".to_string()))
}

/// Copy validated theme TOML into the user theme store under the resolved id.
/// Resolving through `sdk/theme` is the validation: it applies the required-field
/// check AND the TH-0 inert-data floor (a value that could carry config syntax is
/// neutralised), and the resolver already slugifies `meta.id` to a path-safe form,
/// so a malicious file can neither smuggle syntax nor escape the themes dir. A
/// file that does not resolve is refused, not installed.
fn install_theme_content(content: &str) -> Result<ThemeSummary, String> {
    let theme = arlen_theme::ArlenTheme::from_bundled(content)
        .map_err(|e| format!("not a valid theme: {e}"))?;
    let id = theme.meta.id.clone();
    // Belt-and-suspenders over the resolver's slug: never write outside the dir.
    if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
        return Err(format!("theme id is not a safe filename: {id}"));
    }
    let dir = arlen_theme::ArlenTheme::user_themes_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("create themes dir: {e}"))?;
    std::fs::write(dir.join(format!("{id}.toml")), content)
        .map_err(|e| format!("install theme: {e}"))?;
    summary_of(content, false).ok_or_else(|| "theme resolved but summary failed".to_string())
}

/// Install a theme from a user-picked `.toml` file: pick, validate, copy into
/// `~/.local/share/arlen/themes/{id}.toml`. Returns the installed theme's summary
/// so the gallery can add it without a full reload. Errors (no file / invalid
/// theme) distinguish a cancel from a bad file by the message.
#[tauri::command]
pub async fn theme_install_file() -> Result<ThemeSummary, String> {
    let src = crate::commands::picker::pick_theme_file()
        .await
        .ok_or_else(|| "no file selected".to_string())?;
    let content = std::fs::read_to_string(&src).map_err(|e| format!("read theme: {e}"))?;
    install_theme_content(&content)
}

/// Map a catppuccin flavor name to its enum (case-insensitive).
fn parse_flavor(name: &str) -> Option<arlen_theme::catppuccin::Flavor> {
    use arlen_theme::catppuccin::Flavor;
    match name.to_ascii_lowercase().as_str() {
        "latte" => Some(Flavor::Latte),
        "frappe" => Some(Flavor::Frappe),
        "macchiato" => Some(Flavor::Macchiato),
        "mocha" => Some(Flavor::Mocha),
        _ => None,
    }
}

/// Map a catppuccin accent name to its enum (case-insensitive).
fn parse_accent(name: &str) -> Option<arlen_theme::catppuccin::Accent> {
    use arlen_theme::catppuccin::Accent;
    Some(match name.to_ascii_lowercase().as_str() {
        "rosewater" => Accent::Rosewater,
        "flamingo" => Accent::Flamingo,
        "pink" => Accent::Pink,
        "mauve" => Accent::Mauve,
        "red" => Accent::Red,
        "maroon" => Accent::Maroon,
        "peach" => Accent::Peach,
        "yellow" => Accent::Yellow,
        "green" => Accent::Green,
        "teal" => Accent::Teal,
        "sky" => Accent::Sky,
        "sapphire" => Accent::Sapphire,
        "blue" => Accent::Blue,
        "lavender" => Accent::Lavender,
        _ => return None,
    })
}

/// Import a colour scheme into a full Arlen theme and install it. `catppuccin`
/// adapts the named flavor + accent (defaults mocha/mauve); `base16` picks a
/// scheme file (YAML/JSON/TOML), parses it, and adapts it. Both run through the
/// sdk/theme inbound adapters (Rule A/B contrast clamp) and then the same
/// validated install path. Returns the installed theme's summary.
#[tauri::command]
pub async fn theme_import_scheme(
    kind: String,
    flavor: Option<String>,
    accent: Option<String>,
) -> Result<ThemeSummary, String> {
    let theme_toml = match kind.as_str() {
        "catppuccin" => {
            let flavor = parse_flavor(flavor.as_deref().unwrap_or("mocha"))
                .ok_or_else(|| "unknown catppuccin flavor".to_string())?;
            let accent = parse_accent(accent.as_deref().unwrap_or("mauve"))
                .ok_or_else(|| "unknown catppuccin accent".to_string())?;
            arlen_theme::catppuccin::adapt_catppuccin(flavor, accent)
        }
        "base16" => {
            let src = crate::commands::picker::pick_scheme_file()
                .await
                .ok_or_else(|| "no scheme selected".to_string())?;
            let text = std::fs::read_to_string(&src).map_err(|e| format!("read scheme: {e}"))?;
            let scheme = arlen_theme::base16::parse_scheme(&text)
                .map_err(|e| format!("not a base16 scheme: {e}"))?;
            arlen_theme::base16::adapt_base16(&scheme)
        }
        other => return Err(format!("unknown scheme kind: {other}")),
    };
    install_theme_content(&theme_toml)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_has_all_roles_with_valid_hex() {
        let theme = arlen_theme::ArlenTheme::from_bundled(arlen_theme::DARK_TOML).unwrap();
        let palette = palette_of(&theme);
        assert_eq!(palette.len(), 18, "every semantic role is present");
        for role in &palette {
            assert!(
                role.hex.starts_with('#') && role.hex.len() >= 7,
                "role {} has an invalid hex {}",
                role.role,
                role.hex
            );
        }
        // A resolved dark theme has an accent distinct from the app background.
        let accent = palette.iter().find(|r| r.role == "accent").unwrap();
        let bg = palette.iter().find(|r| r.role == "bg_app").unwrap();
        assert_ne!(accent.hex, bg.hex);
    }
}
