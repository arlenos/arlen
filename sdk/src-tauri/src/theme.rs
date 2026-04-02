/// Theme loader for Lunaris.
///
/// Uses the `lunaris-theme` SDK crate to load and watch `~/.config/lunaris/theme.toml`.
/// Converts the resolved `LunarisTheme` into `SurfaceTokens` for the Tauri frontend.

use lunaris_theme::LunarisTheme;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

/// Surface tokens serialized to the TypeScript frontend.
/// Colors are hex strings so CSS can consume them directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceTokens {
    pub bg_shell: String,
    pub bg_app: String,
    pub bg_card: String,
    pub bg_overlay: String,
    pub bg_input: String,
    pub fg_shell: String,
    pub fg_app: String,
    pub fg_secondary: String,
    pub fg_disabled: String,
    pub accent: String,
    pub border: String,
    pub radius: String,
}

/// Convert an RGBA [f32; 4] to a CSS hex string.
fn rgba_to_hex(c: [f32; 4]) -> String {
    let r = (c[0] * 255.0).round() as u8;
    let g = (c[1] * 255.0).round() as u8;
    let b = (c[2] * 255.0).round() as u8;
    let a = (c[3] * 255.0).round() as u8;
    if a == 255 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}

impl SurfaceTokens {
    /// Derive surface tokens from a resolved LunarisTheme.
    pub fn from_theme(theme: &LunarisTheme) -> Self {
        Self {
            bg_shell:     rgba_to_hex(theme.bg_shell),
            bg_app:       rgba_to_hex(theme.bg_app),
            bg_card:      rgba_to_hex(theme.bg_card),
            bg_overlay:   rgba_to_hex(theme.bg_overlay),
            bg_input:     rgba_to_hex(theme.bg_input),
            // Shell foreground: derive from bg_shell brightness.
            // Panda shell is dark, so shell fg is light.
            fg_shell:     if theme.is_dark { rgba_to_hex(theme.fg_primary) } else { "#fafafa".into() },
            fg_app:       rgba_to_hex(theme.fg_primary),
            fg_secondary: rgba_to_hex(theme.fg_secondary),
            fg_disabled:  rgba_to_hex(theme.fg_disabled),
            accent:       rgba_to_hex(theme.accent),
            border:       rgba_to_hex(theme.border),
            radius:       format!("{}rem", theme.radius_s[0] / 16.0),
        }
    }

    /// Panda defaults for backward compatibility.
    pub fn panda() -> Self {
        Self::from_theme(&LunarisTheme::panda())
    }
}

/// Load surface tokens from `~/.config/lunaris/theme.toml`.
pub fn load_tokens() -> SurfaceTokens {
    SurfaceTokens::from_theme(&LunarisTheme::load())
}

/// Tauri command: return current surface tokens.
#[tauri::command]
pub fn get_surface_tokens() -> SurfaceTokens {
    load_tokens()
}

/// Start watching theme.toml for changes and emit Tauri events.
pub fn start_watcher(app: AppHandle) {
    let app_clone = app.clone();
    // ThemeWatcher runs on a background thread internally.
    let watcher = lunaris_theme::ThemeWatcher::start(move |theme| {
        let tokens = SurfaceTokens::from_theme(&theme);
        if let Err(e) = app_clone.emit("lunaris://theme-changed", &tokens) {
            eprintln!("lunaris: failed to emit theme-changed: {e}");
        }
    });

    match watcher {
        Ok(w) => {
            // Keep the watcher alive for the process lifetime.
            std::mem::forget(w);
        }
        Err(e) => {
            eprintln!("lunaris: failed to start theme watcher: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panda_tokens_are_valid_hex() {
        let t = SurfaceTokens::panda();
        for color in [
            &t.bg_shell, &t.bg_app, &t.bg_card, &t.fg_shell, &t.fg_app,
            &t.fg_secondary, &t.fg_disabled, &t.accent, &t.border,
        ] {
            assert!(color.starts_with('#'), "expected hex: {color}");
        }
    }

    #[test]
    fn rgba_to_hex_opaque() {
        assert_eq!(rgba_to_hex([1.0, 0.0, 0.0, 1.0]), "#ff0000");
    }

    #[test]
    fn rgba_to_hex_with_alpha() {
        assert_eq!(rgba_to_hex([0.0, 0.0, 0.0, 0.502]), "#00000080");
    }

    #[test]
    fn from_theme_roundtrips_panda_colors() {
        let t = SurfaceTokens::from_theme(&LunarisTheme::panda());
        assert_eq!(t.bg_shell, "#09090b");
        assert_eq!(t.bg_app, "#ffffff");
        assert_eq!(t.accent, "#09090b");
    }
}
