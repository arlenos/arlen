/// File watcher for config hot-reload.
///
/// Watches the parent directory of the config file (editors do atomic
/// rename, not in-place write). On change, re-loads the config and
/// sends the new value through a broadcast channel. The theme-emitted
/// `sounds.toml` lives in the same directory and feeds the same
/// [`load_config`] (it merges the theme's per-event cue names), so a theme
/// switch that rewrites it also triggers a reload.

use std::path::{Path, PathBuf};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::broadcast;

use super::{load_config, theme_sounds_path, Config};

/// The file name the theme's sound map is emitted under (`sdk/theme`'s apply
/// writes `arlen/sounds.toml`), sharing the config directory we already watch.
fn sounds_file_name() -> String {
    theme_sounds_path()
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

/// Whether a changed file name warrants a reload: the daemon's own config, or
/// the theme-emitted sound map beside it (both feed [`load_config`]).
fn triggers_reload(changed: &str, config_name: &str, sounds_name: &str) -> bool {
    changed == config_name || changed == sounds_name
}

/// Start watching a config file for changes.
///
/// Returns a broadcast receiver that emits the new `Config` whenever
/// the file changes. The watcher runs in a background thread.
pub fn watch_config(
    path: PathBuf,
) -> Result<(broadcast::Receiver<Config>, RecommendedWatcher), notify::Error> {
    let (tx, rx) = broadcast::channel::<Config>(8);
    let file_name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let watch_path = path.clone();
    let sounds_name = sounds_file_name();

    let mut watcher = notify::recommended_watcher(move |res: Result<Event, _>| {
        let Ok(event) = res else { return };
        match event.kind {
            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
                // React to our config file or the theme-emitted sound map beside it.
                let is_our_file = event.paths.iter().any(|p| {
                    p.file_name()
                        .map(|n| triggers_reload(&n.to_string_lossy(), &file_name, &sounds_name))
                        .unwrap_or(false)
                });
                if is_our_file {
                    let new_config = load_config(&watch_path);
                    let _ = tx.send(new_config);
                    tracing::info!("config reloaded");
                }
            }
            _ => {}
        }
    })?;

    // Watch parent directory (editors rename, not modify).
    let parent = path.parent().unwrap_or(Path::new("."));
    if !parent.exists() {
        let _ = std::fs::create_dir_all(parent);
    }
    watcher.watch(parent, RecursiveMode::NonRecursive)?;

    tracing::info!("watching config at {}", path.display());

    Ok((rx, watcher))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reload_triggers_on_the_config_or_the_theme_sound_map() {
        assert!(triggers_reload("notifications.toml", "notifications.toml", "sounds.toml"));
        assert!(triggers_reload("sounds.toml", "notifications.toml", "sounds.toml"));
        assert!(!triggers_reload("appearance.toml", "notifications.toml", "sounds.toml"));
        assert!(!triggers_reload("notifications.toml.bak", "notifications.toml", "sounds.toml"));
    }

    #[test]
    fn the_theme_sound_map_is_sounds_toml() {
        assert_eq!(sounds_file_name(), "sounds.toml");
    }
}
