//! The daemon's minimal read of `ai.toml`: just the `[ai] enabled` master
//! switch that decides whether to spawn the pi sidecar (`pi-agent-adoption.md`
//! §D: `[ai] enabled=false` -> the daemon does not spawn pi).
//!
//! This deliberately does NOT depend on the ai-agent crate (the system being
//! replaced side-by-side); it reads only the one flag it needs, fail-closed:
//! a missing, unreadable or malformed config leaves AI disabled, so pi is never
//! spawned by accident.

use serde::Deserialize;
use std::path::PathBuf;

/// The `ai.toml` path: the `ARLEN_AI_CONFIG` override, else
/// `$HOME/.config/arlen/ai.toml`. Mirrors the ai-agent resolver so all the AI
/// components read the same file.
pub fn ai_config_path() -> PathBuf {
    if let Ok(path) = std::env::var("ARLEN_AI_CONFIG") {
        return PathBuf::from(path);
    }
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".config/arlen/ai.toml")
}

#[derive(Deserialize, Default)]
struct RawAi {
    #[serde(default)]
    enabled: bool,
    /// The provider backend NAME (e.g. `ollama`), keyed `ai.provider` like the
    /// other AI components; the model/window/token live in `[provider]`.
    #[serde(default)]
    provider: String,
}

fn default_model() -> String {
    "llama3:8b".to_string()
}
fn default_context_window() -> u32 {
    8192
}
fn default_audit_token() -> String {
    "ai-engine".to_string()
}

#[derive(Deserialize)]
struct RawProvider {
    #[serde(default = "default_model")]
    model: String,
    #[serde(default = "default_context_window")]
    context_window: u32,
    #[serde(default = "default_audit_token")]
    audit_token: String,
}

impl Default for RawProvider {
    fn default() -> Self {
        Self {
            model: default_model(),
            context_window: default_context_window(),
            audit_token: default_audit_token(),
        }
    }
}

#[derive(Deserialize, Default)]
struct RawAgent {
    /// The executor-live master switch: when false (default) the agent's graph
    /// writes stay fail-closed (proposals only); when true, an authorized write
    /// actually applies. The gate lift AND the live write executor both read it.
    #[serde(default)]
    executor_live: bool,
    /// The names of the behaviours the user has enabled in Settings (`[agent]
    /// enabled = ["auto-tag-by-project", ...]`). A behaviour not on this list is
    /// loaded but disabled (listed, never dispatched); the curator orchestrator
    /// only subscribes + dispatches the enabled set.
    #[serde(default)]
    enabled: Vec<String>,
}

#[derive(Deserialize, Default)]
struct RawConfig {
    #[serde(default)]
    ai: RawAi,
    #[serde(default)]
    provider: RawProvider,
    #[serde(default)]
    agent: RawAgent,
}

/// The resolved provider settings the live read pipeline builds its
/// `ProxiedProvider` from: the backend name (`ai.provider`) plus the
/// `[provider]` model / context window / audit token (with safe defaults). Maps
/// directly onto `arlen_ai_core::proxied::ProxiedConfig`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSettings {
    /// The backend name (`ai.provider`); empty means no provider configured.
    pub name: String,
    /// The model id.
    pub model: String,
    /// The context window (tokens).
    pub context_window: u32,
    /// The audit token the proxy records the call under.
    pub audit_token: String,
}

/// Whether `[ai] enabled` is true in the given `ai.toml` text. A malformed
/// document is treated as disabled (fail-closed), so a broken config never
/// causes the daemon to spawn pi.
pub fn ai_enabled_from_text(text: &str) -> bool {
    toml::from_str::<RawConfig>(text).map(|c| c.ai.enabled).unwrap_or(false)
}

/// The config-broker's master switches, cached so the daemon's sync accessors
/// (`ai_enabled`/`executor_live`, invoked as `fn() -> bool` from the executor
/// gates) read the authoritative switches without blocking on the broker's async
/// socket. `None` means the broker has not been reached (not yet fetched, or
/// currently unavailable); the accessors then fall back to reading `ai.toml`, so a
/// deployment where the config-broker is not (yet) running keeps working unchanged.
/// [`refresh_broker_switches`] keeps this current.
static BROKER_SWITCHES: std::sync::RwLock<Option<arlen_config_broker::AiMasterSwitches>> =
    std::sync::RwLock::new(None);

/// Publish the broker's switches (or `None` when it is unreachable) into the cache
/// the sync accessors read. Called by [`refresh_broker_switches`]; also the test
/// seam.
pub fn publish_broker_switches(switches: Option<arlen_config_broker::AiMasterSwitches>) {
    if let Ok(mut w) = BROKER_SWITCHES.write() {
        *w = switches;
    }
}

