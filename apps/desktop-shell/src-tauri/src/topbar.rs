//! Top-bar arrangement inventory (topbar-applets-plan.md).
//!
//! `topbar_items` gives the Settings Topbar-arrangement panel the live inventory:
//! the first-party right-cluster applets plus the live SNI tray items, each merged
//! with the saved order and per-item visibility from `~/.config/arlen/topbar.toml`
//! (the file the panel writes). A tray item not yet in the config defaults to the
//! overflow (`shown = false`), so a newly-appeared tray icon never silently claims
//! bar space. This is the inventory half (seam 1); the shell rendering from the
//! same config is the separate, metal-verified seam.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::sni::SniItems;

/// One arrangeable top-bar item as the panel lists it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopbarItem {
    /// Stable id: an applet key, or `tray:<sni-id>` for a tray item.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Icon name (a lucide name for applets, a freedesktop icon name for tray).
    pub icon: String,
    /// `"applet"` (first-party) or `"tray"` (a StatusNotifierItem).
    pub kind: String,
    /// Whether it is shown on the bar (vs. in the overflow).
    pub shown: bool,
}

/// The first-party right-cluster applets in default order, each with a display
/// name and a lucide icon name. The id is the stable key the arrangement config
/// orders and hides by; it matches the applet keys the shell renderer uses.
const APPLETS: &[(&str, &str, &str)] = &[
    ("notifications", "Notifications", "Bell"),
    // Between notifications and audio because that is where the bar draws it.
    // It was missing entirely, so the arrangement panel could neither show nor
    // move the one indicator that offers to undo what just happened - the list
    // has to match what the bar renders, or it quietly arranges a different
    // desktop than the one on screen.
    ("undo", "Recent actions", "Undo2"),
    ("audio", "Audio", "Volume2"),
    ("network", "Network", "Wifi"),
    ("bluetooth", "Bluetooth", "Bluetooth"),
    ("battery", "Battery", "BatteryMedium"),
    ("layout", "Layout", "LayoutPanelLeft"),
    ("clock", "Clock", "Clock"),
    ("quick-settings", "Quick Settings", "SlidersHorizontal"),
];

/// The saved arrangement from `topbar.toml`: an explicit order (id array) and a
/// per-item visibility map. Both default empty (no saved arrangement yet).
#[derive(Debug, Default, Deserialize)]
struct TopbarConfig {
    #[serde(default)]
    order: Vec<String>,
    #[serde(default)]
    visible: HashMap<String, bool>,
}

/// `~/.config/arlen/topbar.toml`, or `None` if no config dir resolves.
fn topbar_config_path() -> Option<std::path::PathBuf> {
    Some(dirs::config_dir()?.join("arlen").join("topbar.toml"))
}

/// Load the saved arrangement, or the empty default when the file is absent or
/// unparseable (a fresh install, or a hand-broken file: the panel then shows the
/// default arrangement rather than failing).
fn read_topbar_config() -> TopbarConfig {
    topbar_config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or_default()
}

/// Merge the applet list, the live tray, and the saved config into the ordered
/// inventory. Pure over its inputs, so the visibility defaults and the ordering
/// are unit-tested without the SNI state or the filesystem. `tray` is
/// `(sni_id, title, icon_name)` per live item.
fn assemble(tray: Vec<(String, String, String)>, config: &TopbarConfig) -> Vec<TopbarItem> {
    let mut items: Vec<TopbarItem> = APPLETS
        .iter()
        .map(|(id, name, icon)| TopbarItem {
            id: (*id).to_string(),
            name: (*name).to_string(),
            icon: (*icon).to_string(),
            kind: "applet".to_string(),
            // A first-party applet is shown unless the config hides it.
            shown: config.visible.get(*id).copied().unwrap_or(true),
        })
        .collect();

    for (sni_id, title, icon_name) in tray {
        let id = format!("tray:{sni_id}");
        // A tray item not yet in the config defaults to the overflow.
        let shown = config.visible.get(&id).copied().unwrap_or(false);
        items.push(TopbarItem {
            id,
            name: title,
            icon: icon_name,
            kind: "tray".to_string(),
            shown,
        });
    }

    // Apply the saved order: configured ids first in their saved order, anything
    // not listed (a new applet or tray item) keeps its append order after them.
    if !config.order.is_empty() {
        items.sort_by_key(|it| {
            config
                .order
                .iter()
                .position(|o| o == &it.id)
                .unwrap_or(usize::MAX)
        });
    }
    items
}

