//! Theme-specific Tauri commands.
//!
//! These are thin convenience wrappers around the generic `config_*`
//! commands, kept separate so the frontend can call them without
//! building dot-notation keys itself.

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use super::config::{config_get, config_reset, config_set, ConfigFile};

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

/// Resolve the active appearance: the active theme's base merged with the
/// `theme.toml` customization layer (the per-field overrides the Appearance
/// suite writes via `config_set(Customization, ...)`), resolved through
/// `sdk/theme`. NB the legacy `appearance.toml [overrides]` (accent/radius/font)
/// are a separate, superseded path and are not folded in here; the suite writes
/// overrides to `theme.toml`.
fn resolve_active_theme() -> Result<arlen_theme::ArlenTheme, String> {
    let id = get_active_theme_id()?;
    let base =
        active_theme_content(&id).ok_or_else(|| format!("active theme '{id}' not found"))?;
    let customization =
        std::fs::read_to_string(arlen_theme::ArlenTheme::user_customization_path()).ok();
    arlen_theme::ArlenTheme::resolve(&base, None, customization.as_deref())
        .map_err(|e| format!("resolve: {e}"))
}

/// The resolved colour palette of the active appearance: every semantic role's
/// hex so the Appearance preview and per-field override rows render the real
/// theme instead of a fixture.
#[tauri::command]
pub fn theme_resolved_palette() -> Result<Vec<PaletteRole>, String> {
    Ok(palette_of(&resolve_active_theme()?))
}

/// One foreground-over-background pair audited for contrast, serialized for the
/// Accessibility page's contrast surface: the human label, both measures (the
/// WCAG 2.x ratio and the signed APCA `Lc`), the floor the pair is held to
/// (`body` or `large`), and whether it clears each. `apca` is signed only for
/// polarity; the pass tests its magnitude.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContrastRole {
    /// Human label, e.g. `"fg.primary on bg.app"`.
    pub pair: String,
    /// The WCAG 2.x contrast ratio (`1.0..=21.0`).
    pub wcag: f32,
    /// The signed APCA `Lc`.
    pub apca: f32,
    /// The floor this pair is held to: `"body"` (WCAG 4.5 / APCA 60) or
    /// `"large"` (WCAG 3.0 / APCA 45).
    pub usage: String,
    /// The WCAG ratio clears its AA floor.
    pub wcag_pass: bool,
    /// The APCA magnitude clears its bronze floor.
    pub apca_pass: bool,
}

/// Audit the active appearance's key foreground/background pairs against the
/// WCAG 2.2 AA and APCA bronze floors, so the Accessibility page can surface any
/// illegible pair (WCAG 2.x is the legal floor for EN 301 549, APCA the
/// perceptual read). The compute lives in `sdk/theme`; this resolves the live
/// theme and serializes the findings for the frontend to render.
#[tauri::command]
pub fn theme_contrast_report() -> Result<Vec<ContrastRole>, String> {
    use arlen_theme::color::{contrast_report, ContrastUse};
    let theme = resolve_active_theme()?;
    Ok(contrast_report(&theme.color)
        .into_iter()
        .map(|f| ContrastRole {
            pair: f.pair.to_string(),
            wcag: f.wcag,
            apca: f.apca,
            usage: match f.usage {
                ContrastUse::Body => "body".to_string(),
                ContrastUse::Large => "large".to_string(),
            },
            wcag_pass: f.wcag_pass,
            apca_pass: f.apca_pass,
        })
        .collect())
}

/// The resolved terminal colours for the Appearance terminal-colour editor:
/// foreground, background, cursor, and the 16 ANSI slots (0-7 normal, 8-15
/// bright) of the active appearance, as hex. The editor writes slot edits back
/// via `config_set(Customization, "terminal.ansi....", ...)`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalPalette {
    /// Foreground colour.
    pub fg: String,
    /// Background colour.
    pub bg: String,
    /// Cursor colour.
    pub cursor: String,
    /// The 16 ANSI colours, hex.
    pub ansi: Vec<String>,
}

/// Resolve the active appearance's terminal colours (see [`TerminalPalette`]).
#[tauri::command]
pub fn theme_resolved_terminal() -> Result<TerminalPalette, String> {
    use arlen_theme::gtk::rgba_to_hex;
    let theme = resolve_active_theme()?;
    let t = &theme.terminal;
    Ok(TerminalPalette {
        fg: rgba_to_hex(t.fg),
        bg: rgba_to_hex(t.bg),
        cursor: rgba_to_hex(t.cursor),
        ansi: t.ansi.iter().map(|c| rgba_to_hex(*c)).collect(),
    })
}

/// One resolved sound binding for the Appearance sound page: a system event, a
/// human label, and the freedesktop XDG sound name the Notification Daemon plays.
#[derive(Debug, Clone, Serialize)]
pub struct SoundBinding {
    /// The event key (`notification` / `error` / `warning` / `action`).
    pub event: String,
    /// A human label for the page.
    pub label: String,
    /// The resolved freedesktop XDG sound name.
    pub sound: String,
}

/// The six default-on sound events mapped to their resolved names. Pure over the
/// resolved [`arlen_theme::SoundTokens`] so it is testable without config I/O.
fn sound_bindings(s: &arlen_theme::SoundTokens) -> Vec<SoundBinding> {
    vec![
        SoundBinding {
            event: "notification".into(),
            label: "Notification".into(),
            sound: s.notification.clone(),
        },
        SoundBinding { event: "error".into(), label: "Error".into(), sound: s.error.clone() },
        SoundBinding { event: "warning".into(), label: "Warning".into(), sound: s.warning.clone() },
        SoundBinding {
            event: "action".into(),
            label: "Action completion".into(),
            sound: s.action.clone(),
        },
        SoundBinding {
            event: "device-added".into(),
            label: "Device attached".into(),
            sound: s.device_added.clone(),
        },
        SoundBinding {
            event: "device-removed".into(),
            label: "Device removed".into(),
            sound: s.device_removed.clone(),
        },
    ]
}

