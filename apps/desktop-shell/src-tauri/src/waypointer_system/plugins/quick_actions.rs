/// Quick Actions plugin for the Waypointer.
///
/// One-shot toggles, theme switches, and Settings-launchers. Mirrors
/// the `power.rs` pattern but with a critical difference: the
/// dispatch path goes through a dedicated Tauri command
/// (`quick_action_run`) rather than this plugin's `execute()`. The
/// reason is process-boundary: `WaypointerPlugin::execute()` is sync
/// with no `AppHandle`, so it can't call into Tauri-managed state
/// (`set_dnd_enabled`, `night_light_set`, …). The frontend's
/// dispatch path mirrors the `waypointerPower.ts` pattern: a
/// dedicated store + invoke pair.
///
/// `execute()` here is therefore a no-op success — the action has
/// already been performed by the time the manager calls it. The
/// catalog (id, title, description, icon, keywords) is the only
/// source of truth for what's available; dispatch logic lives in
/// `lib.rs::quick_action_run`.

use crate::waypointer_system::plugin::*;

pub struct QuickActionsPlugin;

/// Catalog row. Same shape as `power.rs::PowerAction` minus the
/// argv field — dispatch happens server-side via Tauri command.
struct QuickAction {
    /// Stable identifier shipped to the frontend in `Action::Custom.data.id`.
    /// `quick_action_run` switches on this string.
    id: &'static str,
    title: &'static str,
    description: &'static str,
    /// freedesktop icon name. Frontend resolves through the
    /// existing `lucide-svelte` icon set when possible.
    icon: &'static str,
    /// Keywords matched fuzzy against the user's query. First entry
    /// is canonical; additional aliases broaden discoverability.
    keywords: &'static [&'static str],
}

const ACTIONS: &[QuickAction] = &[
    // ── Toggles (binary state) ───────────────────────────────────────
    // DND is split into explicit enable / disable because the
    // shell side has no local DND-state cache — the daemon owns
    // it. Implicit "toggle" would require an async daemon round-
    // trip just to flip a known value (Sprint D plan E11 + the
    // Codex review's audit of cascading state).
    QuickAction {
        id: "qa.dnd_enable",
        title: "Enable Do Not Disturb",
        description: "Suppress notifications until disabled",
        icon: "bell-off",
        keywords: &["dnd", "do not disturb", "silent", "mute notifications", "enable"],
    },
    QuickAction {
        id: "qa.dnd_disable",
        title: "Disable Do Not Disturb",
        description: "Resume normal notifications",
        icon: "bell",
        keywords: &["dnd", "do not disturb", "disable", "resume", "unmute"],
    },
    QuickAction {
        id: "qa.toggle_night_light",
        title: "Toggle Night Light",
        description: "Warm-tint screen for evening reading",
        icon: "moon",
        keywords: &["night", "light", "warm", "evening", "blue light"],
    },
    QuickAction {
        id: "qa.toggle_airplane",
        title: "Toggle Airplane Mode",
        description: "Disable WiFi and Bluetooth",
        icon: "plane",
        keywords: &["airplane", "flight", "plane", "offline"],
    },
    QuickAction {
        id: "qa.toggle_wifi",
        title: "Toggle WiFi",
        description: "Enable or disable wireless networking",
        icon: "wifi",
        keywords: &["wifi", "wireless", "network"],
    },
    QuickAction {
        id: "qa.toggle_bluetooth",
        title: "Toggle Bluetooth",
        description: "Enable or disable the Bluetooth adapter",
        icon: "bluetooth",
        keywords: &["bluetooth", "bt"],
    },
    QuickAction {
        id: "qa.toggle_caffeine",
        title: "Toggle Caffeine",
        description: "Prevent the system from sleeping",
        icon: "coffee",
        keywords: &["caffeine", "sleep", "stay awake", "presentation"],
    },
    QuickAction {
        id: "qa.toggle_recording",
        title: "Toggle Screen Recording",
        description: "Start or stop wf-recorder",
        icon: "video",
        keywords: &["screen", "recording", "record", "video", "capture"],
    },

    // ── Theme switches (explicit, not toggle) ────────────────────────
    QuickAction {
        id: "qa.theme_dark",
        title: "Switch to Dark Theme",
        description: "Apply the built-in dark theme",
        icon: "moon",
        keywords: &["dark", "theme", "night mode"],
    },
    QuickAction {
        id: "qa.theme_light",
        title: "Switch to Light Theme",
        description: "Apply the built-in light theme",
        icon: "sun",
        keywords: &["light", "theme", "day mode"],
    },

    // ── Settings launchers ───────────────────────────────────────────
    QuickAction {
        id: "qa.open_settings",
        title: "Open Settings",
        description: "Launch the Lunaris Settings app",
        icon: "settings",
        keywords: &["settings", "preferences", "config"],
    },
    QuickAction {
        id: "qa.open_settings_appearance",
        title: "Settings: Appearance",
        description: "Theme, accent color, fonts",
        icon: "palette",
        keywords: &["appearance", "theme", "color", "fonts"],
    },
    QuickAction {
        id: "qa.open_settings_display",
        title: "Settings: Display",
        description: "Monitors, resolution, night light",
        icon: "monitor",
        keywords: &["display", "monitor", "resolution", "night light"],
    },
    QuickAction {
        id: "qa.open_settings_keyboard",
        title: "Settings: Keyboard",
        description: "Layout and shortcuts",
        icon: "keyboard",
        keywords: &["keyboard", "shortcut", "layout"],
    },
    QuickAction {
        id: "qa.open_settings_focus",
        title: "Settings: Focus Mode",
        description: "Project detection and suppressed apps",
        icon: "crosshair",
        keywords: &["focus", "project", "concentration", "do not disturb"],
    },
    QuickAction {
        id: "qa.open_settings_notifications",
        title: "Settings: Notifications",
        description: "DND, per-app rules, toast appearance",
        icon: "bell",
        keywords: &["notifications", "alerts", "dnd", "toast"],
    },
];