/// Whether the engine sources its master switches from the config-broker. OFF by
/// default: the broker read is only correct once the broker is the CURRENT source
/// (Settings writes it + it is always deployed), otherwise the engine would read a
/// broker that was seeded once and then went stale as the user edited `ai.toml`.
/// The deployment sets `ARLEN_ENGINE_CONFIG_BROKER=1` to complete the cutover; until
/// then the accessors read `ai.toml` exactly as before (no behaviour change). This
/// is the executor_live-style go-live gate for the config-broker cutover.
pub fn config_broker_enabled() -> bool {
    std::env::var("ARLEN_ENGINE_CONFIG_BROKER")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Read one field from the cached broker switches, if the cutover is enabled AND
/// the broker has been reached. Returns `None` (so the accessor falls back to
/// `ai.toml`) when the cutover flag is off or the broker is unreachable.
fn from_broker<T>(pick: impl FnOnce(&arlen_config_broker::AiMasterSwitches) -> T) -> Option<T> {
    if !config_broker_enabled() {
        return None;
    }
    BROKER_SWITCHES.read().ok().and_then(|g| g.as_ref().map(pick))
}

/// Poll the config-broker and publish its switches into the cache, so the sync
/// accessors see an authoritative change within `interval`. On ANY broker error
/// (down, corrupt, unauthorised) publish `None`, so the accessors fall back to
/// `ai.toml` rather than serving a stale broker value. Runs until the process
/// exits; spawned once at daemon startup.
pub async fn refresh_broker_switches(interval: std::time::Duration) {
    let client = arlen_config_broker::ConfigBrokerClient::default_socket();
    loop {
        match client.get().await {
            Ok(switches) => publish_broker_switches(Some(switches)),
            Err(_) => publish_broker_switches(None),
        }
        tokio::time::sleep(interval).await;
    }
}

/// Whether AI is enabled: the config-broker (the authoritative owner of the master
/// switch) when reached, else the on-disk `ai.toml` (missing/unreadable = disabled,
/// fail-closed). Preferring the broker is what moves the switch off the ambient
/// `ai.toml`; the ai.toml fallback keeps a broker-less deployment working during
/// the transition.
pub fn ai_enabled() -> bool {
    if let Some(v) = from_broker(|s| s.enabled) {
        return v;
    }
    std::fs::read_to_string(ai_config_path()).map(|t| ai_enabled_from_text(&t)).unwrap_or(false)
}

/// Resolve the [`ProviderSettings`] from the given `ai.toml` text. A malformed
/// document yields the safe defaults with an empty name (no provider), so the
/// caller builds no live provider rather than a wrong one (fail-closed).
pub fn provider_settings_from_text(text: &str) -> ProviderSettings {
    let cfg = toml::from_str::<RawConfig>(text).unwrap_or_default();
    ProviderSettings {
        name: cfg.ai.provider,
        model: cfg.provider.model,
        context_window: cfg.provider.context_window,
        audit_token: cfg.provider.audit_token,
    }
}

/// Resolve the [`ProviderSettings`] from the on-disk `ai.toml` (safe defaults +
/// empty name if missing/unreadable).
pub fn provider_settings() -> ProviderSettings {
    std::fs::read_to_string(ai_config_path())
        .map(|t| provider_settings_from_text(&t))
        .unwrap_or_else(|_| provider_settings_from_text(""))
}

/// Whether `[agent] executor_live` is true in the given `ai.toml` text. A
/// malformed document is treated as false (fail-closed): a broken config never
/// lifts the agent's writes out of proposal-only.
pub fn executor_live_from_text(text: &str) -> bool {
    toml::from_str::<RawConfig>(text)
        .map(|c| c.agent.executor_live)
        .unwrap_or(false)
}

/// Whether the executor is live: the config-broker (the authoritative owner of the
/// executor "human gate") when reached, else the on-disk `ai.toml`
/// (missing/unreadable = false, fail-closed). The broker read is what moves the
/// security-critical gate off the ambient `ai.toml`; the fallback keeps a
/// broker-less deployment working during the transition.
pub fn executor_live() -> bool {
    if let Some(v) = from_broker(|s| s.executor_live) {
        return v;
    }
    std::fs::read_to_string(ai_config_path())
        .map(|t| executor_live_from_text(&t))
        .unwrap_or(false)
}

/// The names of the behaviours enabled in the given `ai.toml` text (`[agent]
/// enabled = [...]`). A malformed document yields an empty list (fail-closed):
/// nothing is dispatched rather than guessing.
pub fn enabled_behaviour_names_from_text(text: &str) -> Vec<String> {
    toml::from_str::<RawConfig>(text)
        .map(|c| c.agent.enabled)
        .unwrap_or_default()
}

/// The enabled behaviour names from the on-disk `ai.toml` (missing/unreadable =
/// empty, fail-closed).
pub fn enabled_behaviour_names() -> Vec<String> {
    std::fs::read_to_string(ai_config_path())
        .map(|t| enabled_behaviour_names_from_text(&t))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes the cache tests: `BROKER_SWITCHES` is a process-global, and these
    /// also set `ARLEN_AI_CONFIG` (process env), so they must not run concurrently.
    static CACHE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn the_broker_read_is_off_by_default_and_wins_only_when_the_cutover_is_enabled() {
        let _g = CACHE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Point ai.toml at a temp file that says DISABLED + executor-off.
        let ai = std::env::temp_dir().join(format!("arlen-engcfg-{}.toml", std::process::id()));
        std::fs::write(&ai, "[ai]\nenabled = false\n[agent]\nexecutor_live = false\n").unwrap();
        std::env::set_var("ARLEN_AI_CONFIG", &ai);
        publish_broker_switches(Some(arlen_config_broker::AiMasterSwitches {
            enabled: true,
            executor_live: true,
            ..Default::default()
        }));

        // Cutover OFF (default): the broker cache is IGNORED, ai.toml governs - so a
        // seeded-then-stale broker never overrides the user's live ai.toml.
        std::env::remove_var("ARLEN_ENGINE_CONFIG_BROKER");
        assert!(!ai_enabled(), "with the cutover off, the broker cache must not win");
        assert!(!executor_live());

        // Cutover ON: the broker cache WINS over ai.toml.
        std::env::set_var("ARLEN_ENGINE_CONFIG_BROKER", "1");
        assert!(ai_enabled(), "with the cutover on, the broker cache overrides ai.toml");
        assert!(executor_live());

        // Reset the global cache + env so nothing leaks into other tests.
        publish_broker_switches(None);
        std::env::remove_var("ARLEN_ENGINE_CONFIG_BROKER");
        std::env::remove_var("ARLEN_AI_CONFIG");
        let _ = std::fs::remove_file(&ai);
    }

    #[test]
    fn enabled_true_only_when_set() {
        assert!(ai_enabled_from_text("[ai]\nenabled = true\n"));
        assert!(!ai_enabled_from_text("[ai]\nenabled = false\n"));
        // Absent flag / absent section -> disabled.
        assert!(!ai_enabled_from_text("[ai]\naccess_level = 2\n"));
        assert!(!ai_enabled_from_text(""));
        // Other sections present, no [ai] enabled -> disabled.
        assert!(!ai_enabled_from_text("[agent]\nexecutor_live = true\n"));
    }

    #[test]
    fn malformed_config_is_disabled() {
        assert!(!ai_enabled_from_text("this is = not valid toml ["));
        // A wrong type for enabled fails the parse -> fail-closed disabled.
        assert!(!ai_enabled_from_text("[ai]\nenabled = \"yes\"\n"));
    }

    #[test]
    fn provider_settings_reads_name_and_section_with_defaults() {
        let s = provider_settings_from_text(
            "[ai]\nenabled = true\nprovider = \"ollama\"\n\n[provider]\nmodel = \"qwen2:7b\"\ncontext_window = 4096\n",
        );
        assert_eq!(s.name, "ollama");
        assert_eq!(s.model, "qwen2:7b");
        assert_eq!(s.context_window, 4096);
        // Unset audit_token falls to the default.
        assert_eq!(s.audit_token, "ai-engine");
    }

    #[test]
    fn executor_live_only_when_set_true_fail_closed() {
        assert!(executor_live_from_text("[agent]\nexecutor_live = true\n"));
        assert!(!executor_live_from_text("[agent]\nexecutor_live = false\n"));
        // Absent section / flag -> false.
        assert!(!executor_live_from_text("[ai]\nenabled = true\n"));
        assert!(!executor_live_from_text(""));
        // Malformed -> false (fail-closed, never lifts writes).
        assert!(!executor_live_from_text("not = valid ["));
        assert!(!executor_live_from_text("[agent]\nexecutor_live = \"yes\"\n"));
    }

    #[test]
    fn enabled_behaviour_names_parse_fail_closed() {
        assert_eq!(
            enabled_behaviour_names_from_text("[agent]\nenabled = [\"auto-tag-by-project\", \"meeting-prep\"]\n"),
            vec!["auto-tag-by-project".to_string(), "meeting-prep".to_string()]
        );
        // Absent section / flag -> empty.
        assert!(enabled_behaviour_names_from_text("[ai]\nenabled = true\n").is_empty());
        assert!(enabled_behaviour_names_from_text("").is_empty());
        // Malformed -> empty (fail-closed, nothing dispatched).
        assert!(enabled_behaviour_names_from_text("not = valid [").is_empty());
    }

    #[test]
    fn provider_settings_defaults_when_absent_or_malformed() {
        // No [provider] section, no ai.provider: empty name (no provider), safe
        // model/window/token defaults.
        let s = provider_settings_from_text("[ai]\nenabled = false\n");
        assert_eq!(s.name, "");
        assert_eq!(s.model, "llama3:8b");
        assert_eq!(s.context_window, 8192);
        // Malformed -> defaults with empty name (fail-closed, no live provider).
        let m = provider_settings_from_text("not = valid [");
        assert_eq!(m.name, "");
        assert_eq!(m.model, "llama3:8b");
    }
}