/// The active appearance's resolved event-to-sound map (the six default-on
/// events), so the Appearance sound page shows the real bindings instead of a
/// fixture. Resolved through `sdk/theme` - the same `SoundTokens` the Notification
/// Daemon plays - so the page and playback agree.
#[tauri::command]
pub fn theme_resolved_sounds() -> Result<Vec<SoundBinding>, String> {
    Ok(sound_bindings(&resolve_active_theme()?.sounds))
}

/// Recursively merge `over` onto `base`: tables merge key-by-key, and `over`
/// wins on any leaf. Used to overlay the customization onto a base theme so the
/// export carries both.
fn merge_toml(base: &mut toml::Value, over: &toml::Value) {
    match (base, over) {
        (toml::Value::Table(b), toml::Value::Table(o)) => {
            for (k, ov) in o {
                match b.get_mut(k) {
                    Some(bv) => merge_toml(bv, ov),
                    None => {
                        b.insert(k.clone(), ov.clone());
                    }
                }
            }
        }
        (b, o) => *b = o.clone(),
    }
}

/// Export the active appearance as one self-contained theme TOML: the active
/// theme's base overlaid with the `theme.toml` customization (the per-field
/// overrides), so the result re-imports as a theme carrying every current edit
/// (the "Generate Theme From Current Settings" flow). The importer resolves any
/// `extends` and defaults unset dimensions, so the file is a valid theme; the
/// caller's save flow renames it (the export keeps the base `[meta]`).
#[tauri::command]
pub fn theme_export() -> Result<String, String> {
    let id = get_active_theme_id()?;
    let base =
        active_theme_content(&id).ok_or_else(|| format!("active theme '{id}' not found"))?;
    let mut merged: toml::Value =
        toml::from_str(&base).map_err(|e| format!("parse base theme: {e}"))?;
    if let Ok(custom) =
        std::fs::read_to_string(arlen_theme::ArlenTheme::user_customization_path())
    {
        if let Ok(over) = toml::from_str::<toml::Value>(&custom) {
            merge_toml(&mut merged, &over);
        }
    }
    toml::to_string_pretty(&merged).map_err(|e| format!("serialize theme: {e}"))
}

/// The resolved non-colour metrics of the active appearance (radius, spacing,
/// motion, typography, depth) as a flat `dotted-key -> value` map, so the
/// Appearance dimension pages render each field's resolved value. Numbers are
/// stringified and the frontend reads the keys it needs; the colour dimensions
/// are in [`theme_resolved_palette`] / [`theme_resolved_terminal`].
#[tauri::command]
pub fn theme_resolved_metrics() -> Result<std::collections::BTreeMap<String, String>, String> {
    let t = resolve_active_theme()?;
    let mut m = std::collections::BTreeMap::new();
    // Radius (authored base + intensity, matching what the override rows edit).
    m.insert("radius.chip".into(), t.radius.chip.to_string());
    m.insert("radius.button".into(), t.radius.button.to_string());
    m.insert("radius.input".into(), t.radius.input.to_string());
    m.insert("radius.card".into(), t.radius.card.to_string());
    m.insert("radius.modal".into(), t.radius.modal.to_string());
    m.insert("radius.full".into(), t.radius.full.to_string());
    m.insert("radius.intensity".into(), t.radius.intensity.to_string());
    m.insert(
        "radius.window_corners".into(),
        t.radius
            .window_corners
            .iter()
            .map(|c| c.to_string())
            .collect::<Vec<_>>()
            .join(","),
    );
    // Spacing.
    m.insert("spacing.xs".into(), t.spacing.xs.clone());
    m.insert("spacing.sm".into(), t.spacing.sm.clone());
    m.insert("spacing.md".into(), t.spacing.md.clone());
    m.insert("spacing.lg".into(), t.spacing.lg.clone());
    m.insert("spacing.xl".into(), t.spacing.xl.clone());
    // Motion.
    m.insert("motion.duration_fast".into(), t.motion.duration_fast.clone());
    m.insert("motion.duration_normal".into(), t.motion.duration_normal.clone());
    m.insert("motion.duration_slow".into(), t.motion.duration_slow.clone());
    m.insert("motion.easing_default".into(), t.motion.easing_default.clone());
    m.insert("motion.easing_spring".into(), t.motion.easing_spring.clone());
    // Typography.
    m.insert("typography.font_sans".into(), t.typography.font_sans.clone());
    m.insert("typography.font_mono".into(), t.typography.font_mono.clone());
    m.insert("typography.size_base".into(), t.typography.size_base.clone());
    m.insert("typography.line_height".into(), t.typography.line_height.clone());
    m.insert("typography.weight_normal".into(), t.typography.weight_normal.to_string());
    m.insert("typography.weight_medium".into(), t.typography.weight_medium.to_string());
    m.insert("typography.weight_bold".into(), t.typography.weight_bold.to_string());
    // Depth.
    m.insert("depth.shadow_sm".into(), t.depth.shadow_sm.clone());
    m.insert("depth.shadow_md".into(), t.depth.shadow_md.clone());
    m.insert("depth.shadow_lg".into(), t.depth.shadow_lg.clone());
    m.insert("depth.shadow_card".into(), t.depth.shadow_card.clone());
    m.insert("depth.blur_enabled".into(), t.depth.blur_enabled.to_string());
    Ok(m)
}

