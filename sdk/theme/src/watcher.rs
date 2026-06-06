//! File watcher for live theme updates.
//!
//! The watcher signals "something changed, re-resolve". Re-resolution
//! (loading the bundled bytes + reading user-theme + customization
//! files + computing the merged `ArlenTheme`) is the caller's
//! responsibility — that's where the bundled bytes live (per-crate
//! `include_str!`) and where the `appearance.toml [theme].active`
//! lookup happens.
//!
//! What the watcher monitors:
//!
//! - `~/.config/arlen/theme.toml` (user customization overlay)
//! - `~/.config/arlen/appearance.toml` (active theme, intensity)
//!
//! On any change to either file, the callback fires. Atomic editor
//! renames are handled by watching the parent directory and
//! filtering on filename.

use crate::ArlenTheme;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

/// Watches the user theme files and notifies on changes.
pub struct ThemeWatcher {
    _watcher: RecommendedWatcher,
}

impl ThemeWatcher {
    /// Start watching the standard theme paths and call `on_change`
    /// (with no argument) whenever any of them change. The callback
    /// re-loads + re-resolves and applies the new theme.
    pub fn start<F>(on_change: F) -> Result<Self, notify::Error>
    where
        F: Fn() + Send + 'static,
    {
        let custom = ArlenTheme::user_customization_path();
        let appearance = appearance_path();
        Self::start_at(vec![custom, appearance], on_change)
    }

    /// Watch an explicit list of file paths. Used by tests + for
    /// non-default config locations.
    pub fn start_at<F>(paths: Vec<PathBuf>, on_change: F) -> Result<Self, notify::Error>
    where
        F: Fn() + Send + 'static,
    {
        let interesting: Vec<OsString> = paths
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_os_string()))
            .collect();

        let mut watcher = notify::recommended_watcher(move |res: Result<notify::Event, _>| {
            if let Ok(event) = res {
                use notify::EventKind;
                let dominated = matches!(
                    event.kind,
                    EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                );
                let relevant = event.paths.iter().any(|p| {
                    p.file_name()
                        .map(|n| interesting.iter().any(|w| w == n))
                        .unwrap_or(false)
                });
                if dominated && relevant {
                    on_change();
                }
            }
        })?;

        // Watch each parent directory to catch atomic editor renames.
        // Multiple paths may share a parent (`~/.config/arlen/`),
        // dedup so we don't double-watch.
        let mut watched_parents: Vec<&Path> = Vec::new();
        for p in paths.iter() {
            if let Some(parent) = p.parent() {
                if !watched_parents.iter().any(|w| *w == parent) && parent.exists() {
                    watcher.watch(parent, RecursiveMode::NonRecursive)?;
                    watched_parents.push(parent);
                }
            }
        }

        Ok(ThemeWatcher { _watcher: watcher })
    }

    /// Channel-based variant: returns a receiver that ticks once
    /// per change. Caller still re-resolves and applies the theme
    /// — the watcher does not carry state.
    pub fn channel() -> Result<(Self, mpsc::Receiver<()>), notify::Error> {
        let (tx, rx) = mpsc::channel();
        let watcher = Self::start(move || {
            let _ = tx.send(());
        })?;
        Ok((watcher, rx))
    }
}

fn appearance_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("arlen")
        .join("appearance.toml")
}
