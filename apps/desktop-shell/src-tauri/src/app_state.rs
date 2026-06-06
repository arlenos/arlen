//! Backend-side mirror of the per-app state surfaces
//! (shortcuts, eventually badges + ambient too) so backend
//! consumers — primarily the Waypointer plugin
//! `core.app-shortcuts` — can read what apps have registered
//! without going through the frontend.
//!
//! The frontend stores in `appStateStores.ts` are the *render*
//! state. The state here is the *query* state consulted by
//! Rust-side surfaces. Both flow from the same Event Bus
//! events; the forward functions in `event_bus.rs` update both
//! sides.
//!
//! Per-app keying. Multi-window apps share one shortcut list
//! across windows (foundation §6.4 Listing 13).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// One shortcut as stored in the backend mirror. Distinct from
/// the wire `proto::Shortcut` so frontend / Waypointer code
/// doesn't have to import proto types.
#[derive(Debug, Clone)]
pub struct ShortcutEntry {
    pub label: String,
    pub icon: String,
    pub action: String,
    #[allow(dead_code)]
    pub context: Vec<String>,
    pub confirm: Option<String>,
    pub enabled: bool,
    pub badge: Option<String>,
}

/// Shared per-app shortcut store. Populated by
/// `event_bus::forward_shortcut_event`. Read by
/// `waypointer_system::plugins::app_shortcuts::AppShortcutsPlugin`.
pub type ShortcutsState = Arc<Mutex<HashMap<String, Vec<ShortcutEntry>>>>;

pub fn new_shortcuts_state() -> ShortcutsState {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Apply a register event: replace this app's shortcut list.
/// Empty list deletes the entry (acts like cleared).
pub fn apply_register(state: &ShortcutsState, app_id: String, shortcuts: Vec<ShortcutEntry>) {
    let mut s = state.lock().unwrap();
    if shortcuts.is_empty() {
        s.remove(&app_id);
    } else {
        s.insert(app_id, shortcuts);
    }
}

/// Apply a state-change event: mutate one shortcut's enabled
/// or badge fields by action lookup. Silent no-op on miss.
pub fn apply_state_changed(
    state: &ShortcutsState,
    app_id: &str,
    action: &str,
    enabled: Option<bool>,
    badge: Option<String>,
) {
    let mut s = state.lock().unwrap();
    let Some(list) = s.get_mut(app_id) else {
        return;
    };
    for sc in list.iter_mut() {
        if sc.action == action {
            if let Some(en) = enabled {
                sc.enabled = en;
            }
            if let Some(b) = &badge {
                sc.badge = if b.is_empty() { None } else { Some(b.clone()) };
            }
            return;
        }
    }
}

/// Apply a clear event: drop this app's entire shortcut list.
pub fn apply_cleared(state: &ShortcutsState, app_id: &str) {
    state.lock().unwrap().remove(app_id);
}