/// Persist one appearance metric override.
///
/// The write counterpart to [`theme_resolved_metrics`], which had none: the theme
/// backend exposed eleven READ commands and exactly two writes (the accent and the
/// active theme), so every override row in the Appearance suite edited an
/// in-memory store that was never persisted and never applied.
///
/// `key` is validated against the same key set the read command emits, so the
/// frontend can only write metric overrides and cannot reach an arbitrary dotted
/// path in the config. That agreement is test-pinned rather than restated: a
/// metric that becomes readable but not writable, or the reverse, is a row that
/// silently does nothing.
///
/// WHERE IT WRITES, AND WHY IT MOVED. This wrote `overrides.<key>` into
/// `appearance.toml` until 19 Aug, and nothing on the reading side ever looked
/// there. The `[overrides]` table deserialises into `UserOverrides`, which holds
/// exactly three fields - `accent`, `font_scale`, `radius_intensity` - with no
/// catch-all, so `overrides.depth.shadow_card` landed on disk and was dropped by
/// the loader. The command was added to fix rows that "edited an in-memory store
/// that was never persisted and never applied", and it got as far as persisted.
///
/// The per-field channel is `theme.toml`, sdk/theme's layer 3, which the resolver
/// merges field-by-field over the active theme. `ConfigFile::Customization` was
/// already defined for it and its doc already said the Appearance suite writes it;
/// nothing did. The metric keys are `section.field` and the theme file is grouped
/// into exactly those sections, so the key IS the path - no prefix.
///
/// TYPES MATTER HERE. The read command stringifies everything, so the frontend
/// round-trips strings, but `radius.button` is an `f32` in the schema and
/// `depth.blur_enabled` a `bool`. Writing those as TOML strings would make the
/// merged theme fail to parse, which is worse than being ignored: an unreadable
/// customization file takes the whole theme down rather than one row. So the value
/// is converted to the type the schema declares, and a value that will not convert
/// is refused rather than written.
#[tauri::command]
pub async fn theme_set_metric(key: String, value: String) -> Result<(), String> {
    if !is_known_metric(&key) {
        return Err(format!("not an appearance metric: {key}"));
    }
    config_set(ConfigFile::Customization, key.clone(), metric_value(&key, &value)?).await
}

/// The TOML type a metric holds, from the theme schema rather than from the
/// stringified read.
///
/// Hand-kept, and pinned by a test that every key `theme_resolved_metrics` emits
/// appears here: a metric that gains a row but no type would be written as a
/// string and break the file it is written into.
fn metric_value(key: &str, value: &str) -> Result<serde_json::Value, String> {
    let number = |v: &str| {
        v.parse::<f64>()
            .map_err(|_| format!("{key} is a number and {v:?} is not one"))
            .and_then(|n| {
                serde_json::Number::from_f64(n)
                    .map(serde_json::Value::Number)
                    .ok_or_else(|| format!("{key} cannot hold {v:?}"))
            })
    };
    match key {
        // Radii are floats; `window_corners` is the four-corner array and is not
        // offered as a metric row, so it needs no case here.
        k if k.starts_with("radius.") => number(value),
        // Weights are `u32` in the schema and must be written as INTEGERS. This
        // arm used to share the float path on the reasoning that "a JSON number
        // covers both and toml_edit writes an integer for a whole value", which is
        // simply untrue: it wrote `weight_bold = 800.0`, and the resolver then
        // refused the ENTIRE file with `invalid type: floating point, expected
        // u32`. One font-weight edit took the whole theme down. Found by writing
        // one and reading it back through the real resolver, not by reading this.
        "typography.weight_normal" | "typography.weight_medium" | "typography.weight_bold" => value
            .parse::<u32>()
            .map(|n| serde_json::Value::Number(n.into()))
            .map_err(|_| format!("{key} is a whole number and {value:?} is not one")),
        "depth.blur_enabled" => match value {
            "true" => Ok(serde_json::Value::Bool(true)),
            "false" => Ok(serde_json::Value::Bool(false)),
            other => Err(format!("{key} is a switch and {other:?} is neither on nor off")),
        },
        // Everything else is a CSS-ish string the theme carries verbatim: spacing
        // lengths, durations, easings, font families, shadows.
        _ => Ok(serde_json::Value::String(value.to_string())),
    }
}

/// The colour overrides `theme.toml` currently holds, keyed by role.
///
/// The page needs this to open on what is actually set, and to light the reset
/// affordance beside a row that is overridden. It is a BACKEND read rather than
/// the page walking the config itself, because walking it means inverting
/// [`color_role_path`] on the frontend - a second copy of the rule, in another
/// language, that drifts silently: a role the inverse misses reads as
/// not-overridden while the file holds it and the desktop shows it.
#[tauri::command]
pub fn theme_color_overrides() -> Result<std::collections::BTreeMap<String, String>, String> {
    let doc = config_get(ConfigFile::Customization, Some("color".into()))?;
    let mut out = std::collections::BTreeMap::new();
    let Some(groups) = doc.as_object() else {
        return Ok(out); // absent or not a table: nothing is overridden
    };
    // Driven from the roles themselves, so this cannot report a key the setter
    // could not have written.
    for role in palette_of(&resolve_active_theme()?).into_iter().map(|r| r.role) {
        let Some(path) = color_role_path(&role) else { continue };
        let field = path.trim_start_matches("color.");
        let Some((group, name)) = field.split_once('.') else { continue };
        if let Some(hex) = groups.get(group).and_then(|g| g.get(name)).and_then(|v| v.as_str()) {
            out.insert(role, hex.to_string());
        }
    }
    Ok(out)
}

