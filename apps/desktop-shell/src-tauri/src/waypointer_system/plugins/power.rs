/// Power-action plugin: sleep / lock / restart / shutdown / logout.
///
/// Activated by any of the keyword aliases below (fuzzy-matched on
/// prefix). One action per result — hitting Enter executes immediately;
/// there is deliberately no confirmation step. Rationale: if the user
/// typed `shutdown` and pressed Enter, they meant shutdown. Adding a
/// confirmation dialog would either be a modal (fights with the
/// Waypointer's "Escape to cancel" muscle memory) or a description
/// warning (ignored after the first use). macOS and GNOME ship the
/// same no-confirmation model for Spotlight / GNOME Shell search.
use std::process::{Command, Stdio};

use crate::waypointer_system::plugin::*;

pub struct PowerPlugin;

/// One power-action definition: keywords that match it, the freedesktop
/// icon name for display, and the command-line invocation to run on
/// execute. All actions share this shape so the search loop is a
/// single flat iteration.
struct PowerAction {
    /// Stable id used in `SearchResult.id` and as the `handler` in
    /// `Action::Custom`. `power.sleep`, `power.lock`, …
    id: &'static str,

    /// freedesktop icon name. `system-suspend`, `system-lock-screen`,
    /// `system-reboot`, `system-shutdown`, `system-log-out`.
    icon: &'static str,
    /// Keywords that trigger this action. First entry is the canonical
    /// name; the rest are aliases. All matched case-insensitive.
    ///
    /// **English only, and that is a gap, not a decision.** Typing
    /// "ruhezustand" finds nothing. Matching happens here, in a process with no
    /// catalog on disk to read: the shell's messages live in TypeScript and only
    /// the Settings app exports catalogs for a Rust reader today. Closing it
    /// means exporting the shell's catalog the same way and resolving a keyword
    /// message per locale, which is the same shape as the settings index and is
    /// its own piece of work.
    keywords: &'static [&'static str],
    /// Command + args to run on execute. An empty argv signals the
    /// special "terminate session" path handled in `execute_action`
    /// (needs `XDG_SESSION_ID` at runtime).
    command: &'static [&'static str],
}

/// All power actions. Keep priorities stable for UX: the order here
/// also seeds the tie-break when every keyword scores identically.
const ACTIONS: &[PowerAction] = &[
    PowerAction {
        id: "power.sleep",
        icon: "system-suspend",
        keywords: &["sleep", "suspend"],
        command: &["systemctl", "suspend"],
    },
    PowerAction {
        id: "power.lock",
        icon: "system-lock-screen",
        keywords: &["lock", "screen"],
        command: &["loginctl", "lock-session"],
    },
    PowerAction {
        id: "power.restart",
        icon: "system-reboot",
        keywords: &["restart", "reboot"],
        command: &["systemctl", "reboot"],
    },
    PowerAction {
        id: "power.shutdown",
        icon: "system-shutdown",
        keywords: &["shutdown", "poweroff"],
        command: &["systemctl", "poweroff"],
    },
    PowerAction {
        id: "power.logout",
        icon: "system-log-out",
        // Empty command = handled specially via XDG_SESSION_ID.
        keywords: &["logout", "sign out"],
        command: &[],
    },
];

impl WaypointerPlugin for PowerPlugin {
    fn id(&self) -> &str {
        "core.power"
    }
    fn name(&self) -> &str {
        "Power"
    }
    fn description(&self) -> &str {
        "Sleep, lock, restart, shut down, or log out from the keyboard."
    }
    // Priority 0 puts Power on par with App Search; the relevance-first
    // sort in the manager ensures an exact keyword match (relevance 1.0)
    // wins over a partial app match (~0.7).
    fn priority(&self) -> u32 {
        0
    }
    fn max_results(&self) -> usize {
        5
    }