/// The top-bar arrangement inventory for the Settings panel: first-party applets
/// plus the live SNI tray, merged with the saved order and visibility.
#[tauri::command]
pub fn topbar_items(sni: tauri::State<'_, SniItems>) -> Result<Vec<TopbarItem>, String> {
    let tray: Vec<(String, String, String)> = sni
        .lock()
        .map_err(|_| "sni state poisoned".to_string())?
        .values()
        .filter(|item| item.status != "Passive")
        .map(|item| (item.id.clone(), item.title.clone(), item.icon_name.clone()))
        .collect();
    Ok(assemble(tray, &read_topbar_config()))
}

// ---------------------------------------------------------------------------
// Live-reload watcher
// ---------------------------------------------------------------------------

/// The event the bar listens for to re-read the arrangement.
pub const ARRANGEMENT_CHANGED: &str = "arlen://topbar-arrangement-changed";

/// Does a filesystem event concern the arrangement file?
///
/// The watch is on the whole config directory, so `appearance.toml`,
/// `shell.toml` and every other neighbour arrive here too and must be dropped:
/// a theme save that bounced the bar would be a new bug traded for the old one.
/// The atomic-write dance shows up as events on the temp name and on the target,
/// so the filename is matched as well as the full path.
fn touches_arrangement(paths: &[std::path::PathBuf], target: &std::path::Path) -> bool {
    paths
        .iter()
        .any(|p| p == target || p.file_name().map(|n| n == "topbar.toml").unwrap_or(false))
}

/// Watch `~/.config/arlen/topbar.toml` and tell the bar when it changes.
///
/// Without this the bar reads the arrangement once, at startup: reordering an
/// applet in Settings wrote the file, the panel showed the new order, and the
/// bar kept the old one until the next login with nothing on screen saying so.
/// A control that persists but does not take effect is worse than one that
/// refuses, because it looks like it worked.
///
/// The event carries no payload. The bar re-invokes `topbar_items`, which
/// merges the file with the live SNI tray - a payload built here would be the
/// file half only, and the tray half would go stale on exactly the reload meant
/// to freshen it.
///
/// Editors and the Settings writer both write atomically (tmp + rename), so the
/// watch is on the parent directory with a filename filter; watching the file
/// itself stops firing after the first rename replaces the inode. The debounce
/// collapses the create/modify/rename burst of one save into one reload.
///
/// NOT covered: a change made while the shell is not running. That needs no
/// event - the bar reads the file at startup anyway.
pub fn start_topbar_watcher(app: AppHandle) {
    let Some(target) = topbar_config_path() else {
        log::warn!("topbar: no config dir to watch for arrangement changes");
        return;
    };
    let Some(watch_dir) = target.parent().map(std::path::Path::to_path_buf) else {
        log::warn!("topbar: topbar.toml has no parent dir");
        return;
    };
    // The directory need not exist yet on a fresh install; create it so the
    // watch can be established before the first save rather than after it.
    let _ = std::fs::create_dir_all(&watch_dir);

    std::thread::spawn(move || {
        let app_clone = app.clone();
        let last_fire = Mutex::new(Instant::now() - Duration::from_secs(1));

        let mut watcher = match notify::recommended_watcher(move |event: Result<Event, _>| {
            let Ok(event) = event else { return };
            if !matches!(
                event.kind,
                EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
            ) {
                return;
            }
            if !touches_arrangement(&event.paths, &target) {
                return;
            }

            {
                let mut lf = last_fire.lock().unwrap();
                if lf.elapsed() < Duration::from_millis(100) {
                    return;
                }
                *lf = Instant::now();
            }
            // Let the rename settle before the bar reads the file back.
            std::thread::sleep(Duration::from_millis(30));

            let _ = app_clone.emit(ARRANGEMENT_CHANGED, ());
        }) {
            Ok(w) => w,
            Err(e) => {
                log::warn!("topbar: failed to create arrangement watcher: {e}");
                return;
            }
        };

        if let Err(e) = watcher.watch(&watch_dir, RecursiveMode::NonRecursive) {
            log::warn!("topbar: failed to watch {}: {e}", watch_dir.display());
            return;
        }

        // Keep the watcher alive for the life of the shell.
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    });
}