impl WaypointerPlugin for QuickActionsPlugin {
    fn id(&self) -> &str {
        "core.quick_actions"
    }
    fn name(&self) -> &str {
        "Quick Actions"
    }
    fn description(&self) -> &str {
        "Toggles, theme switches, and Settings shortcuts."
    }
    /// Priority 50 puts Quick Actions BELOW Apps (priority 10) so
    /// `Settings.app` wins over `Open Settings` for the "settings"
    /// query — the user's expectation when typing an app name is
    /// to launch that app. Quick Actions still surface for
    /// Lunaris-specific keywords (DND, brightness, …) where Apps
    /// has nothing to offer (compositor #29 sprint D plan).
    fn priority(&self) -> u32 {
        50
    }
    fn max_results(&self) -> usize {
        8
    }

    fn search(&self, query: &str) -> Vec<SearchResult> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }

        let mut results: Vec<(f32, &QuickAction)> = Vec::new();
        for action in ACTIONS {
            if let Some(score) = score_against_keywords(&q, action.keywords) {
                results.push((score, action));
            }
        }
        results.sort_by(|a, b| {
            b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal)
        });

        results
            .into_iter()
            .map(|(score, action)| SearchResult {
                id: action.id.into(),
                title: action.title.into(),
                description: Some(action.description.into()),
                icon: Some(action.icon.into()),
                relevance: score,
                action: Action::Custom {
                    handler: "quick_action_run".into(),
                    // The Tauri command reads `id` from this
                    // payload. Catalog → id mapping is the only
                    // source of truth; the actual dispatch logic
                    // lives in `lib.rs::quick_action_run`.
                    data: serde_json::json!({ "id": action.id }),
                },
                plugin_id: String::new(),
            })
            .collect()
    }

    /// No-op success. The frontend dispatches via a dedicated
    /// `quick_action_run` Tauri command before this hook fires —
    /// returning Ok keeps the manager's accounting clean.
    ///
    /// We deliberately don't touch Tauri-managed state from this
    /// path: `WaypointerPlugin::execute` is sync and has no
    /// `AppHandle`. The dispatch contract is documented at the top
    /// of this file.
    fn execute(&self, _result: &SearchResult) -> Result<(), PluginError> {
        Ok(())
    }
}