    fn search(&self, query: &str) -> Vec<SearchResult> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }

        let mut results: Vec<(f32, &PowerAction)> = Vec::new();

        for action in ACTIONS {
            if let Some(score) = score_against_keywords(&q, action.keywords) {
                results.push((score, action));
            }
        }

        // Sort by score DESC, stable (keeps ACTIONS declaration order
        // for ties — sleep before lock before restart, etc.).
        results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        results
            .into_iter()
            .map(|(score, action)| SearchResult {
                id: action.id.into(),
                // Ids: this process cannot know what language the reader wants,
                // and the Waypointer holds the live locale. `power.sleep` becomes
                // `s.wp.power.sleep`, so the id list is the message list.
                title_key: Some(format!("s.wp.{}", action.id)),
                // Only shown if the shell's catalog is missing the message; the
                // action id is at least true then.
                title: action.id.into(),
                description_key: Some(format!("s.wp.{}.desc", action.id)),
                description: None,
                icon: Some(action.icon.into()),
                relevance: score,
                action: Action::Custom {
                    handler: "power_action".into(),
                    // Carry the action id through to `execute()`. The
                    // frontend opens the Custom dispatch path which
                    // round-trips to `execute()` below.
                    data: serde_json::json!({ "action": action.id }),
                },
                plugin_id: String::new(),
            })
            .collect()
    }

    fn execute(&self, result: &SearchResult) -> Result<(), PluginError> {
        let Action::Custom { ref data, .. } = result.action else {
            return Err(PluginError::ExecuteFailed(
                "power: unexpected action variant".into(),
            ));
        };
        let Some(action_id) = data.get("action").and_then(|v| v.as_str()) else {
            return Err(PluginError::ExecuteFailed(
                "power: missing 'action' field".into(),
            ));
        };
        let Some(action) = ACTIONS.iter().find(|a| a.id == action_id) else {
            return Err(PluginError::ExecuteFailed(format!(
                "power: unknown action '{action_id}'"
            )));
        };

        execute_action(action)
    }
}