#[cfg(test)]
mod tests {
    /// The inventory is what the arrangement panel offers, so an applet the bar
    /// draws and the list omits is an applet the user cannot arrange - which is
    /// how `undo` came to be invisible to the panel while sitting on the bar.
    #[test]
    fn every_first_party_applet_the_bar_draws_is_listed() {
        use super::APPLETS;
        let ids: Vec<&str> = APPLETS.iter().map(|(id, _, _)| *id).collect();
        for expected in [
            "notifications",
            "undo",
            "audio",
            "network",
            "bluetooth",
            "battery",
            "layout",
            "clock",
            "quick-settings",
        ] {
            assert!(ids.contains(&expected), "{expected} is on the bar but not in the inventory");
        }
    }

    use super::*;

    #[test]
    fn applets_default_shown_and_a_new_tray_item_defaults_to_overflow() {
        let items = assemble(
            vec![("org.x".into(), "X".into(), "x-icon".into())],
            &TopbarConfig::default(),
        );
        assert_eq!(items.len(), APPLETS.len() + 1);
        assert!(items.iter().find(|i| i.id == "clock").unwrap().shown);
        let tray = items.iter().find(|i| i.id == "tray:org.x").unwrap();
        assert!(!tray.shown, "a new tray item defaults to the overflow");
        assert_eq!(tray.kind, "tray");
        assert_eq!(tray.name, "X");
    }

    /// The watcher's filter, which decides whether a save bounces the bar.
    ///
    /// NOT covered here: that notify delivers the event at all. That rests on
    /// watching the parent directory rather than the file, because the atomic
    /// rename every writer here performs replaces the inode and a watch on the
    /// file itself would go deaf after the first save. It is the same shape the
    /// appearance watcher already live-reloads through.
    #[test]
    fn only_the_arrangement_file_bounces_the_bar() {
        let target = std::path::PathBuf::from("/home/u/.config/arlen/topbar.toml");
        assert!(touches_arrangement(&[target.clone()], &target));
        // The neighbours that share the watched directory.
        for neighbour in ["appearance.toml", "shell.toml", "graph.toml"] {
            let p = target.with_file_name(neighbour);
            assert!(!touches_arrangement(&[p], &target), "{neighbour} must not reload the bar");
        }
        // A save arrives as several paths at once; one match is enough.
        assert!(touches_arrangement(
            &[target.with_file_name("appearance.toml"), target.clone()],
            &target
        ));
        assert!(!touches_arrangement(&[], &target));
    }

    #[test]
    fn saved_order_and_visibility_apply() {
        let mut visible = HashMap::new();
        visible.insert("clock".to_string(), false);
        visible.insert("tray:org.x".to_string(), true);
        let config = TopbarConfig {
            order: vec!["clock".into(), "audio".into()],
            visible,
        };
        let items = assemble(vec![("org.x".into(), "X".into(), "i".into())], &config);
        // Configured ids come first in the saved order.
        assert_eq!(items[0].id, "clock");
        assert_eq!(items[1].id, "audio");
        // Config visibility wins: clock hidden, the tray item shown.
        assert!(!items.iter().find(|i| i.id == "clock").unwrap().shown);
        assert!(items.iter().find(|i| i.id == "tray:org.x").unwrap().shown);
    }
}
