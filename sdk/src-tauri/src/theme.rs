/// Theme loader for Lunaris.
///
/// Reads `~/.config/lunaris/theme.toml` and returns the surface tokens
/// as a structured response. The frontend sets CSS custom properties
/// from these values at startup and whenever the file changes.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Surface tokens passed to the WebView.
///
/// Mirrors `os_sdk::shell_types::SurfaceTokens` but kept local to avoid
/// a Rust workspace dependency between ui-kit and sdk for now.
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
    pub accent: String,
    pub border: String,
    pub radius: String,
}

impl SurfaceTokens {
    /// Built-in Panda theme tokens.
    /// Used as fallback when no theme.toml exists.
    pub fn panda() -> Self {
        Self {
            bg_shell:   "#1a1a2e".into(),
            bg_app:     "#ffffff".into(),
            bg_card:    "#f5f5f7".into(),
            bg_overlay: "#00000080".into(),
            bg_input:   "#f0f0f0".into(),
            fg_shell:   "#e8e8f0".into(),
            fg_app:     "#1a1a2e".into(),
            accent:     "#7c6af7".into(),
            border:     "#e2e2e8".into(),
            radius:     "0.5rem".into(),
        }
    }
}

/// Raw TOML structure for theme.toml.
#[derive(Debug, Deserialize)]
struct ThemeFile {
    color: Option<ColorSection>,
    radius: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ColorSection {
    bg: Option<BgSection>,
    fg: Option<FgSection>,
    accent: Option<String>,
    border: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BgSection {
    shell: Option<String>,
    app: Option<String>,
    card: Option<String>,
    overlay: Option<String>,
    input: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FgSection {
    shell: Option<String>,
    app: Option<String>,
}

fn theme_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/etc"))
        .join("lunaris")
        .join("theme.toml")
}

/// Load surface tokens from `~/.config/lunaris/theme.toml`.
///
/// Falls back to the built-in Panda theme if the file does not exist
/// or cannot be parsed. Never returns an error to the frontend;
/// the system always has a valid theme.
pub fn load_tokens() -> SurfaceTokens {
    let path = theme_path();

    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return SurfaceTokens::panda(),
    };

    let file: ThemeFile = match toml::from_str(&contents) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("lunaris: failed to parse theme.toml: {e}, using Panda defaults");
            return SurfaceTokens::panda();
        }
    };

    let panda = SurfaceTokens::panda();
    let bg = file.color.as_ref().and_then(|c| c.bg.as_ref());
    let fg = file.color.as_ref().and_then(|c| c.fg.as_ref());

    SurfaceTokens {
        bg_shell:   bg.and_then(|b| b.shell.clone()).unwrap_or(panda.bg_shell),
        bg_app:     bg.and_then(|b| b.app.clone()).unwrap_or(panda.bg_app),
        bg_card:    bg.and_then(|b| b.card.clone()).unwrap_or(panda.bg_card),
        bg_overlay: bg.and_then(|b| b.overlay.clone()).unwrap_or(panda.bg_overlay),
        bg_input:   bg.and_then(|b| b.input.clone()).unwrap_or(panda.bg_input),
        fg_shell:   fg.and_then(|f| f.shell.clone()).unwrap_or(panda.fg_shell),
        fg_app:     fg.and_then(|f| f.app.clone()).unwrap_or(panda.fg_app),
        accent:     file.color.as_ref().and_then(|c| c.accent.clone()).unwrap_or(panda.accent),
        border:     file.color.as_ref().and_then(|c| c.border.clone()).unwrap_or(panda.border),
        radius:     file.radius.unwrap_or(panda.radius),
    }
}

/// Tauri command: load and return the current surface tokens.
#[tauri::command]
pub fn get_surface_tokens() -> SurfaceTokens {
    load_tokens()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panda_tokens_are_valid_hex() {
        let t = SurfaceTokens::panda();
        for color in [&t.bg_shell, &t.bg_app, &t.bg_card, &t.fg_shell, &t.fg_app, &t.accent, &t.border] {
            assert!(color.starts_with('#'), "expected hex: {color}");
        }
    }

    #[test]
    fn load_tokens_returns_panda_when_no_file() {
        // Set XDG_CONFIG_HOME to a nonexistent path so theme.toml won't be found.
        std::env::set_var("XDG_CONFIG_HOME", "/tmp/lunaris-test-nonexistent-99999");
        let tokens = load_tokens();
        assert_eq!(tokens.bg_shell, SurfaceTokens::panda().bg_shell);
        std::env::remove_var("XDG_CONFIG_HOME");
    }

    #[test]
    fn load_tokens_parses_theme_toml() {
        let dir = tempfile::tempdir().unwrap();
        let theme_dir = dir.path().join("lunaris");
        std::fs::create_dir_all(&theme_dir).unwrap();
        std::fs::write(
            theme_dir.join("theme.toml"),
            r##"
[color.bg]
shell = "#2d2d2d"
app = "#fafafa"

[color]
accent = "#ff6b6b"
"##,
        ).unwrap();

        std::env::set_var("XDG_CONFIG_HOME", dir.path());
        let tokens = load_tokens();
        assert_eq!(tokens.bg_shell, "#2d2d2d");
        assert_eq!(tokens.bg_app, "#fafafa");
        assert_eq!(tokens.accent, "#ff6b6b");
        // Unset values fall back to Panda
        assert_eq!(tokens.border, SurfaceTokens::panda().border);
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}