/// Same scoring rules power.rs uses — keep behaviour consistent
/// across plugins so users can rely on a single muscle-memory.
fn score_against_keywords(query: &str, keywords: &[&str]) -> Option<f32> {
    let mut best: Option<f32> = None;
    for kw in keywords {
        let kw_lower = kw.to_lowercase();
        let score = if query == kw_lower {
            1.0
        } else if kw_lower.starts_with(query) {
            0.75 + 0.2 * (query.len() as f32 / kw_lower.len() as f32)
        } else if kw_lower.contains(query) {
            0.55
        } else {
            continue;
        };
        best = Some(best.map_or(score, |b| b.max(score)));
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Catalog-coverage smoke: a dozen common queries each return
    /// at least one matching action. Catches regressions when a
    /// keyword gets removed or renamed.
    #[test]
    fn catalog_covers_common_keywords() {
        let p = QuickActionsPlugin;
        for q in [
            "dnd",
            "wifi",
            "bluetooth",
            "night",
            "airplane",
            "caffeine",
            "recording",
            "dark",
            "light",
            "settings",
        ] {
            let r = p.search(q);
            assert!(
                !r.is_empty(),
                "no quick-action results for query {q:?}"
            );
        }
    }

    /// Each result must carry the dispatch handler + id. Frontend
    /// reads both — missing either breaks the dispatch path.
    #[test]
    fn results_carry_handler_and_id() {
        let p = QuickActionsPlugin;
        for r in p.search("dnd") {
            let Action::Custom { ref handler, ref data } = r.action else {
                panic!("expected Action::Custom, got {:?}", r.action);
            };
            assert_eq!(handler, "quick_action_run");
            assert!(
                data.get("id").and_then(|v| v.as_str()).is_some(),
                "missing id in data: {data}"
            );
        }
    }

    /// Search relevance: an exact keyword match must score 1.0
    /// and rank first. "dnd" matches both enable + disable since
    /// both rows declare the keyword.
    #[test]
    fn exact_match_scores_highest() {
        let p = QuickActionsPlugin;
        let r = p.search("dnd");
        assert!(!r.is_empty());
        assert!(
            (r[0].relevance - 1.0).abs() < f32::EPSILON,
            "exact 'dnd' should score 1.0, got {}",
            r[0].relevance
        );
        assert!(
            r[0].id.starts_with("qa.dnd_"),
            "top-scoring DND action should be a dnd_* row, got {}",
            r[0].id
        );
    }

    /// Plugin ID is stable across releases — the registry file and
    /// any external module that references this plugin must keep
    /// resolving.
    #[test]
    fn plugin_id_is_stable() {
        let p = QuickActionsPlugin;
        assert_eq!(p.id(), "core.quick_actions");
    }

    /// Priority must stay at 50 so Apps' priority-10 results
    /// outrank Quick-Actions for app-name queries (see "settings"
    /// keyword overlap discussed in Sprint D plan E27).
    #[test]
    fn priority_is_below_apps() {
        let p = QuickActionsPlugin;
        assert_eq!(p.priority(), 50);
    }

    /// Empty query returns nothing — Quick-Actions is search-only,
    /// it doesn't suggest random items on focus.
    #[test]
    fn empty_query_returns_empty() {
        let p = QuickActionsPlugin;
        assert!(p.search("").is_empty());
        assert!(p.search("   ").is_empty());
    }

    /// `execute()` is a no-op by design — frontend dispatched via
    /// Tauri command before the manager called us.
    #[test]
    fn execute_is_noop_success() {
        let p = QuickActionsPlugin;
        let stub = SearchResult {
            id: "qa.toggle_dnd".into(),
            title: "Toggle DND".into(),
            description: None,
            icon: None,
            relevance: 1.0,
            action: Action::Custom {
                handler: "quick_action_run".into(),
                data: serde_json::json!({ "id": "qa.toggle_dnd" }),
            },
            plugin_id: "core.quick_actions".into(),
        };
        assert!(p.execute(&stub).is_ok());
    }
}
