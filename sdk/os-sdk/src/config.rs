/// TOML-based configuration system for Lunaris applications.
///
/// Each application has its own config file at `~/.config/lunaris/<app_id>.toml`.
/// The system-wide theme lives at `~/.config/lunaris/theme.toml`.
///
/// # Example
///
/// ```text
/// let config = Config::load("shell").unwrap();
/// let position: String = config.get("panel.position").unwrap_or("bottom".to_string());
/// ```

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use toml::Value;

/// A loaded configuration file.
///
/// Values are accessed by dot-separated key paths, e.g. `"panel.position"`.
/// Missing keys return `None`; type mismatches return `None`.
/// The underlying TOML is immutable after load; use `Config::reload` for updates.
pub struct Config {
    path: PathBuf,
    root: Value,
}

impl Config {
    /// Load a config file for `app_id`.
    ///
    /// Looks for `~/.config/lunaris/<app_id>.toml`. If the file does not exist,
    /// returns an empty config (all `get` calls return `None`).
    pub fn load(app_id: &str) -> Result<Self, ConfigError> {
        let path = config_path(app_id);
        Self::load_path(&path)
    }

    /// Load a config file from an explicit path.
    pub fn load_path(path: &Path) -> Result<Self, ConfigError> {
        let root = if path.exists() {
            let contents = std::fs::read_to_string(path)
                .map_err(|e| ConfigError::Io(e))?;
            contents.parse::<Value>()
                .map_err(|e| ConfigError::Parse(e.to_string()))?
        } else {
            Value::Table(toml::map::Map::new())
        };

        Ok(Self {
            path: path.to_path_buf(),
            root,
        })
    }

    /// Reload the config from disk.
    ///
    /// Call this when the file watcher signals a change.
    pub fn reload(&mut self) -> Result<(), ConfigError> {
        let fresh = Self::load_path(&self.path)?;
        self.root = fresh.root;
        Ok(())
    }

    /// Get a value by dot-separated key path.
    ///
    /// Returns `None` if the key does not exist or the value cannot be
    /// converted to type `T`.
    ///
    /// # Supported types
    ///
    /// `String`, `i64`, `f64`, `bool`
    pub fn get<T: FromToml>(&self, key: &str) -> Option<T> {
        let value = self.traverse(key)?;
        T::from_toml(value)
    }

    /// Get a raw TOML value by dot-separated key path.
    pub fn get_raw(&self, key: &str) -> Option<&Value> {
        self.traverse(key)
    }

    /// The path this config was loaded from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Watch this config file for changes.
    ///
    /// Returns a `Receiver` that receives `()` whenever the file changes.
    /// The receiver is disconnected when the watcher is dropped.
    ///
    /// The caller is responsible for calling `Config::reload` after receiving
    /// a change notification.
    pub fn watch(&self) -> Result<ConfigWatcher, ConfigError> {
        let (tx, rx) = mpsc::channel::<()>();
        let path = self.path.clone();

        let mut watcher = notify::recommended_watcher(move |event: Result<Event, _>| {
            if let Ok(event) = event {
                match event.kind {
                    EventKind::Modify(_) | EventKind::Create(_) => {
                        let _ = tx.send(());
                    }
                    _ => {}
                }
            }
        })
        .map_err(|e| ConfigError::Watch(e.to_string()))?;

        // Watch the parent directory because editors often replace files
        // atomically (write to temp, rename) which would miss a direct
        // file watch.
        let parent = path.parent().unwrap_or(Path::new("."));
        watcher
            .watch(parent, RecursiveMode::NonRecursive)
            .map_err(|e| ConfigError::Watch(e.to_string()))?;

        Ok(ConfigWatcher {
            _watcher: watcher,
            rx,
            path,
        })
    }

    /// Traverse the TOML tree by dot-separated key path.
    fn traverse(&self, key: &str) -> Option<&Value> {
        let mut current = &self.root;
        for part in key.split('.') {
            current = current.get(part)?;
        }
        Some(current)
    }
}

/// A file watcher for a config file.
///
/// Receives `()` on the inner channel whenever the watched file changes.
/// Drop to stop watching.
pub struct ConfigWatcher {
    _watcher: RecommendedWatcher,
    rx: Receiver<()>,
    path: PathBuf,
}

impl ConfigWatcher {
    /// Block until the config file changes, then return.
    ///
    /// Returns `Err` if the watcher has been dropped.
    pub fn recv(&self) -> Result<(), mpsc::RecvError> {
        self.rx.recv()
    }

