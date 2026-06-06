/// Typed structs shared between Rust and TypeScript via the Tauri IPC bridge.
///
/// All types here derive `ts_rs::TS` which generates TypeScript type definitions
/// in `os-sdk/bindings/` when `cargo test` runs. The generated files are
/// committed to the repo and consumed by ui-kit and desktop-shell.
///
/// If you add or change a type here, run `cargo test -p os-sdk` to regenerate
/// the bindings, then commit the updated `.ts` files.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A single event emitted over the Lunaris Event Bus.
///
/// Sent from Rust to TypeScript when the shell needs to react to system events.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../bindings/")]
pub struct ShellEvent {
    /// UUID v7.
    pub id: String,
    /// Event type in `category.action` format, e.g. `window.focused`.
    pub event_type: String,
    /// Unix timestamp in microseconds.
    pub timestamp: i64,
    /// Emitting component, e.g. `compositor` or `ebpf`.
    pub source: String,
    /// Session the event belongs to.
    pub session_id: String,
}

/// A single row returned by a Knowledge Graph query.
///
/// Fields are serialized as a JSON string to avoid serde_json::Value
/// not implementing ts_rs::TS. The TypeScript side parses this with JSON.parse().
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../bindings/")]
pub struct GraphRow {
    /// JSON-encoded map of field name to value.
    pub fields_json: String,
}

impl GraphRow {
    /// Create a GraphRow from a HashMap.
    pub fn from_fields(fields: std::collections::HashMap<String, serde_json::Value>) -> Self {
        Self {
            fields_json: serde_json::to_string(&fields).unwrap_or_else(|_| "{}".to_string()),
        }
    }
}

/// The result of a Knowledge Graph query.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../bindings/")]
pub struct GraphQueryResult {
    pub rows: Vec<GraphRow>,
}

/// A Lunaris config value passed from Rust to TypeScript.
///
/// The TypeScript shell reads config via Tauri commands; this is the
/// response type.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../bindings/")]
#[serde(tag = "kind", content = "value")]
pub enum ConfigValue {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Missing,
}

/// Surface token colors loaded from `theme.toml`.
///
/// Passed to the WebView at startup so the shell can set CSS custom properties.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../bindings/")]
pub struct SurfaceTokens {
    /// Shell background (panel, launcher). Usually dark in Panda theme.
    pub bg_shell: String,
    /// App window background. Usually light in Panda theme.
    pub bg_app: String,
    /// Card/panel backgrounds inside apps.
    pub bg_card: String,
    /// Overlay backgrounds (modals, popovers).
    pub bg_overlay: String,
    /// Input field backgrounds.
    pub bg_input: String,
    /// Primary text color on shell surfaces.
    pub fg_shell: String,
    /// Primary text color on app surfaces.
    pub fg_app: String,
    /// Accent color for interactive elements and focus rings.
    pub accent: String,
    /// Border color for separators and input outlines.
    pub border: String,
}

impl SurfaceTokens {
    /// Returns the built-in Panda theme tokens.
    ///
    /// Panda: dark shell, light apps. The default Lunaris theme.
    pub fn panda() -> Self {
        Self {
            bg_shell:  "#1a1a2e".to_string(),
            bg_app:    "#ffffff".to_string(),
            bg_card:   "#f5f5f7".to_string(),
            bg_overlay:"#00000080".to_string(),
            bg_input:  "#f0f0f0".to_string(),
            fg_shell:  "#e8e8f0".to_string(),
            fg_app:    "#1a1a2e".to_string(),
            accent:    "#7c6af7".to_string(),
            border:    "#e2e2e8".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_typescript_bindings() {
        // ts_rs exports bindings when this test runs.
        // The generated files appear in os-sdk/bindings/.
        // This test fails if the bindings directory cannot be written.
        ShellEvent::export_all().unwrap();
        GraphRow::export_all().unwrap();
        GraphQueryResult::export_all().unwrap();
        ConfigValue::export_all().unwrap();
        SurfaceTokens::export_all().unwrap();
    }

    #[test]
    fn panda_tokens_have_valid_hex_colors() {
        let tokens = SurfaceTokens::panda();
        for color in [
            &tokens.bg_shell, &tokens.bg_app, &tokens.bg_card,
            &tokens.fg_shell, &tokens.fg_app, &tokens.accent, &tokens.border,
        ] {
            assert!(color.starts_with('#'), "expected hex color, got: {color}");
            assert!(color.len() >= 7, "expected at least #rrggbb, got: {color}");
        }
    }

    #[test]
    fn config_value_serializes_correctly() {
        let val = ConfigValue::String("bottom".to_string());
        let json = serde_json::to_string(&val).unwrap();
        assert!(json.contains("\"kind\":\"String\""));
        assert!(json.contains("\"value\":\"bottom\""));

        let val = ConfigValue::Missing;
        let json = serde_json::to_string(&val).unwrap();
        assert!(json.contains("\"kind\":\"Missing\""));
    }

    #[test]
    fn graph_row_round_trips() {
        let mut fields = std::collections::HashMap::new();
        fields.insert("path".to_string(), serde_json::Value::String("/etc/hostname".to_string()));
        fields.insert("count".to_string(), serde_json::Value::Number(42.into()));

        let row = GraphRow::from_fields(fields);
        assert!(row.fields_json.contains("/etc/hostname"));
        assert!(row.fields_json.contains("42"));

        let json = serde_json::to_string(&row).unwrap();
        let decoded: GraphRow = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.fields_json, row.fields_json);
    }
}
