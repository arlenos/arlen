/// TOML configuration loader for Lunaris OS.
///
/// Loads component configs from two locations:
/// 1. System defaults: `/usr/share/lunaris/defaults/{component}.toml`
/// 2. User overrides: `~/.config/lunaris/{component}.toml`
///
/// User values override system defaults via deep merge on nested tables.
///
/// See `docs/architecture/config-system.md`.

pub mod watcher;

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

const SYSTEM_DEFAULTS_DIR: &str = "/usr/share/lunaris/defaults";

/// Resolve the user config directory (`~/.config/lunaris`).
fn user_config_dir() -> Option<PathBuf> {
    // Respect $XDG_CONFIG_HOME if set, otherwise ~/.config.
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .ok()
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".config"))
        })?;
    Some(base.join("lunaris"))
}

/// Resolve the system defaults directory.
fn system_defaults_dir() -> PathBuf {
    std::env::var("LUNARIS_DEFAULTS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(SYSTEM_DEFAULTS_DIR))
}

/// Build the full path for a component config.
fn system_path(component: &str) -> PathBuf {
    system_defaults_dir().join(format!("{component}.toml"))
}

fn user_path(component: &str) -> Option<PathBuf> {
    user_config_dir().map(|d| d.join(format!("{component}.toml")))
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Configuration loading errors.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Neither system defaults nor user config found.
    #[error("config not found for component '{0}' (checked system defaults and user config)")]
    NotFound(String),

    /// TOML parsing failed.
    #[error("parse error in {path}: {message}")]
    ParseError { path: String, message: String },

    /// File could not be read.
    #[error("IO error reading {path}: {source}")]
    IoError {
        path: String,
        source: std::io::Error,
    },
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Load a component config, merging user overrides onto system defaults.
///
/// `T` must implement `DeserializeOwned` (typically via `#[derive(Deserialize)]`).
///
/// Resolution order:
/// 1. Load system defaults from `/usr/share/lunaris/defaults/{component}.toml`
/// 2. Load user config from `~/.config/lunaris/{component}.toml`
/// 3. Deep-merge user values onto system defaults
/// 4. Deserialize merged TOML into `T`
///
/// If only one source exists, it is used directly. If neither exists,
/// returns `ConfigError::NotFound`.
pub fn load<T: DeserializeOwned>(component: &str) -> Result<T, ConfigError> {
    let sys = system_path(component);
    let usr = user_path(component);

    let sys_exists = sys.exists();
    let usr_exists = usr.as_ref().map(|p| p.exists()).unwrap_or(false);

    if !sys_exists && !usr_exists {
        return Err(ConfigError::NotFound(component.into()));
    }

    let merged = match (sys_exists, usr_exists) {
        (true, true) => {
            let sys_table = read_toml_table(&sys)?;
            let usr_table = read_toml_table(usr.as_ref().unwrap())?;
            deep_merge(sys_table, usr_table)
        }
        (true, false) => read_toml_table(&sys)?,
        (false, true) => read_toml_table(usr.as_ref().unwrap())?,
        (false, false) => unreachable!(),
    };

    let value = toml::Value::Table(merged);
    value.try_into().map_err(|e: toml::de::Error| ConfigError::ParseError {
        path: usr
            .as_ref()
            .filter(|p| p.exists())
            .unwrap_or(&sys)
            .display()
            .to_string(),
        message: e.to_string(),
    })
}

/// Load from explicit paths (for testing or custom locations).
pub fn load_from<T: DeserializeOwned>(
    defaults_path: Option<&Path>,
    user_path: Option<&Path>,
) -> Result<T, ConfigError> {
    let def_exists = defaults_path.map(|p| p.exists()).unwrap_or(false);
    let usr_exists = user_path.map(|p| p.exists()).unwrap_or(false);

    if !def_exists && !usr_exists {
        return Err(ConfigError::NotFound("(custom paths)".into()));
    }

    let merged = match (def_exists, usr_exists) {
        (true, true) => {
            let def_table = read_toml_table(defaults_path.unwrap())?;
            let usr_table = read_toml_table(user_path.unwrap())?;
            deep_merge(def_table, usr_table)
        }
        (true, false) => read_toml_table(defaults_path.unwrap())?,
        (false, true) => read_toml_table(user_path.unwrap())?,
        (false, false) => unreachable!(),
    };

    let display = user_path
        .filter(|p| p.exists())
        .or(defaults_path)
        .map(|p| p.display().to_string())
        .unwrap_or_default();

    let value = toml::Value::Table(merged);
    value.try_into().map_err(|e: toml::de::Error| ConfigError::ParseError {
        path: display,
        message: e.to_string(),
    })
}

// ---------------------------------------------------------------------------
// TOML deep merge
// ---------------------------------------------------------------------------

/// Read a TOML file into a Table.
fn read_toml_table(path: &Path) -> Result<toml::map::Map<String, toml::Value>, ConfigError> {
    let content = std::fs::read_to_string(path).map_err(|e| ConfigError::IoError {
        path: path.display().to_string(),
        source: e,
    })?;
    let table: toml::Value =
        toml::from_str(&content).map_err(|e| ConfigError::ParseError {
            path: path.display().to_string(),
            message: e.to_string(),
        })?;
    match table {
        toml::Value::Table(t) => Ok(t),
        _ => Err(ConfigError::ParseError {
            path: path.display().to_string(),
            message: "expected TOML table at root".into(),
        }),
    }
}

/// Deep-merge `overrides` onto `base`. For nested tables, recurse.
/// For all other types, the override value wins.
pub fn deep_merge(
    mut base: toml::map::Map<String, toml::Value>,
    overrides: toml::map::Map<String, toml::Value>,
) -> toml::map::Map<String, toml::Value> {
    for (key, override_val) in overrides {
        match (base.get(&key), &override_val) {
            // Both are tables: recurse.
            (Some(toml::Value::Table(base_inner)), toml::Value::Table(over_inner)) => {
                let merged = deep_merge(base_inner.clone(), over_inner.clone());
                base.insert(key, toml::Value::Table(merged));
            }
            // Otherwise: override wins.
            _ => {
                base.insert(key, override_val);
            }
        }
    }
    base
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::io::Write;
    use tempfile::TempDir;

    #[derive(Debug, Deserialize, PartialEq)]
    struct ShellConfig {
        #[serde(default)]
        bar_height: u32,
        #[serde(default)]
        layout_mode: String,
        #[serde(default)]
        night_light: Option<NightLight>,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct NightLight {
        enabled: bool,
        temperature: u32,
    }

    fn write_file(dir: &Path, name: &str, content: &str) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    // ── Only defaults ──

    #[test]
    fn test_only_defaults() {
        let dir = TempDir::new().unwrap();
        write_file(
            dir.path(),
            "shell.toml",
            r#"
bar_height = 36
layout_mode = "float"

[night_light]
enabled = false
temperature = 3400
"#,
        );

        let cfg: ShellConfig =
            load_from(Some(&dir.path().join("shell.toml")), None).unwrap();
        assert_eq!(cfg.bar_height, 36);
        assert_eq!(cfg.layout_mode, "float");
        assert_eq!(
            cfg.night_light,
            Some(NightLight {
                enabled: false,
                temperature: 3400
            })
        );
    }

    // ── User overrides individual fields ──

    #[test]
    fn test_user_overrides_fields() {
        let dir = TempDir::new().unwrap();

        write_file(
            dir.path(),
            "defaults/shell.toml",
            r#"
bar_height = 36
layout_mode = "float"

[night_light]
enabled = false
temperature = 3400
"#,
        );

        write_file(
            dir.path(),
            "user/shell.toml",
            r#"
layout_mode = "tile"

[night_light]
enabled = true
"#,
        );

        let cfg: ShellConfig = load_from(
            Some(&dir.path().join("defaults/shell.toml")),
            Some(&dir.path().join("user/shell.toml")),
        )
        .unwrap();

        // bar_height kept from defaults.
        assert_eq!(cfg.bar_height, 36);
        // layout_mode overridden by user.
        assert_eq!(cfg.layout_mode, "tile");
        // night_light: enabled overridden, temperature kept from defaults.
        assert_eq!(
            cfg.night_light,
            Some(NightLight {
                enabled: true,
                temperature: 3400
            })
        );
    }

    // ── User adds new nested section ──

    #[test]
    fn test_user_adds_section() {
        let dir = TempDir::new().unwrap();

        write_file(
            dir.path(),
            "defaults/shell.toml",
            r#"
bar_height = 36
layout_mode = "float"
"#,
        );

        write_file(
            dir.path(),
            "user/shell.toml",
            r#"
[night_light]
enabled = true
temperature = 2700
"#,
        );

        let cfg: ShellConfig = load_from(
            Some(&dir.path().join("defaults/shell.toml")),
            Some(&dir.path().join("user/shell.toml")),
        )
        .unwrap();

        assert_eq!(cfg.bar_height, 36);
        assert_eq!(
            cfg.night_light,
            Some(NightLight {
                enabled: true,
                temperature: 2700
            })
        );
    }

    // ── Only user config ──

    #[test]
    fn test_only_user_config() {
        let dir = TempDir::new().unwrap();
        write_file(
            dir.path(),
            "shell.toml",
            r#"
bar_height = 40
layout_mode = "tile"
"#,
        );

        let cfg: ShellConfig =
            load_from(None, Some(&dir.path().join("shell.toml"))).unwrap();
        assert_eq!(cfg.bar_height, 40);
        assert_eq!(cfg.layout_mode, "tile");
    }

    // ── Neither file exists ──

    #[test]
    fn test_not_found() {
        let result: Result<ShellConfig, _> = load_from(
            Some(Path::new("/tmp/lunaris-test-missing-xyz/defaults.toml")),
            Some(Path::new("/tmp/lunaris-test-missing-xyz/user.toml")),
        );
        assert!(matches!(result, Err(ConfigError::NotFound(_))));
    }

    // ── Invalid TOML ──

    #[test]
    fn test_invalid_toml() {
        let dir = TempDir::new().unwrap();
        write_file(dir.path(), "bad.toml", "this is not valid toml {{{{");

        let result: Result<ShellConfig, _> =
            load_from(Some(&dir.path().join("bad.toml")), None);
        assert!(matches!(result, Err(ConfigError::ParseError { .. })));

        if let Err(ConfigError::ParseError { path, message }) = result {
            assert!(path.contains("bad.toml"));
            assert!(!message.is_empty());
        }
    }

    // ── Type mismatch ──

    #[test]
    fn test_type_mismatch() {
        let dir = TempDir::new().unwrap();
        write_file(
            dir.path(),
            "shell.toml",
            r#"bar_height = "not a number""#,
        );

        let result: Result<ShellConfig, _> =
            load_from(Some(&dir.path().join("shell.toml")), None);
        assert!(matches!(result, Err(ConfigError::ParseError { .. })));
    }

    // ── Deep merge unit tests ──

    #[test]
    fn test_deep_merge_scalar() {
        let mut base = toml::map::Map::new();
        base.insert("a".into(), toml::Value::Integer(1));
        base.insert("b".into(), toml::Value::Integer(2));

        let mut over = toml::map::Map::new();
        over.insert("b".into(), toml::Value::Integer(99));
        over.insert("c".into(), toml::Value::Integer(3));

        let merged = deep_merge(base, over);
        assert_eq!(merged["a"].as_integer(), Some(1));
        assert_eq!(merged["b"].as_integer(), Some(99));
        assert_eq!(merged["c"].as_integer(), Some(3));
    }

    #[test]
    fn test_deep_merge_nested() {
        let base: toml::Value = toml::from_str(
            r#"
[section]
a = 1
b = 2
"#,
        )
        .unwrap();

        let over: toml::Value = toml::from_str(
            r#"
[section]
b = 99
c = 3
"#,
        )
        .unwrap();

        let merged = deep_merge(
            base.as_table().unwrap().clone(),
            over.as_table().unwrap().clone(),
        );

        let section = merged["section"].as_table().unwrap();
        assert_eq!(section["a"].as_integer(), Some(1));
        assert_eq!(section["b"].as_integer(), Some(99));
        assert_eq!(section["c"].as_integer(), Some(3));
    }

    #[test]
    fn test_deep_merge_override_table_with_scalar() {
        let base: toml::Value = toml::from_str(
            r#"
[section]
a = 1
"#,
        )
        .unwrap();

        let over: toml::Value = toml::from_str(
            r#"
section = "replaced"
"#,
        )
        .unwrap();

        let merged = deep_merge(
            base.as_table().unwrap().clone(),
            over.as_table().unwrap().clone(),
        );

        // Override wins even if types differ.
        assert_eq!(merged["section"].as_str(), Some("replaced"));
    }
}