/// Persist one colour-role override, or clear it when `hex` is `None`.
///
/// The colour pickers had no write path either: `themeColors.ts` held its
/// overrides in a store nothing loaded and nothing saved, so the Appearance
/// palette editor was a colour picker attached to a variable.
///
/// Unlike the System page's fields, the path here is DERIVED rather than mapped.
/// A role name carries its own group (`bg_card` lives at `color.bg.card`,
/// `border_strong` at `color.border.strong`), so there is one rule instead of two
/// hand-kept tables that can disagree - and because clearing goes through this
/// command too, the rule never has to be repeated on the frontend.
#[tauri::command]
pub async fn theme_set_color(role: String, hex: Option<String>) -> Result<(), String> {
    let path = color_role_path(&role).ok_or_else(|| format!("not a colour role: {role}"))?;
    match hex {
        None => config_reset(ConfigFile::Customization, Some(path)),
        Some(hex) => {
            // Parsed rather than pattern-matched: the resolver drops a value it
            // cannot read, so an unparseable colour would save, do nothing, and
            // leave the picker showing a colour the machine never took.
            if arlen_theme::parse_hex(&hex).is_none() {
                return Err(format!("{hex:?} is not a colour"));
            }
            config_set(ConfigFile::Customization, path, serde_json::Value::String(hex)).await
        }
    }
}

/// The theme-file path a colour role writes to, derived from its own name.
///
/// `None` for anything that is not a role, which is what keeps this from being a
/// general write into the theme file.
fn color_role_path(role: &str) -> Option<String> {
    let (group, field) = match role {
        "accent" | "accent_hover" | "accent_pressed" | "success" | "warning" | "error" | "info" => {
            ("semantic", role)
        }
        _ => {
            let (prefix, rest) = role.split_once('_')?;
            match prefix {
                "bg" | "fg" => (prefix, rest),
                "border" => ("border", rest),
                _ => return None,
            }
        }
    };
    Some(format!("color.{group}.{field}"))
}

/// Persist one Appearance > System field.
///
/// The sibling of [`theme_set_metric`] for the fields that page owns: the cursor
/// and icon theme, the six sound cues, and the terminal palette. They had no write
/// path at all - `themeSystem.ts` held them in a store that nothing loaded and
/// nothing saved, so every control on that page was a knob attached to a variable
/// that died with the window.
///
/// The frontend key is a flat camelCase name (`ansi0`, `sndError`, `cursorTheme`)
/// and the destination is a dotted path in the theme file. The mapping lives here
/// rather than in TypeScript because the schema does, and a mapping written on the
/// far side of the bridge drifts from it silently.
#[tauri::command]
pub async fn theme_set_system(key: String, value: Option<String>) -> Result<(), String> {
    let path = system_key_path(&key).ok_or_else(|| format!("not a system field: {key}"))?;
    let Some(value) = value else {
        // Clearing goes through here too, so the field-to-path table stays in one
        // place. It briefly lived in the store as well, for the generic
        // `config_reset`, and two copies of a map like this drift silently: a
        // wrong path does not throw, it deletes nothing, and the row resets on
        // screen while the file keeps the override.
        return config_reset(ConfigFile::Customization, Some(path.to_string()));
    };
    let value = match key.as_str() {
        // The only numeric one; the rest are theme names, cue names and hex.
        "cursorSize" => value
            .parse::<u32>()
            .map(|n| serde_json::Value::Number(n.into()))
            .map_err(|_| format!("cursor size is a number and {value:?} is not one"))?,
        _ => serde_json::Value::String(value),
    };
    config_set(ConfigFile::Customization, path.to_string(), value).await
}

/// The System-page overrides `theme.toml` holds, keyed by the page's own field
/// names, so the page opens on what is set without inverting the path table on
/// the frontend.
#[tauri::command]
pub fn theme_system_overrides() -> Result<std::collections::BTreeMap<String, String>, String> {
    let doc = config_get(ConfigFile::Customization, None)?;
    let mut out = std::collections::BTreeMap::new();
    for key in SYSTEM_FIELDS {
        let Some(path) = system_key_path(key) else { continue };
        let mut node = &doc;
        for part in path.split('.') {
            match node.get(part) {
                Some(next) => node = next,
                None => {
                    node = &serde_json::Value::Null;
                    break;
                }
            }
        }
        // Numbers come back as numbers; the page takes strings either way.
        match node {
            serde_json::Value::String(v) => {
                out.insert(key.to_string(), v.clone());
            }
            serde_json::Value::Number(n) => {
                out.insert(key.to_string(), n.to_string());
            }
            _ => {}
        }
    }
    Ok(out)
}

/// Every field the System page owns. Kept beside [`system_key_path`] and pinned
/// against it by a test, so a field that gains a path but not an entry here is
/// writable and invisible on load.
const SYSTEM_FIELDS: &[&str] = &[
    "cursorTheme",
    "cursorSize",
    "iconTheme",
    "sndNotification",
    "sndError",
    "sndWarning",
    "sndAction",
    "sndDeviceAdded",
    "sndDeviceRemoved",
    "termFg",
    "termBg",
    "ansi0",
    "ansi1",
    "ansi2",
    "ansi3",
    "ansi4",
    "ansi5",
    "ansi6",
    "ansi7",
    "ansi8",
    "ansi9",
    "ansi10",
    "ansi11",
    "ansi12",
    "ansi13",
    "ansi14",
    "ansi15",
];

