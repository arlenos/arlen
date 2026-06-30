//! `ai.toml` config loading + live watch.
//!
//! The AI layer is opt-in (Foundation §5.1-5.2): the daemon starts
//! fail-closed and only begins serving queries once Settings writes
//! `enabled = true` into `~/.config/arlen/ai.toml`. This module is
//! the watcher that makes that toggle live (Phase 9-α S7).
//!
//! Scope of the live reload:
//!
//! * `enabled` — applied live. Toggling it in Settings switches the
//!   AI layer on/off without a daemon restart.
//! * `provider`: read once at startup. The provider name (`ai.provider`)
//!   and the optional `[provider]` section (model, context window, audit
//!   token) are applied at startup only; a provider change needs a daemon
//!   restart, the same convention `graph.toml` uses (Settings surfaces that
//!   hint). Live provider switching waits for multi-provider routing; with a
//!   single catalogued provider there is nothing to switch between.

use std::sync::Arc;

use os_sdk::config::Config;

use arlen_ai_core::audit::{config_change_event, AuditSink};
use arlen_ai_core::capability::access_tier_from_level;
use arlen_ai_core::graph_query::QueryScope;
use arlen_ai_core::graph_schema::GraphSchema;
use arlen_config_broker::{AiMasterSwitches, ClientError, ConfigBrokerClient};

use crate::service::AiDaemonService;

/// The catalogued LLM provider the daemon forwards completions through,
/// resolved from `ai.toml`: the name from the shared `ai.provider` key (also
/// read by ai-agent, written by Settings), and the model, context window, and
/// audit token from an optional `[provider]` section. Mirrors ai-agent's
/// `ProviderSettings` (`ai-agent/src/config.rs`) so both halves of the AI layer
/// read one provider config from one file; a shared `ai-core` type is a clean
/// follow-up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSettings {
    /// Catalogued provider name the proxy forwards to (`ai.provider`).
    pub name: String,
    /// Model identifier (`[provider] model`).
    pub model: String,
    /// The model's usable input context window, in tokens
    /// (`[provider] context_window`).
    pub context_window: u32,
    /// Capability token presented to the proxy (`[provider] audit_token`).
    pub audit_token: String,
}

impl Default for ProviderSettings {
    fn default() -> Self {
        Self {
            name: DEFAULT_PROVIDER.to_string(),
            model: DEFAULT_MODEL.to_string(),
            context_window: DEFAULT_CONTEXT_WINDOW,
            audit_token: DEFAULT_AUDIT_TOKEN.to_string(),
        }
    }
}

/// Settings parsed from `ai.toml`. The [`Default`] is the fail-closed posture
/// used when the file cannot be read: disabled, the default local provider
/// (unused while disabled), and Minimal access (level 0, no graph reads until
/// the user raises it).
#[derive(Debug, Clone, Default)]
pub struct AiSettings {
    /// Whether the AI layer accepts queries.
    pub enabled: bool,
    /// The provider the daemon forwards completions through.
    pub provider: ProviderSettings,
    /// Global read access level 0..=4 (Foundation §8.4 table). Decides
    /// how much of the graph the AI can see; mapped to an
    /// `AccessTier` by `arlen_ai_core::capability::access_tier_from_level`.
    pub access_level: u8,
    /// Whether queries run through the interactive MCP tool-use loop
    /// (`tool_loop`) instead of the single-shot graph-query pipeline.
    /// Default ON (PR-5): the loop is built, tested and gated (the
    /// always-confirm list + read-only default-permit + depth/chain bound),
    /// and without it the connected MCP servers are unreachable from a user
    /// query. Set `ai.tool_routing = false` to opt back to the single-shot
    /// `QueryRunner` path (`docs/architecture/ai-tool-routing.md`).
    pub tool_routing: bool,
}

/// Catalogued provider when `ai.provider` is absent: the local Ollama backend.
/// Used only for a missing key (the unconfigured query daemon needs a backend),
/// not for a present-but-invalid value, which fails closed.
const DEFAULT_PROVIDER: &str = "ollama-default";
/// Model when `[provider] model` is omitted. Matches the `ollama-default`
/// backend and ai-agent's default, so a named provider works out of the box.
const DEFAULT_MODEL: &str = "llama3:8b";
/// Conservative input context window when `[provider] context_window` is
/// omitted: llama3:8b ships 8192. A real deployment sets its model's window.
const DEFAULT_CONTEXT_WINDOW: u32 = 8_192;
/// Token presented to the proxy when `[provider] audit_token` is omitted. The
/// proxy only records it until S15 validates it against the caller identity.
const DEFAULT_AUDIT_TOKEN: &str = "ai-daemon-default-token";

