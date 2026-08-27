//! Night-light Tauri commands for the Settings panel.
//!
//! Settings runs in its own Tauri process and can't reach
//! desktop-shell's `night_light_*` commands directly. Instead we
//! surgically write the `[night_light]` section of
//! `~/.config/arlen/shell.toml` and let desktop-shell's existing
//! shell-config watcher pick up the change and replay the state to
//! the compositor via the `arlen-shell-overlay` protocol. Same
//! pattern the rest of the cross-app config uses.
//!
//! We deliberately do NOT deserialize the full `ShellConfig`
//! struct on read so app-settings doesn't have to track the
//! desktop-shell schema. We work on `toml::Table` and only touch
//! fields the user changed.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

fn shell_toml_path() -> Result<PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
    Ok(PathBuf::from(home).join(".config/arlen/shell.toml"))
}

/// The existing `shell.toml` as a generic table, or why it could not be read, so
/// app-settings can mutate one field without forcing the rest of the schema.
///
/// ABSENT IS NOT UNREADABLE, and this file is the one where that matters most.
/// `write_shell_table` writes the WHOLE table back, and three commands here are
/// read-modify-write over it - so answering an unreadable or corrupt `shell.toml`
/// with an empty table meant one flick of the night-light switch replaced the
/// person's night light, brightness, layout, focus and launcher settings with a
/// file containing only night light. Every other section, gone, and the call
/// returned Ok. The same damage `compositor.toml` had from the gaps slider, in
/// the file next to it.
fn read_shell_table() -> Result<toml::Table, String> {
    let path = shell_toml_path()?;
    match fs::read_to_string(&path) {
        Ok(text) => toml::from_str(&text)
            .map_err(|e| format!("{} could not be read: {e}", path.display())),
        // No shell.toml yet: the empty table IS the state, and the write creates
        // the file with just this section in it.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(toml::Table::new()),
        Err(e) => Err(format!("{} could not be read: {e}", path.display())),
    }
}

/// Write the table back atomically. Tempfile + rename so a partial
/// write never strands the user with an unparseable shell.toml.
fn write_shell_table(table: &toml::Table) -> Result<(), String> {
    let path = shell_toml_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    let serialized = toml::to_string_pretty(table)
        .map_err(|e| format!("serialize shell.toml: {e}"))?;
    // Per INSTANCE, not per app (`app-instance-model.md`): two windows sharing a
    // temp name cross over rather than tear.
    let tmp = path.with_extension(format!("toml.{}.tmp", std::process::id()));
    fs::write(&tmp, serialized).map_err(|e| format!("write tmp: {e}"))?;
    fs::rename(&tmp, &path).map_err(|e| format!("rename: {e}"))?;
    Ok(())
}

fn night_light_section_mut(table: &mut toml::Table) -> &mut toml::Table {
    if !table.contains_key("night_light") {
        table.insert("night_light".to_string(), toml::Value::Table(toml::Table::new()));
    }
    table
        .get_mut("night_light")
        .and_then(|v| v.as_table_mut())
        .expect("just inserted as table")
}

/// Read the night_light section. Returns sensible defaults when the
/// file or section is missing — same defaults desktop-shell uses.
#[tauri::command]
pub fn night_light_get_state() -> NightLightState {
    // Read-only: nothing is written back, so an unreadable file renders as
    // the defaults rather than refusing to show the page.
    let table = read_shell_table().unwrap_or_default();
    let nl = table
        .get("night_light")
        .and_then(|v| v.as_table())
        .cloned()
        .unwrap_or_default();
    NightLightState {
        enabled: nl
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        temperature: nl
            .get("temperature")
            .and_then(|v| v.as_integer())
            .map(|n| n as u16)
            .unwrap_or(3400),
        schedule: nl
            .get("schedule")
            .and_then(|v| v.as_str())
            .unwrap_or("manual")
            .to_string(),
        custom_start: nl
            .get("custom_start")
            .and_then(|v| v.as_integer())
            .map(|n| n as u32)
            .unwrap_or(22 * 60),
        custom_end: nl
            .get("custom_end")
            .and_then(|v| v.as_integer())
            .map(|n| n as u32)
            .unwrap_or(7 * 60),
        latitude: nl
            .get("latitude")
            .and_then(|v| v.as_float())
            .unwrap_or(0.0),
        longitude: nl
            .get("longitude")
            .and_then(|v| v.as_float())
            .unwrap_or(0.0),
    }
}

/// Toggle on/off and set the target temperature. Mirrors the
/// desktop-shell `night_light_set` semantics — we just write to disk
/// and let the watcher relay to the compositor.
#[tauri::command]
pub fn night_light_set(enabled: bool, temperature: u16) -> Result<(), String> {
    let mut table = read_shell_table()?;
    {
        let nl = night_light_section_mut(&mut table);
        nl.insert("enabled".into(), toml::Value::Boolean(enabled));
        nl.insert(
            "temperature".into(),
            toml::Value::Integer(temperature as i64),
        );
    }
    write_shell_table(&table)
}

/// Set schedule mode + custom-window times. `schedule` is one of
/// `manual | sunset_sunrise | custom` to match the TOML enum
/// rename.
#[tauri::command]
pub fn night_light_set_schedule(
    schedule: String,
    custom_start: u32,
    custom_end: u32,
) -> Result<(), String> {
    if !matches!(schedule.as_str(), "manual" | "sunset_sunrise" | "custom") {
        return Err(format!("unknown schedule '{schedule}'"));
    }
    let mut table = read_shell_table()?;
    {
        let nl = night_light_section_mut(&mut table);
        nl.insert("schedule".into(), toml::Value::String(schedule));
        nl.insert(
            "custom_start".into(),
            toml::Value::Integer(custom_start as i64),
        );
        nl.insert(
            "custom_end".into(),
            toml::Value::Integer(custom_end as i64),
        );
    }
    write_shell_table(&table)
}

/// Set the user's geographic location. Used for the
/// sunset/sunrise schedule mode.
#[tauri::command]
pub fn night_light_set_location(latitude: f64, longitude: f64) -> Result<(), String> {
    let mut table = read_shell_table()?;
    {
        let nl = night_light_section_mut(&mut table);
        nl.insert("latitude".into(), toml::Value::Float(latitude));
        nl.insert("longitude".into(), toml::Value::Float(longitude));
    }
    write_shell_table(&table)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct NightLightState {
    pub enabled: bool,
    pub temperature: u16,
    pub schedule: String,
    pub custom_start: u32,
    pub custom_end: u32,
    pub latitude: f64,
    pub longitude: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn night_light_section_mut_creates_when_missing() {
        let mut table = toml::Table::new();
        let nl = night_light_section_mut(&mut table);
        nl.insert("enabled".into(), toml::Value::Boolean(true));
        assert_eq!(
            table
                .get("night_light")
                .and_then(|v| v.as_table())
                .and_then(|t| t.get("enabled"))
                .and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn night_light_section_mut_preserves_other_keys() {
        let mut table = toml::Table::new();
        table.insert(
            "display".into(),
            toml::Value::Table({
                let mut t = toml::Table::new();
                t.insert("brightness".into(), toml::Value::Float(0.5));
                t
            }),
        );
        let nl = night_light_section_mut(&mut table);
        nl.insert("enabled".into(), toml::Value::Boolean(true));
        // Display section must still be there and untouched.
        let display = table.get("display").and_then(|v| v.as_table()).unwrap();
        assert_eq!(display.get("brightness").and_then(|v| v.as_float()), Some(0.5));
    }
}
