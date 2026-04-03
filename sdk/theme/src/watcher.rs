//! File watcher for live theme updates.

use crate::LunarisTheme;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::mpsc;

/// Watches `~/.config/lunaris/theme.toml` and calls a callback with the
/// newly loaded `LunarisTheme` whenever the file changes.
///
/// The watcher monitors the parent directory (to catch atomic editor renames)
/// and filters for changes to `theme.toml` specifically.
pub struct ThemeWatcher {
    _watcher: RecommendedWatcher,
}

impl ThemeWatcher {
    /// Start watching and call `on_change` with the new theme on every update.
    ///
    /// The callback runs on a background thread. The returned `ThemeWatcher`
    /// must be kept alive; dropping it stops the watcher.
    pub fn start<F>(on_change: F) -> Result<Self, notify::Error>
    where
        F: Fn(LunarisTheme) + Send + 'static,
    {
        let theme_path = LunarisTheme::default_path();
        Self::start_at(theme_path, on_change)
    }

    /// Start watching a specific path.
    pub fn start_at<F>(theme_path: PathBuf, on_change: F) -> Result<Self, notify::Error>
    where
        F: Fn(LunarisTheme) + Send + 'static,
    {
        let path_for_load = theme_path.clone();
        let file_name = theme_path
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_default();

        let mut watcher = notify::recommended_watcher(move |res: Result<notify::Event, _>| {
            if let Ok(event) = res {
                use notify::EventKind;
                let dominated = matches!(
                    event.kind,
                    EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                );
                let relevant = event
                    .paths
                    .iter()
                    .any(|p| p.file_name().map(|n| n == file_name).unwrap_or(false));

                if dominated && relevant {
                    let theme = LunarisTheme::load_from(&path_for_load);
                    on_change(theme);
                }
            }
        })?;

        // Watch parent directory to catch atomic renames.
        if let Some(parent) = theme_path.parent() {
            if parent.exists() {
                watcher.watch(parent, RecursiveMode::NonRecursive)?;
            }
        }

        Ok(ThemeWatcher { _watcher: watcher })
    }

    /// Create a channel-based watcher that sends new themes to a receiver.
    pub fn channel() -> Result<(Self, mpsc::Receiver<LunarisTheme>), notify::Error> {
        let (tx, rx) = mpsc::channel();
        let watcher = Self::start(move |theme| {
            let _ = tx.send(theme);
        })?;
        Ok((watcher, rx))
    }
}