/// Read `ai.access_level` as a clamped `u8`. The config loader only
/// decodes TOML integers as `i64`, so this narrows it: a MISSING level
/// yields 3 (TimeScoped, recent activity) - the generous default so an
/// enabled AI is useful out of the box rather than blind, the user
/// narrowing if they want. A present-but-negative or out-of-byte-range
/// value still yields 0 (Minimal), and `access_tier_from_level` clamps
/// anything above 4 back to Minimal, so a MALFORMED level never widens
/// access; only the deliberate generous default does.
fn read_access_level(cfg: &Config) -> u8 {
    u8::try_from(cfg.get::<i64>("ai.access_level").unwrap_or(3)).unwrap_or(0)
}

/// Resolve the provider config from `ai.toml`: the name from the shared
/// `ai.provider` key, the model, context window, and audit token from an
/// optional `[provider]` section.
///
/// The provider name decides whether and where LLM traffic leaves, so an
/// *absent* key (an unconfigured daemon) defaults to the local backend, the
/// value Settings writes, while a *present but invalid* value (blank or
/// wrong-typed) fails closed: it yields an empty name the proxy rejects, so a
/// cleared or corrupted field never silently re-enables forwarding. Absent is
/// distinguished from invalid deliberately, because the provider is read once
/// at startup (a change needs a restart): defaulting an absent key keeps a
/// daemon that started before `ai.toml` existed from latching an empty provider
/// for its lifetime, whereas an explicitly invalid value is a misconfiguration
/// that should not be papered over with a backend. The within-provider fields
/// (model, window, token) default on a blank value, since they select within
/// an already-named backend.
fn read_provider(cfg: &Config) -> ProviderSettings {
    let non_empty = |key: &str| cfg.get::<String>(key).filter(|s| !s.is_empty());
    let name = match cfg.get_raw("ai.provider") {
        // Absent: default to the local backend (an unconfigured query daemon
        // needs one, and this matches the value Settings writes).
        None => DEFAULT_PROVIDER.to_string(),
        // Present: only a non-empty string forwards; a blank or wrong-typed
        // value fails closed rather than guessing a backend.
        Some(_) => non_empty("ai.provider").unwrap_or_else(|| {
            tracing::warn!(
                "ai.provider is set but blank or not a string; the daemon will not \
                 forward to any backend (queries fail at the provider step)"
            );
            String::new()
        }),
    };
    // The loader decodes TOML integers as i64; narrow to a positive u32,
    // falling back to the default on a missing, non-positive, or oversized
    // value so a malformed window never reaches the provider.
    let context_window = cfg
        .get::<i64>("provider.context_window")
        .and_then(|v| u32::try_from(v).ok())
        .filter(|&w| w > 0)
        .unwrap_or(DEFAULT_CONTEXT_WINDOW);
    ProviderSettings {
        name,
        model: non_empty("provider.model").unwrap_or_else(|| DEFAULT_MODEL.to_string()),
        context_window,
        audit_token: non_empty("provider.audit_token")
            .unwrap_or_else(|| DEFAULT_AUDIT_TOKEN.to_string()),
    }
}

/// Drive the service into the fail-closed security state: no queries
/// accepted, no graph access. Applied on every path where `ai.toml`
/// cannot be trusted — it could not be loaded, watched, or reparsed —
/// so a malformed, truncated, or unreadable file never leaves a broad
/// scope or an enabled daemon in place, regardless of what an earlier
/// valid load had applied.
fn fail_closed(service: &AiDaemonService) {
    // Publish disabled + Minimal as one atomic admission state, so a
    // concurrent query cannot catch a half-applied transition.
    service.set_admission(
        false,
        access_tier_from_level(0),
        QueryScope::for_tier(access_tier_from_level(0), &GraphSchema::knowledge_graph()),
    );
}

