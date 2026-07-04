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

use serde::{Deserialize, Serialize};

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

#[cfg(test)]
mod tests {
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