/// The theme-file path a System field writes to, or `None` if the key names no
/// field.
///
/// Refusing an unknown key is what keeps this from being a general write into the
/// theme file: the frontend can reach exactly these paths and nothing else.
fn system_key_path(key: &str) -> Option<&'static str> {
    Some(match key {
        "cursorTheme" => "cursor.theme",
        "cursorSize" => "cursor.size",
        "iconTheme" => "icons.theme",
        "sndNotification" => "sounds.notification",
        "sndError" => "sounds.error",
        "sndWarning" => "sounds.warning",
        "sndAction" => "sounds.action",
        "sndDeviceAdded" => "sounds.device_added",
        "sndDeviceRemoved" => "sounds.device_removed",
        "termFg" => "terminal.fg",
        "termBg" => "terminal.bg",
        "ansi0" => "terminal.ansi.black",
        "ansi1" => "terminal.ansi.red",
        "ansi2" => "terminal.ansi.green",
        "ansi3" => "terminal.ansi.yellow",
        "ansi4" => "terminal.ansi.blue",
        "ansi5" => "terminal.ansi.magenta",
        "ansi6" => "terminal.ansi.cyan",
        "ansi7" => "terminal.ansi.white",
        "ansi8" => "terminal.ansi.bright_black",
        "ansi9" => "terminal.ansi.bright_red",
        "ansi10" => "terminal.ansi.bright_green",
        "ansi11" => "terminal.ansi.bright_yellow",
        "ansi12" => "terminal.ansi.bright_blue",
        "ansi13" => "terminal.ansi.bright_magenta",
        "ansi14" => "terminal.ansi.bright_cyan",
        "ansi15" => "terminal.ansi.bright_white",
        // `soundsEnabled` and `soundTheme` are the notification daemon's, not the
        // theme's: the theme names which CUE an event plays, the daemon decides
        // whether sound happens at all and from which installed theme. They go
        // through the Notifications config, which the page already reaches.
        _ => return None,
    })
}

/// Whether `key` names a metric [`theme_resolved_metrics`] reports.
///
/// Derived from the read command itself rather than from a second hand-kept list,
/// because the two drifting apart is the failure this guard exists to prevent.
fn is_known_metric(key: &str) -> bool {
    theme_resolved_metrics().is_ok_and(|m| m.contains_key(key))
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

/// The XDG GTK-theme search directories: `/usr/share/themes`, the user data
/// dir's `themes/`, and legacy `~/.themes`. A missing directory is skipped.
fn gtk_theme_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = vec![std::path::PathBuf::from("/usr/share/themes")];
    if let Some(data) = dirs::data_dir() {
        dirs.push(data.join("themes"));
    }
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".themes"));
    }
    dirs
}

/// Whether the `adw-gtk3` GTK theme is installed - the prerequisite the Toolkits
/// page names for full GTK3 shape. An `adw-gtk3` or `adw-gtk3-dark` directory
/// under any GTK-theme dir counts. Pure over the search dirs so it is testable.
fn adw_gtk3_present(gtk_theme_dirs: &[std::path::PathBuf]) -> bool {
    gtk_theme_dirs
        .iter()
        .any(|d| d.join("adw-gtk3").is_dir() || d.join("adw-gtk3-dark").is_dir())
}

/// Whether qt6ct is the configured Qt platform theme - the prerequisite for the
/// Fusion-shaped Qt colours the generator targets. Met when
/// `QT_QPA_PLATFORMTHEME=qt6ct` or a `qt6ct.conf` is present. Pure over its two
/// inputs so it is testable.
fn qt6ct_configured(platform_theme: Option<&str>, conf_present: bool) -> bool {
    platform_theme == Some("qt6ct") || conf_present
}

/// The DETECTABLE toolkit prerequisites keyed by toolkit id (matching the
/// Toolkits page's `Toolkit.id`): `gtk3` needs `adw-gtk3` installed, `qt` needs
/// `qt6ct` configured. The other toolkits (arlen/gtk4/terminal/wine) carry only
/// informational notes, not a detectable prerequisite, so they are not reported.
/// Pure over its inputs.
fn detect_toolkit_prereqs(
    gtk_theme_dirs: &[std::path::PathBuf],
    qt6ct_ready: bool,
) -> std::collections::BTreeMap<String, bool> {
    let mut prereqs = std::collections::BTreeMap::new();
    prereqs.insert("gtk3".to_string(), adw_gtk3_present(gtk_theme_dirs));
    prereqs.insert("qt".to_string(), qt6ct_ready);
    prereqs
}