/// Read `ai.toml` once. A missing or unreadable file yields the
/// fail-closed default (`enabled = false`).
pub fn load_ai_settings() -> AiSettings {
    match Config::load("ai") {
        Ok(cfg) => AiSettings {
            enabled: cfg.get::<bool>("ai.enabled").unwrap_or(false),
            provider: read_provider(&cfg),
            access_level: read_access_level(&cfg),
            tool_routing: cfg.get::<bool>("ai.tool_routing").unwrap_or(true),
        },
        Err(err) => {
            tracing::warn!(error = %err, "ai.toml unreadable, defaulting to disabled");
            AiSettings::default()
        }
    }
}

/// Read the raw `ai.toml` text, for `Screener::from_config`'s `[classifier]`
/// parse. Empty on a missing or unreadable file, so the screener flows `Off`
/// (the deliberate opt-out posture).
pub fn load_ai_text() -> String {
    std::fs::read_to_string(os_sdk::config::config_path("ai")).unwrap_or_default()
}

/// Apply a freshly-loaded `ai.toml` to the service as one atomic
/// admission state.
///
/// A disabled daemon is given an effective Minimal scope, so the
/// configured tier is never installed while the AI layer is off; either
/// way `enabled` and the scope are published together via
/// [`AiDaemonService::set_admission`], so the query path samples a
/// consistent pair and never a torn one.
/// The security-bearing `ai.toml` keys this daemon owns and audits on change
/// (ai.toml-hardening piece 2): a flip of any of these changes what the AI
/// daemon is permitted, so it must become VISIBLE in the HMAC ledger, not only
/// a `tracing::info!`. `executor_live` / `autonomous_apps` / `action_mode` are
/// the ai-agent's keys, audited by its own watcher.
#[derive(Clone, PartialEq, Eq)]
struct WatchedKeys {
    enabled: bool,
    access_level: u8,
    provider: String,
}

/// Fetch the AI master switches from the config broker, or `None` when it is
/// unreachable.
///
/// `None` is the pre-cutover fallback: [`read_watched_keys`] then reads the
/// admission keys from `ai.toml` (today's behaviour), so a broker not yet
/// deployed does not break the daemon. The FINAL cutover replaces this fallback
/// with fail-closed once the broker is deployed in every launch path. Never
/// fatal. (A broker-only change is not picked up live by this ai.toml watcher -
/// the root-owned broker store is not watchable by the user-uid daemon - so it
/// applies on the next ai.toml event or restart, the same epoch-scoped liveness
/// the provider already has; a broker change-push is a follow-up.)
async fn broker_switches() -> Option<AiMasterSwitches> {
    match ConfigBrokerClient::default_socket().get().await {
        Ok(switches) => Some(switches),
        // Genuinely unreachable: fall back to ai.toml (pre-cutover) via None.
        Err(ClientError::Transport(e)) => {
            tracing::debug!(error = %e, "config broker unreachable; admission keys fall back to ai.toml");
            None
        }
        // Reachable but errored (corrupt store / unexpected refusal): do NOT
        // trust the user-writable ai.toml - a same-uid attacker could corrupt
        // the store to force that fallback. Fail CLOSED to the floor (disabled /
        // level 0), which `read_watched_keys(Some(..))` then applies.
        Err(e) => {
            tracing::warn!(error = %e, "config broker errored; failing admission closed (not ai.toml)");
            Some(AiMasterSwitches::default())
        }
    }
}

/// Read the security-bearing admission keys. `enabled` / `access_level` /
/// `provider` come from the config broker when present (the separate-uid owner
/// of a store the user's normal uid cannot write), and from `ai.toml` otherwise
/// (the pre-cutover fallback). The broker value is taken as-is: the broker
/// already clamps `access_level`, and `access_tier_from_level` clamps again
/// when it is applied.
fn read_watched_keys(cfg: &Config, broker: Option<&AiMasterSwitches>) -> WatchedKeys {
    match broker {
        Some(b) => WatchedKeys {
            enabled: b.enabled,
            access_level: b.access_level,
            provider: b.provider.clone(),
        },
        None => WatchedKeys {
            enabled: cfg.get::<bool>("ai.enabled").unwrap_or(false),
            access_level: read_access_level(cfg),
            provider: cfg.get::<String>("ai.provider").unwrap_or_default(),
        },
    }
}