/// Score a query against a set of keywords. Returns `None` if no
/// keyword matches at all. Scoring ranks exact > prefix > substring
/// so that `sleep` typed in full scores higher than `slee` which in
/// turn scores higher than just `lee` matching in the middle.
fn score_against_keywords(query: &str, keywords: &[&str]) -> Option<f32> {
    let mut best: Option<f32> = None;
    for kw in keywords {
        let kw_lower = kw.to_lowercase();
        let score = if query == kw_lower {
            1.0
        } else if kw_lower.starts_with(query) {
            // Prefix score scales with how much of the keyword the
            // user typed: 1/3 of "sleep" (`sle`) scores 0.80, 4/5
            // (`slee`) scores 0.92.
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

/// Run the `PowerAction`'s command. Uses `systemctl` / `loginctl`
/// without arguments that would prompt — the assumption is that
/// the user's logind policy allows this without polkit prompts
/// (the default on most systemd distros for desktop sessions).
fn execute_action(action: &PowerAction) -> Result<(), PluginError> {
    // Logout: the command needs the runtime session ID from the
    // environment. We can't hard-code it in the table because it
    // differs per session.
    if action.id == "power.logout" {
        let session = std::env::var("XDG_SESSION_ID").map_err(|_| {
            PluginError::ExecuteFailed(
                "logout: XDG_SESSION_ID not set; cannot locate session".into(),
            )
        })?;
        return spawn(&["loginctl", "terminate-session", &session]);
    }
    spawn(action.command)
}

/// Spawn a command, detach stdio, and return immediately. Any failure
/// to spawn is surfaced as a PluginError; success doesn't imply the
/// action succeeded (e.g. systemctl may reject the verb) — but for
/// power actions the process is typically gone before we could check
/// a status code anyway.
fn spawn(argv: &[&str]) -> Result<(), PluginError> {
    if argv.is_empty() {
        return Err(PluginError::ExecuteFailed(
            "power: empty command vector".into(),
        ));
    }
    Command::new(argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| PluginError::ExecuteFailed(format!("power: spawn {} failed: {e}", argv[0])))
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_scores_one() {
        let p = PowerPlugin;
        let r = p.search("sleep");
        assert!(!r.is_empty());
        assert!((r[0].relevance - 1.0).abs() < f32::EPSILON);
        assert_eq!(r[0].id, "power.sleep");
    }

    #[test]
    fn prefix_match_scores_below_exact() {
        let p = PowerPlugin;
        let slee = &p.search("slee")[0];
        let full = &p.search("sleep")[0];
        assert!(slee.relevance < full.relevance);
        assert!(slee.relevance > 0.7);
    }

    #[test]
    fn alias_maps_to_canonical_id() {
        let p = PowerPlugin;
        let r = p.search("suspend");
        assert_eq!(r[0].id, "power.sleep");
        // The name is a message id now; the shell resolves it in the reader's
        // language, and this process does not know what that is.
        assert_eq!(r[0].title_key.as_deref(), Some("s.wp.power.sleep"));

        let r = p.search("reboot");
        assert_eq!(r[0].id, "power.restart");
    }

    /// Every power action must have a name in the shell's catalog.
    ///
    /// The prose is no longer here, so nothing in this file breaks if the message
    /// is missing: the Waypointer would show `s.wp.power.sleep` in its results.
    /// The id-scanner cannot catch that one, because the id is minted in Rust and
    /// appears in no Svelte file.
    #[test]
    fn every_power_action_has_a_name_in_the_catalog() {
        let catalog = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../src/lib/i18n/messages.ts"
        ))
        .expect("the shell catalog");

        let missing: Vec<String> = ACTIONS
            .iter()
            .flat_map(|a| [format!("s.wp.{}", a.id), format!("s.wp.{}.desc", a.id)])
            .filter(|key| !catalog.contains(&format!("\"{key}\":")))
            .collect();
        assert!(missing.is_empty(), "no message for: {missing:?}");
    }

    #[test]
    fn unrelated_query_returns_nothing() {
        let p = PowerPlugin;
        assert!(p.search("firefox").is_empty());
        assert!(p.search("").is_empty());
        assert!(p.search("   ").is_empty());
    }

    #[test]
    fn all_five_actions_reachable() {
        let p = PowerPlugin;
        for (query, expected_id) in [
            ("sleep", "power.sleep"),
            ("lock", "power.lock"),
            ("restart", "power.restart"),
            ("shutdown", "power.shutdown"),
            ("logout", "power.logout"),
        ] {
            let r = p.search(query);
            assert!(!r.is_empty(), "query '{query}' returned nothing");
            assert_eq!(r[0].id, expected_id, "query '{query}'");
        }
    }

    #[test]
    fn fuzzy_fragment_finds_action() {
        // Inner-substring match still scores but ranks below prefix.
        let p = PowerPlugin;
        let r = p.search("boot"); // contained in "reboot"
        assert!(!r.is_empty());
        assert_eq!(r[0].id, "power.restart");
        assert!(r[0].relevance >= 0.5 && r[0].relevance < 0.75);
    }

    #[test]
    fn case_insensitive_matching() {
        let p = PowerPlugin;
        assert!(!p.search("SLEEP").is_empty());
        assert!(!p.search("SlEeP").is_empty());
    }

    #[test]
    fn execute_rejects_missing_action_field() {
        let p = PowerPlugin;
        let result = SearchResult {
            title_key: None,
            description_key: None,
            id: "power.sleep".into(),
            title: "Sleep".into(),
            description: None,
            icon: None,
            relevance: 1.0,
            action: Action::Custom {
                handler: "power_action".into(),
                data: serde_json::json!({}),
            },
            plugin_id: String::new(),
        };
        assert!(p.execute(&result).is_err());
    }

    #[test]
    fn execute_rejects_unknown_action_id() {
        let p = PowerPlugin;
        let result = SearchResult {
            title_key: None,
            description_key: None,
            id: "power.fly".into(),
            title: "Fly".into(),
            description: None,
            icon: None,
            relevance: 1.0,
            action: Action::Custom {
                handler: "power_action".into(),
                data: serde_json::json!({ "action": "power.fly" }),
            },
            plugin_id: String::new(),
        };
        assert!(p.execute(&result).is_err());
    }
}