/// Whether each detectable toolkit prerequisite is met on this system, so the
/// Toolkits page shows an HONEST status instead of the fixture: `{ "gtk3": bool,
/// "qt": bool }` (adw-gtk3 installed, qt6ct configured). The other toolkits carry
/// only informational notes and are not reported.
#[tauri::command]
pub fn theme_toolkit_prereqs() -> std::collections::BTreeMap<String, bool> {
    let conf_present = dirs::config_dir()
        .map(|c| c.join("qt6ct").join("qt6ct.conf").is_file())
        .unwrap_or(false);
    let platform_theme = std::env::var("QT_QPA_PLATFORMTHEME").ok();
    let qt6ct_ready = qt6ct_configured(platform_theme.as_deref(), conf_present);
    detect_toolkit_prereqs(&gtk_theme_dirs(), qt6ct_ready)
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

    /// Every metric the read command reports converts to the type the theme
    /// schema declares for it.
    ///
    /// The failure this guards is quiet and total: a numeric field written as a
    /// TOML string makes `theme.toml` fail to deserialise, and a customization
    /// file that will not parse does not lose one row, it takes the whole theme
    /// down. So a new metric that arrives without a type entry has to fail here
    /// rather than on the user's machine.
    #[test]
    fn every_metric_converts_to_the_type_the_schema_declares() {
        let Ok(metrics) = super::theme_resolved_metrics() else {
            return; // no resolvable theme here; the shape cases below still hold
        };
        for (key, resolved) in metrics {
            if key == "radius.window_corners" {
                continue; // the four-corner array is not offered as a metric row
            }
            let v = super::metric_value(&key, &resolved)
                .unwrap_or_else(|e| panic!("{key} does not round-trip its own resolved value: {e}"));
            let want_number = key.starts_with("radius.") || key.starts_with("typography.weight_");
            let want_bool = key == "depth.blur_enabled";
            assert_eq!(v.is_number(), want_number, "{key} number-ness");
            assert_eq!(v.is_boolean(), want_bool, "{key} bool-ness");

            // The part that matters, and the part number-ness alone missed: write
            // it the way the command would and hand the file to the RESOLVER. A
            // font weight went out as `800.0` for a `u32` field and the resolver
            // refused the whole file - one edit, whole theme gone - while this
            // test was happily asserting that 800.0 is a number.
            let (section, field) = key.rsplit_once('.').expect("dotted");
            // Rendered through the same converter the write path uses. Rendering
            // it by hand here produced invalid TOML for the font stacks, which
            // carry their own quotes - the test would have reported a product
            // failure that was its own.
            let rendered =
                arlen_settings_core::config::json_to_toml_edit(v.clone()).to_string();
            let doc = format!("[{section}]\n{field} = {rendered}\n");
            arlen_theme::ArlenTheme::resolve(arlen_theme::DARK_TOML, Some(&doc), None)
                .unwrap_or_else(|e| panic!("{key} written as `{rendered}` breaks the theme: {e}"));
        }
    }

    /// A value of the wrong shape is refused rather than written. Writing
    /// `"maybe"` into a boolean field is the same total failure as writing a
    /// string into a float one.
    #[test]
    fn a_value_that_is_not_the_declared_type_is_refused() {
        assert!(super::metric_value("radius.button", "quite round").is_err());
        assert!(super::metric_value("depth.blur_enabled", "maybe").is_err());
        assert!(super::metric_value("typography.weight_bold", "heavy").is_err());
        // And the string fields take what they are given, since the theme carries
        // them verbatim.
        assert!(super::metric_value("spacing.md", "0.75rem").is_ok());
    }

    /// Every role the palette reports derives a path the resolver reads back.
    ///
    /// Stronger than checking the strings against each other: each derived path
    /// is written into a theme file and the colour is read back out through the
    /// real resolver, so a wrong group (`color.semantic.card`) shows up as a
    /// value that does not arrive rather than as a plausible-looking string.
    #[test]
    fn every_palette_role_derives_a_path_the_resolver_reads_back() {
        let Ok(roles) = super::theme_resolved_palette() else {
            return; // no resolvable theme here
        };
        assert!(!roles.is_empty(), "the palette reported nothing to check");
        for r in roles {
            let path = super::color_role_path(&r.role)
                .unwrap_or_else(|| panic!("{} has no path", r.role));
            let (section, field) = path.rsplit_once('.').expect("dotted");
            let doc = format!("[{section}]\n{field} = \"#123456\"\n");
            let t = arlen_theme::ArlenTheme::resolve(arlen_theme::DARK_TOML, Some(&doc), None)
                .unwrap_or_else(|e| panic!("{} writes an unresolvable file: {e}", r.role));
            let arrived = super::palette_of(&t)
                .into_iter()
                .find(|p| p.role == r.role)
                .map(|p| p.hex)
                .unwrap_or_default();
            assert_eq!(
                arrived.to_lowercase(),
                "#123456",
                "{} written to {path} did not come back out of the resolver",
                r.role
            );
        }
    }

    /// A name that is not a role has no path, so the command cannot be talked
    /// into writing anywhere else in the theme file.
    #[test]
    fn a_name_that_is_not_a_role_has_no_path() {
        for bad in ["", "accent_extra", "radius.card", "bg", "wm_titlebar", "../etc"] {
            assert!(super::color_role_path(bad).is_none(), "{bad:?} must have no path");
        }
    }

    /// Every field the System page offers has a path, and every path names a
    /// field the theme schema really has.
    ///
    /// The list is spelled out on both sides rather than derived, because the two
    /// ends are a flat camelCase key and a dotted schema path and there is no
    /// mechanical relation between them. What CAN be checked is that no path is a
    /// typo: each one is written into a theme file and read back through the
    /// resolver, so a misspelled section or field shows up as a value that does
    /// not arrive.
    #[test]
    fn every_system_field_writes_a_path_the_resolver_reads_back() {
        // One representative per shape: a theme name, a number, a cue, a terminal
        // colour and an ANSI slot. Writing all 27 would test toml_edit, not the
        // mapping.
        let checks: &[(&str, &str, &str)] = &[
            ("cursorTheme", "Bibata", "cursor.theme"),
            ("iconTheme", "Papirus", "icons.theme"),
            ("sndError", "bell", "sounds.error"),
            ("sndDeviceRemoved", "device-removed", "sounds.device_removed"),
            ("termBg", "#101010", "terminal.bg"),
            ("ansi9", "#ff0000", "terminal.ansi.bright_red"),
        ];
        for (key, value, want_path) in checks {
            let path = super::system_key_path(key).unwrap_or_else(|| panic!("{key} has no path"));
            assert_eq!(&path, want_path, "{key} path");
            // The path must land somewhere the resolver looks. Build a one-field
            // theme file at that path and check the value comes back out.
            let (section, field) = path.rsplit_once('.').expect("dotted");
            let doc = format!("[{section}]\n{field} = \"{value}\"\n");
            let t = arlen_theme::ArlenTheme::resolve(arlen_theme::DARK_TOML, Some(&doc), None)
                .unwrap_or_else(|e| panic!("{key} writes an unresolvable file: {e}"));
            let arrived = match *key {
                "cursorTheme" => t.cursor.theme.clone(),
                "iconTheme" => t.icons.theme.clone(),
                "sndError" => t.sounds.error.clone(),
                "sndDeviceRemoved" => t.sounds.device_removed.clone(),
                "termBg" => arlen_theme::gtk::rgba_to_hex(t.terminal.bg),
                "ansi9" => arlen_theme::gtk::rgba_to_hex(t.terminal.ansi[9]),
                other => panic!("unhandled check {other}"),
            };
            assert_eq!(
                arrived.to_lowercase(),
                value.to_lowercase(),
                "{key} written to {path} did not come back out of the resolver"
            );
        }
    }

    /// The field list the load path walks and the path table the write path uses
    /// name the same fields.
    ///
    /// They are two consts beside each other, which is the smallest form the
    /// duplication takes: a field with a path but no list entry is writable and
    /// invisible when the page reopens, which reads as "it did not save".
    #[test]
    fn the_system_field_list_and_the_path_table_cover_the_same_fields() {
        for key in super::SYSTEM_FIELDS {
            assert!(super::system_key_path(key).is_some(), "{key} is listed with no path");
        }
        // And the other direction, from the page's own key set rather than from a
        // third list: every key the store can send must be in SYSTEM_FIELDS.
        let store = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../src/lib/stores/themeSystem.ts"
        ))
        .expect("the store is beside the backend");
        let defaults = store
            .split("SYS_DEFAULTS: Record<string, string | number | boolean> = {")
            .nth(1)
            .expect("SYS_DEFAULTS is gone from the store");
        let defaults = &defaults[..defaults.find("\n};").expect("terminated")];
        for line in defaults.lines() {
            let Some((name, _)) = line.trim().split_once(':') else { continue };
            let name = name.trim();
            if name.is_empty() || name.starts_with("//") {
                continue;
            }
            // The two the notification daemon owns are deliberately not fields of
            // the theme; every other default must be one.
            if name == "soundsEnabled" || name == "soundTheme" {
                continue;
            }
            assert!(
                super::SYSTEM_FIELDS.contains(&name),
                "{name} is a control on the page and the backend cannot save it"
            );
        }
    }

    /// A key the page does not own is refused rather than written. The two sound
    /// switches belong to the notification daemon, and an unknown key must not
    /// become a free write into the theme file.
    #[test]
    fn a_key_the_page_does_not_own_has_no_path() {
        assert!(super::system_key_path("soundsEnabled").is_none());
        assert!(super::system_key_path("soundTheme").is_none());
        assert!(super::system_key_path("color.accent").is_none());
        assert!(super::system_key_path("").is_none());
    }

    /// The guard the write command rests on: it accepts exactly what the read
    /// command reports. A key readable but not writable is a row that silently
    /// does nothing; a key writable but not readable is a value nothing shows.
    #[test]
    fn every_reported_metric_is_writable_and_nothing_else_is() {
        let Ok(metrics) = super::theme_resolved_metrics() else {
            // No resolvable theme in this environment; the agreement is still
            // pinned by the negative cases below.
            assert!(!super::is_known_metric("typography.size_base"));
            return;
        };
        for key in metrics.keys() {
            assert!(super::is_known_metric(key), "{key} is reported but not writable");
        }
        for bad in [
            "overrides.accent",
            "ai.enabled",
            "typography",
            "typography.size_base.extra",
            "",
            "../../etc/passwd",
        ] {
            assert!(!super::is_known_metric(bad), "{bad:?} must not be writable");
        }
    }

    #[test]
    fn sound_bindings_map_every_event_from_the_resolved_theme() {
        let theme = arlen_theme::ArlenTheme::from_bundled(arlen_theme::DARK_TOML).unwrap();
        let bindings = sound_bindings(&theme.sounds);
        let events: Vec<&str> = bindings.iter().map(|b| b.event.as_str()).collect();
        // Four until 19 Aug, when the theme gained device-added and
        // device-removed. The list is spelled out rather than counted, so an
        // event that appears in the schema without a label and a resolved name
        // fails here instead of rendering as a blank row.
        assert_eq!(
            events,
            ["notification", "error", "warning", "action", "device-added", "device-removed"]
        );
        // Every binding carries a non-empty resolved freedesktop sound name.
        assert!(bindings.iter().all(|b| !b.sound.is_empty() && !b.label.is_empty()));
    }

    #[test]
    fn adw_gtk3_is_detected_only_when_a_theme_dir_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = vec![tmp.path().to_path_buf()];
        assert!(!adw_gtk3_present(&dirs), "absent by default");
        std::fs::create_dir(tmp.path().join("adw-gtk3-dark")).unwrap();
        assert!(adw_gtk3_present(&dirs), "the -dark variant counts");
    }

    #[test]
    fn qt6ct_is_configured_by_env_or_a_conf_file() {
        assert!(qt6ct_configured(Some("qt6ct"), false), "env platform theme");
        assert!(qt6ct_configured(None, true), "a present qt6ct.conf");
        assert!(!qt6ct_configured(Some("gtk3"), false), "neither");
        assert!(!qt6ct_configured(None, false), "neither");
    }

    #[test]
    fn toolkit_prereqs_reports_the_two_detectable_toolkits() {
        let tmp = tempfile::tempdir().unwrap();
        let got = detect_toolkit_prereqs(&[tmp.path().to_path_buf()], true);
        assert_eq!(got.get("gtk3"), Some(&false));
        assert_eq!(got.get("qt"), Some(&true));
        // The informational-only toolkits are not reported.
        assert!(!got.contains_key("terminal"));
        assert!(!got.contains_key("wine"));
    }

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

    #[test]
    fn merge_toml_overlays_leaves_and_keeps_base_fields() {
        let mut base: toml::Value =
            toml::from_str("[color]\naccent = \"#111111\"\nbg_app = \"#000000\"\n").unwrap();
        let over: toml::Value = toml::from_str("[color]\naccent = \"#ff0000\"\n").unwrap();
        merge_toml(&mut base, &over);
        let color = base.get("color").unwrap();
        // The override wins on the leaf it sets.
        assert_eq!(color.get("accent").unwrap().as_str(), Some("#ff0000"));
        // A base leaf the override does not touch is preserved.
        assert_eq!(color.get("bg_app").unwrap().as_str(), Some("#000000"));
    }
}