/// The `(key, transition)` pairs that changed between two applies, for auditing.
/// Pure, so the diff is testable without the ledger. The provider transition
/// carries the values (the `config_change_event` builder strips controls and
/// bounds the length, so a hostile provider name is safe) - seeing a repoint
/// target is exactly the anti-Recall point.
fn config_diff(prev: &WatchedKeys, new: &WatchedKeys) -> Vec<(&'static str, String)> {
    let mut changes = Vec::new();
    if prev.enabled != new.enabled {
        changes.push(("enabled", format!("{}->{}", prev.enabled, new.enabled)));
    }
    if prev.access_level != new.access_level {
        changes.push((
            "access_level",
            format!("{}->{}", prev.access_level, new.access_level),
        ));
    }
    if prev.provider != new.provider {
        changes.push(("provider", format!("{}->{}", prev.provider, new.provider)));
    }
    changes
}

/// Emit a content-free audit entry for each changed security key. Best-effort:
/// a config change is an observation audited AFTER the fact, so a down ledger
/// logs a warning but never blocks the apply (unlike audit-before-act, which is
/// fail-closed) - the daemon must still honour the user's config when the ledger
/// is down; the residual unaudited-flip is the deferred same-uid boundary.
fn emit_config_changes(
    audit: &dyn AuditSink,
    handle: &tokio::runtime::Handle,
    prev: &WatchedKeys,
    new: &WatchedKeys,
) {
    for (key, change) in config_diff(prev, new) {
        if let Err(err) = handle.block_on(audit.submit(config_change_event(key, &change))) {
            tracing::warn!(
                key,
                error = %err,
                "ai.toml config-change audit failed (change applied; flip unaudited)"
            );
        }
    }
}

fn apply_config(service: &AiDaemonService, keys: &WatchedKeys) {
    let effective_level = if keys.enabled { keys.access_level } else { 0 };
    let tier = access_tier_from_level(effective_level);
    let scope = QueryScope::for_tier(tier, &GraphSchema::knowledge_graph());
    service.set_admission(keys.enabled, tier, scope);
    tracing::info!(
        enabled = keys.enabled,
        access_level = keys.access_level,
        "ai.toml applied"
    );
}