    /// Non-blocking check for a change.
    pub fn try_recv(&self) -> Result<(), mpsc::TryRecvError> {
        self.rx.try_recv()
    }

    /// The path being watched.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Resolve the config path for an app ID.
///
/// Returns `~/.config/lunaris/<app_id>.toml`.
/// Falls back to `/etc/lunaris/<app_id>.toml` if the home directory
/// cannot be determined.
pub fn config_path(app_id: &str) -> PathBuf {
    let base = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/etc"))
        .join("lunaris");
    base.join(format!("{app_id}.toml"))
}

/// Errors that can occur when loading or watching a config file.
#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Parse(String),
    Watch(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "config I/O error: {e}"),
            Self::Parse(e) => write!(f, "config parse error: {e}"),
            Self::Watch(e) => write!(f, "config watch error: {e}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Trait for types that can be extracted from a TOML value.
pub trait FromToml: Sized {
    fn from_toml(value: &Value) -> Option<Self>;
}

impl FromToml for String {
    fn from_toml(value: &Value) -> Option<Self> {
        value.as_str().map(|s| s.to_string())
    }
}

impl FromToml for i64 {
    fn from_toml(value: &Value) -> Option<Self> {
        value.as_integer()
    }
}

impl FromToml for f64 {
    fn from_toml(value: &Value) -> Option<Self> {
        value.as_float()
    }
}

impl FromToml for bool {
    fn from_toml(value: &Value) -> Option<Self> {
        value.as_bool()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn config_from_str(toml: &str) -> Config {
        let mut file = NamedTempFile::new().unwrap();
        write!(file, "{toml}").unwrap();
        Config::load_path(file.path()).unwrap()
    }

    #[test]
    fn get_string_value() {
        let config = config_from_str(r#"
[panel]
position = "bottom"
"#);
        assert_eq!(config.get::<String>("panel.position"), Some("bottom".to_string()));
    }

    #[test]
    fn get_bool_value() {
        let config = config_from_str("autohide = true");
        assert_eq!(config.get::<bool>("autohide"), Some(true));
    }

    #[test]
    fn get_integer_value() {
        let config = config_from_str("height = 48");
        assert_eq!(config.get::<i64>("height"), Some(48));
    }

    #[test]
    fn get_missing_key_returns_none() {
        let config = config_from_str("[panel]\nposition = \"bottom\"");
        assert_eq!(config.get::<String>("panel.nonexistent"), None);
        assert_eq!(config.get::<String>("completely.missing"), None);
    }

    #[test]
    fn get_nested_three_levels() {
        let config = config_from_str(r##"
[color.bg]
shell = "#1a1a2e"
"##);
        assert_eq!(
            config.get::<String>("color.bg.shell"),
            Some("#1a1a2e".to_string())
        );
    }

    #[test]
    fn empty_file_returns_empty_config() {
        let config = config_from_str("");
        assert_eq!(config.get::<String>("anything"), None);
    }

    #[test]
    fn missing_file_returns_empty_config() {
        let config = Config::load_path(Path::new("/nonexistent/path/config.toml")).unwrap();
        assert_eq!(config.get::<String>("anything"), None);
    }

    #[test]
    fn reload_picks_up_changes() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        write!(file, "value = \"first\"").unwrap();

        let mut config = Config::load_path(file.path()).unwrap();
        assert_eq!(config.get::<String>("value"), Some("first".to_string()));

        // Overwrite the file
        file.as_file_mut().set_len(0).unwrap();
        use std::io::Seek;
        file.seek(std::io::SeekFrom::Start(0)).unwrap();
        write!(file, "value = \"second\"").unwrap();

        config.reload().unwrap();
        assert_eq!(config.get::<String>("value"), Some("second".to_string()));
    }

    #[test]
    fn watch_fires_on_file_change() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        write!(file, "value = \"initial\"").unwrap();

        let config = Config::load_path(file.path()).unwrap();
        let watcher = config.watch().unwrap();

        // Modify the file in a thread
        let path = file.path().to_path_buf();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            std::fs::write(&path, "value = \"changed\"").unwrap();
        });

        // Should receive a change notification within 2 seconds
        let result = watcher.rx.recv_timeout(std::time::Duration::from_secs(2));
        assert!(result.is_ok(), "expected change notification");
    }
}