/// One sound theme the machine actually has.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SoundThemeOption {
    /// The directory name, which is what `[sound] theme` stores.
    pub id: String,
    /// What to show a person.
    pub name: String,
    /// Is this the one in use now?
    pub active: bool,
}

/// The installed sound themes, for the picker.
///
/// Replaces a hardcoded list of "Chime" and "Soft" - names of themes that exist
/// on no machine, so choosing one wrote a `[sound] theme` the resolver could
/// never find and every cue fell through to the synth while the page showed a
/// confident selection.
#[tauri::command]
pub fn sound_themes() -> Result<Vec<SoundThemeOption>, String> {
    let active = arlen_notification_daemon::config::load_config(
        &arlen_notification_daemon::config::default_config_path(),
    )
    .sound
    .theme;
    Ok(arlen_notification_daemon::sound::installed_themes(
        &arlen_notification_daemon::sound::default_sound_roots(),
    )
    .into_iter()
    .map(|t| SoundThemeOption { active: t.id == active, id: t.id, name: t.name })
    .collect())
}

/// The cue names the active theme provides, for the per-event picker.
///
/// Replaces a hardcoded "Bell / Pop / Click" that no theme ships: choosing one
/// wrote a mapping that resolved to nothing, so the event fell silent while the
/// row showed a confident selection.
#[tauri::command]
pub fn sound_cues() -> Result<Vec<String>, String> {
    let theme = arlen_notification_daemon::config::load_config(
        &arlen_notification_daemon::config::default_config_path(),
    )
    .sound
    .theme;
    Ok(arlen_notification_daemon::sound::theme_cue_names(
        &arlen_notification_daemon::sound::default_sound_roots(),
        &theme,
    ))
}