/// Spawn the `ai.toml` watch thread.
///
/// The watcher is the sole owner of the service's admission state: the
/// daemon is constructed fail-closed (disabled, no graph access) and
/// this thread publishes the configured admission only after the file
/// watch is armed, then keeps it live on every change. Because the
/// pre-publish state is fail-closed, there is no window where a stale
/// startup snapshot serves broad access before the watcher is live, and
/// a write that lands before the watch is armed is picked up by the
/// initial publish rather than missed.
///
/// Runs on a dedicated OS thread because [`os_sdk::config::ConfigWatcher`]
/// exposes a blocking `recv()`. The thread exits when the watcher is
/// dropped (process shutdown).
pub fn spawn_config_watch(service: Arc<AiDaemonService>, audit: Arc<dyn AuditSink>) {
    // Captured here, in the async runtime context, so the dedicated (non-runtime)
    // watch thread can `block_on` the audit submit; the watch thread is not
    // latency-critical and config changes are rare, so blocking it briefly on a
    // ledger write is fine.
    let handle = tokio::runtime::Handle::current();
    std::thread::Builder::new()
        .name("ai-config-watch".to_string())
        .spawn(move || {
            // On any load/watch setup failure the service stays in its
            // fail-closed startup state; recovery needs a daemon restart.
            // The live reload path below recovers on its own once the
            // watcher is running.
            let mut cfg = match Config::load("ai") {
                Ok(c) => c,
                Err(err) => {
                    tracing::warn!(error = %err, "ai.toml watch: load failed, failing closed");
                    fail_closed(&service);
                    return;
                }
            };
            let watcher = match cfg.watch() {
                Ok(w) => w,
                Err(err) => {
                    tracing::warn!(error = %err, "ai.toml watch: cannot watch, failing closed");
                    fail_closed(&service);
                    return;
                }
            };
            // Initial publish, now that the watch is armed. Reload first
            // so it reflects the on-disk file as of after registration,
            // closing the gap between the daemon's fail-closed startup
            // state and the first change event (a write before the watch
            // armed fires no event).
            // The last successfully-applied keys, for the change audit. `None`
            // until the initial publish; a fail-closed interlude (malformed
            // write) does NOT update it, so the diff spans valid-to-valid and a
            // transient parse error is not mis-audited as a deliberate flip.
            let mut prev: Option<WatchedKeys> = None;
            if let Err(err) = cfg.reload() {
                tracing::warn!(error = %err, "ai.toml initial reload failed, failing closed");
                fail_closed(&service);
            } else {
                let switches = handle.block_on(broker_switches());
                let keys = read_watched_keys(&cfg, switches.as_ref());
                apply_config(&service, &keys);
                // The initial publish is the baseline, not a change - no audit.
                prev = Some(keys);
            }
            tracing::info!("ai.toml watch active");
            while watcher.recv().is_ok() {
                if let Err(err) = cfg.reload() {
                    // A malformed or partially-written ai.toml must not
                    // leave a previously broad admission in place: we
                    // cannot trust the prior in-memory values. Fail closed
                    // and keep the watcher alive so a later valid rewrite
                    // recovers.
                    tracing::warn!(error = %err, "ai.toml reload failed, failing closed");
                    fail_closed(&service);
                    continue;
                }
                let switches = handle.block_on(broker_switches());
                let keys = read_watched_keys(&cfg, switches.as_ref());
                apply_config(&service, &keys);
                // Audit every security-key flip against the last valid apply, so
                // a silent change to enabled / access_level / provider becomes
                // visible in the ledger (ai.toml-hardening piece 2).
                if let Some(prev_keys) = prev.as_ref() {
                    emit_config_changes(&*audit, &handle, prev_keys, &keys);
                }
                prev = Some(keys);
            }
            tracing::info!("ai.toml watch stopped");
        })
        .expect("spawn ai-config-watch thread");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    /// Parse a provider config from an `ai.toml` body, exercising the same
    /// dot-notation reads `load_ai_settings` uses (via a temp file, since the
    /// loader reads from disk).
    fn provider_from(toml: &str) -> ProviderSettings {
        let mut file = NamedTempFile::new().unwrap();
        write!(file, "{toml}").unwrap();
        let cfg = Config::load_path(file.path()).unwrap();
        read_provider(&cfg)
    }

    #[test]
    fn config_diff_reports_only_changed_security_keys() {
        let base = WatchedKeys {
            enabled: false,
            access_level: 3,
            provider: "ollama-default".to_string(),
        };
        // No change -> nothing audited.
        assert!(config_diff(&base, &base.clone()).is_empty());
        // A single flip per key, each with its transition.
        let enabled_on = WatchedKeys { enabled: true, ..base.clone() };
        assert_eq!(config_diff(&base, &enabled_on), vec![("enabled", "false->true".to_string())]);
        let widened = WatchedKeys { access_level: 4, ..base.clone() };
        assert_eq!(config_diff(&base, &widened), vec![("access_level", "3->4".to_string())]);
        let repointed = WatchedKeys { provider: "evil-endpoint".to_string(), ..base.clone() };
        assert_eq!(
            config_diff(&base, &repointed),
            vec![("provider", "ollama-default->evil-endpoint".to_string())],
            "a provider repoint is audited with its target (the builder bounds it)"
        );
        // Several at once -> all reported.
        let all = WatchedKeys { enabled: true, access_level: 4, provider: "x".to_string() };
        assert_eq!(config_diff(&base, &all).len(), 3);
    }

    fn config_from(toml: &str) -> Config {
        let mut file = NamedTempFile::new().unwrap();
        write!(file, "{toml}").unwrap();
        // Leak the temp file so the path stays valid for the Config's lifetime
        // in the test (the loader reads it eagerly, but keep_temp is simplest).
        let (_f, path) = file.keep().unwrap();
        Config::load_path(&path).unwrap()
    }

    #[test]
    fn read_watched_keys_without_broker_reads_the_file() {
        let cfg = config_from("[ai]\nenabled = true\naccess_level = 2\nprovider = \"ollama-default\"\n");
        let keys = read_watched_keys(&cfg, None);
        assert!(keys.enabled);
        assert_eq!(keys.access_level, 2);
        assert_eq!(keys.provider, "ollama-default");
    }

    #[test]
    fn read_watched_keys_with_broker_overrides_the_file() {
        // The file says off / minimal / a different provider; the broker is
        // authoritative, so a same-uid rewrite of ai.toml cannot widen the
        // daemon's admission.
        let cfg = config_from("[ai]\nenabled = false\naccess_level = 0\nprovider = \"file-provider\"\n");
        let switches = AiMasterSwitches {
            enabled: true,
            access_level: 4,
            executor_live: false,
            action_mode: arlen_config_broker::ActionMode::Suggest,
            provider: "broker-provider".to_string(),
            autonomous_apps: Default::default(),
        };
        let keys = read_watched_keys(&cfg, Some(&switches));
        assert!(keys.enabled, "broker enables even though the file says off");
        assert_eq!(keys.access_level, 4, "broker access_level wins over the file");
        assert_eq!(keys.provider, "broker-provider", "broker provider wins over the file");
    }

    #[test]
    fn a_name_only_config_uses_the_default_model_window_and_token() {
        // The shape Settings writes: [ai] enabled + provider, no [provider].
        let p = provider_from("[ai]\nenabled = true\nprovider = \"ollama-default\"\n");
        assert_eq!(p.name, "ollama-default");
        assert_eq!(p.model, DEFAULT_MODEL);
        assert_eq!(p.context_window, DEFAULT_CONTEXT_WINDOW);
        assert_eq!(p.audit_token, DEFAULT_AUDIT_TOKEN);
    }

    #[test]
    fn a_provider_section_overrides_the_model_window_and_token() {
        let p = provider_from(
            "[ai]\nenabled = true\nprovider = \"my-cloud\"\n\n\
             [provider]\nmodel = \"claude-opus-4-8\"\ncontext_window = 200000\naudit_token = \"tok-123\"\n",
        );
        assert_eq!(p.name, "my-cloud");
        assert_eq!(p.model, "claude-opus-4-8");
        assert_eq!(p.context_window, 200_000);
        assert_eq!(p.audit_token, "tok-123");
    }

    #[test]
    fn an_absent_provider_falls_back_to_the_default_local_backend() {
        // An unconfigured query daemon needs a backend, so an *absent*
        // ai.provider defaults to the local backend (the value Settings
        // writes). This keeps a daemon that started before the config existed
        // from latching an empty provider, since the provider is read once at
        // startup.
        for toml in ["", "[ai]\nenabled = true\n"] {
            assert_eq!(provider_from(toml).name, DEFAULT_PROVIDER, "toml: {toml:?}");
        }
    }

    #[test]
    fn a_present_but_blank_provider_fails_closed_instead_of_defaulting() {
        // A *present* but blank provider is a misconfiguration: it fails closed
        // (the proxy rejects an empty name), rather than silently routing to a
        // backend. Defaulting it would let a cleared Settings field re-enable
        // forwarding. This is the case absent-key handling deliberately differs
        // from.
        let p = provider_from("[ai]\nenabled = true\nprovider = \"\"\n");
        assert_eq!(p.name, "");
    }

    #[test]
    fn a_present_wrong_typed_provider_fails_closed_and_fields_default() {
        // A present but wrong-typed provider name fails closed (no forwarding);
        // wrong-typed within-provider fields fall back to safe defaults rather
        // than reaching the backend malformed.
        let p = provider_from(
            "[ai]\nprovider = 123\n\n[provider]\nmodel = 5\ncontext_window = \"big\"\naudit_token = true\n",
        );
        assert_eq!(p.name, "");
        assert_eq!(p.model, DEFAULT_MODEL);
        assert_eq!(p.context_window, DEFAULT_CONTEXT_WINDOW);
        assert_eq!(p.audit_token, DEFAULT_AUDIT_TOKEN);
    }

    #[test]
    fn a_non_positive_or_oversized_context_window_falls_back_to_the_default() {
        for toml in [
            "[provider]\ncontext_window = 0\n",
            "[provider]\ncontext_window = -5\n",
            "[provider]\ncontext_window = 9999999999\n",
        ] {
            assert_eq!(
                provider_from(toml).context_window,
                DEFAULT_CONTEXT_WINDOW,
                "toml: {toml:?}"
            );
        }
    }

    #[test]
    fn a_blank_model_or_token_does_not_blank_what_reaches_the_proxy() {
        // An empty string in the file is treated as absent, not a literal
        // empty value that would reach the proxy.
        let p = provider_from("[provider]\nmodel = \"\"\naudit_token = \"\"\n");
        assert_eq!(p.model, DEFAULT_MODEL);
        assert_eq!(p.audit_token, DEFAULT_AUDIT_TOKEN);
    }
}