/// Play one cue, exactly as the Notification Daemon would resolve it
/// (`sound-system-plan.md` SO-R3, the picker's play-preview).
///
/// REUSES the daemon's own `resolve_sound` and `SystemSoundPlayer` rather than
/// resolving here. A preview that walked its own theme chain could audition a
/// file the system will never play - a different theme root, a different
/// extension order, a `.disabled` marker it did not honour - and the whole point
/// of the button is to hear what will actually happen.
///
/// NOT gated by do-not-disturb, mute or Focus, and that is deliberate: those gate
/// cues the system RAISES at you. This is a cue you asked for, in the surface
/// where you are choosing it, and a preview button that silently did nothing
/// because DND was on would read as a broken theme.
///
/// Returns what happened rather than unit, so the page can say "this event is
/// silenced" or "the theme has no file for it" instead of leaving a button that
/// looks like it failed.
/// ASYNC, and that is load-bearing rather than stylistic. `SystemSoundPlayer::play`
/// reaps its child through `tokio::task::spawn_blocking`, whose own comment says
/// the assumption out loud: "`play` is always called from the tokio dispatch
/// path, so a runtime is present". A synchronous `#[tauri::command]` runs off
/// that runtime, so `spawn_blocking` panicked and took the whole Settings app
/// down - the WebDriver session died mid-click with "Session terminated without
/// a reply" the first time this button was pressed on a theme that HAD a file to
/// play. An async command runs on Tauri's tokio runtime, which satisfies it.
#[tauri::command]
pub async fn sound_preview(name: String) -> Result<String, String> {
    use arlen_notification_daemon::sound::{
        default_sound_roots, resolve_sound, SoundPlayer, SoundResolution, SystemSoundPlayer,
    };

    // The active sound theme is the notification daemon's own `[sound] theme`,
    // NOT the appearance theme: `SoundTokens` carries per-event NAMES and no
    // theme, and the daemon resolves those names inside whichever theme its
    // config names. Reading the appearance theme here would preview a theme
    // nothing plays from.
    let config = arlen_notification_daemon::config::load_config(
        &arlen_notification_daemon::config::default_config_path(),
    );
    let theme = config.sound.theme.clone();
    let resolution = resolve_sound(&default_sound_roots(), &theme, &name);
    match &resolution {
        SoundResolution::File(_) => match SystemSoundPlayer::discover() {
            Some(player) => {
                player.play(&resolution, 1.0);
                Ok("played".into())
            }
            // A machine with no play command is a real state, not an error: say so
            // rather than reporting a sound that never left the speaker.
            None => Ok("no-audio-tool".into()),
        },
        SoundResolution::Silenced => Ok("silenced".into()),
        SoundResolution::NotFound => Ok("not-found".into()),
    }
}
