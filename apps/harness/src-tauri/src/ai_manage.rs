//! Harness Tauri bridges for the AI provider/model picker, the cost feed, and
//! the autonomy-dial state (arlen-ui flagged these as the missing coder-lane
//! `#[tauri::command]` wrappers: the daemon D-Bus members exist on
//! `org.arlen.AI1` / `org.arlen.AIAgent1`, but the frontend `invoke` had nothing
//! to call). Each is a thin wrapper: open the session bus, call the member,
//! return its JSON string. Reads are advisory - an unreachable daemon yields a
//! fail-safe empty value rather than erroring the UI; the one mutating call
//! (`ai_set_active`) surfaces a real failure so the picker can report it.

use zbus::{Connection, Proxy};

/// The AI daemon: provider/model picker + cost.
const AI_BUS: &str = "org.arlen.AI1";
const AI_PATH: &str = "/org/arlen/AI1";
/// The AI agent: the autonomy-dial state.
const AGENT_BUS: &str = "org.arlen.AIAgent1";
const AGENT_PATH: &str = "/org/arlen/AIAgent1";
/// The egress proxy: the measured token usage lives here (the ledger that meters
/// every forward), exposed un-gated as read-only display data for exactly this
/// transparency feed.
const PROXY_BUS: &str = "org.arlen.AIProxy1";
const PROXY_PATH: &str = "/org/arlen/AIProxy1";

/// Call a String-returning member on `(bus, path, bus)`, returning `None` on any
/// connection or call failure rather than substituting a value.
///
/// Needed wherever an empty result would be read as a fact about the system
/// rather than as "could not read" - a grant list being the clear case, since an
/// empty one states "nothing has access".
async fn try_call_string(bus: &str, path: &str, member: &str) -> Option<String> {
    let connection = Connection::session().await.ok()?;
    let proxy = Proxy::new(&connection, bus, path, bus).await.ok()?;
    proxy.call(member, &()).await.ok()
}

/// The model catalog for the in-chat picker (`ai_models_list`): a JSON array of
/// `{ provider, model, contextWindow, kind, available }`, or `null` when the
/// daemon could not be reached.
///
/// This was the last command in the file substituting a value for a failure: it
/// answered `"[]"` when the call did not happen, and the picker hides itself on
/// an empty catalogue, so a daemon that was down and a daemon with no models
/// configured produced the same missing bar. The `call_string` helper that
/// performed the substitution had no other callers and is gone with it.
///
/// The user is still told, and by the neighbour rather than here: `CapabilityBar`
/// reads the same daemon and prints `h.capability.unreachable` with a retry when
/// its own read comes back null, one sentence below where this bar would be. A
/// second copy of it here would say the same thing twice about one outage.
#[tauri::command]
pub async fn ai_models_list() -> Option<String> {
    try_call_string(AI_BUS, AI_PATH, "ai_models_list").await
}

/// The current live selection (`ai_active`): `{ provider, model }`.
///
/// The active selection IS the config: the engine reads `ai.toml`'s `[ai].provider`
/// and `[provider].model` per epoch, so that file is the source of truth, not a
/// daemon (nothing serves `ai_active` on `org.arlen.AI1`). Reuses the same
/// extraction the capability indicator uses, so the picker and the indicator can
/// never disagree about what is active. Empty object if the config is unreadable
/// or names neither - the picker renders "loading/none" rather than a wrong model.
#[tauri::command]
pub async fn ai_active() -> String {
    let Ok(text) = std::fs::read_to_string(crate::capability::ai_config_path()) else {
        return "{}".to_string();
    };
    let Ok(doc) = text.parse::<toml::Table>() else {
        return "{}".to_string();
    };
    active_selection_json(&doc)
}

/// Format the active `{ provider, model }` from a parsed `ai.toml`. `{}` unless
/// BOTH are present: a half-configured selection (one key) is not a usable active
/// model, so the picker reads none rather than a partial it cannot act on. Pure,
/// so the both-vs-partial branching is tested without touching the filesystem.
fn active_selection_json(doc: &toml::Table) -> String {
    match crate::capability::provider_and_model(doc) {
        (Some(p), Some(m)) => serde_json::json!({ "provider": p, "model": m }).to_string(),
        _ => "{}".to_string(),
    }
}

/// Sum ai-proxy's per-provider usage report into the Cost feed's shape.
///
/// The proxy meters every forward per provider over the current window; the Cost
/// section wants one cumulative `{ inputTokens, outputTokens, totalTokens }`, so
/// this folds the providers together. `None` on any parse failure, which the
/// caller maps to `null` - the honest "not measured", never a fabricated zero.
/// Genuinely-zero usage (the ledger is reachable but nothing has been spent) is a
/// real measured value and returns `{...: 0}`, distinct from `None`.
fn sum_usage(report_json: &str) -> Option<String> {
    let report: serde_json::Value = serde_json::from_str(report_json).ok()?;
    let providers = report.get("providers")?.as_array()?;
    let (mut input, mut output, mut total) = (0u64, 0u64, 0u64);
    for p in providers {
        let usage = p.get("usage")?;
        input += usage.get("promptTokens").and_then(|v| v.as_u64()).unwrap_or(0);
        output += usage.get("completionTokens").and_then(|v| v.as_u64()).unwrap_or(0);
        total += usage.get("totalTokens").and_then(|v| v.as_u64()).unwrap_or(0);
    }
    Some(
        serde_json::json!({
            "inputTokens": input,
            "outputTokens": output,
            "totalTokens": total,
        })
        .to_string(),
    )
}

/// Cumulative token usage (`ai_usage`): `{ inputTokens, outputTokens,
/// totalTokens }` for the transparency-drawer Cost feed.
///
/// Reads the MEASURED usage from ai-proxy's `list_provider_usage` (the ledger
/// that meters every forward) and folds the providers into one total. Nothing
/// serves this on `org.arlen.AI1`, which is why the feed read "not measured"
/// despite the data existing one hop away on the proxy.
///
/// Unreachable or unparseable yields `null`, NOT zeros. This is the transparency
/// surface, so "0 tokens used so far" must mean measured-and-zero; reporting zeros
/// for an unreadable proxy states as fact that the assistant cost nothing. The
/// drawer already renders a "not measured" tag for a null usage - fabricating
/// zeros here is what made that branch unreachable.
#[tauri::command]
pub async fn ai_usage() -> String {
    match try_call_string(PROXY_BUS, PROXY_PATH, "list_provider_usage").await {
        Some(report) => sum_usage(&report).unwrap_or_else(|| "null".to_string()),
        None => "null".to_string(),
    }
}

/// The catalogued providers for the manager surface: the sovereignty-annotated
/// provider list, `null` when it cannot be read (an empty catalogue reads as
/// "you have no providers configured", which is a statement about the user's
/// setup that nothing measured).
///
/// DIALS THE PROXY, not the AI daemon. This asked `org.arlen.AI1` for a member
/// called `ai_providers_list`, and that bus serves `ask` and `explain_system` and
/// nothing else - so the call had never once succeeded. The catalogue lives on
/// `org.arlen.AIProxy1` as `list_providers`, which is where Settings has been
/// reading it from all along, and where this command already goes for
/// `list_provider_usage` two functions up. Nothing was wired to it yet, so
/// nothing breaks.
///
/// ai-proxy authenticates PER METHOD, not per interface, and that is worth
/// stating because I got it wrong in both directions before reading it. Its
/// `forward_completion` and `test_provider` resolve the caller against an
/// executable allowlist of three daemons; `list_providers` and
/// `list_provider_usage` take no header and authenticate nobody, because they
/// are read-only display data with no endpoint or credential in them. So this
/// call answers, and `ai_provider_test` two functions down is the one a surface
/// cannot reach.
#[tauri::command]
pub async fn ai_providers_list() -> String {
    try_call_string(PROXY_BUS, PROXY_PATH, "list_providers")
        .await
        .unwrap_or_else(|| "null".to_string())
}

/// The configured default provider/model for the manager's Default-Models page
/// (`ai_defaults_get`): `{ provider, model, ranking }`. `null` when the daemon is
/// unreachable, rather than an empty object that a page would render as "no
/// default set". Same standing as the provider catalogue above: no harness
/// surface reads it yet.
#[tauri::command]
pub async fn ai_defaults_get() -> String {
    try_call_string(AI_BUS, AI_PATH, "ai_defaults_get")
        .await
        .unwrap_or_else(|| "null".to_string())
}

/// The agent's pending gate proposals (`pending_proposals`): a JSON array the
/// harness renders as inline gate cards (each `{ id, summary, reason, effects }`),
/// oldest first. `null` when the agent is unreachable.
///
/// Not `[]`: an empty list is an answer ("nothing is waiting on you"), and a
/// silent gate feed is precisely what a person reads as "nothing needs me".
/// The store already distinguishes them - it holds null for unread, not empty -
/// so the honest token is what makes that branch reachable.
#[tauri::command]
pub async fn pending_proposals() -> String {
    try_call_string(AGENT_BUS, AGENT_PATH, "pending_proposals")
        .await
        .unwrap_or_else(|| "null".to_string())
}

/// The agent's recently-completed (silent-done) actions (`completed_actions`): a
/// JSON array the harness renders as quiet done-lines each with an `[Undo]`,
/// oldest first. Each entry carries the correlation id the `compensate` undo
/// keys off. `null` when the agent is unreachable, `[]` only when it answered
/// and nothing has executed - the same distinction the pending feed draws.
#[tauri::command]
pub async fn completed_actions() -> String {
    try_call_string(AGENT_BUS, AGENT_PATH, "completed_actions")
        .await
        .unwrap_or_else(|| "null".to_string())
}

/// Dismiss a pending gate proposal (`deny`): the user declined the confirmation.
/// Returns the agent's `denied` / `no-such-proposal` / `error: ...` status; a
/// transport failure maps to an `error:` string so the gate card surfaces it.
/// Deny is purely local and safe in any mode (it forgoes an action), so it is
/// always available.
#[tauri::command]
pub async fn deny(id: u64) -> String {
    let Ok(connection) = Connection::session().await else {
        return "error: session bus unavailable".to_string();
    };
    let Ok(proxy) = Proxy::new(&connection, AGENT_BUS, AGENT_PATH, AGENT_BUS).await else {
        return "error: AI agent unavailable".to_string();
    };
    proxy
        .call("deny", &(id,))
        .await
        .unwrap_or_else(|e| format!("error: {e}"))
}

/// Approve a pending gate-card proposal (`approve`): the user confirmed the
/// action, so the agent performs it. The agent re-runs the full trusted proof
/// against the current graph and audits fail-closed before the write, so the
/// approve authorises the act but never bypasses revalidation. Returns the
/// agent's status (`executed` / `nothing-to-execute` / `not-enabled` in suggest
/// mode / `no-such-proposal` / `error: ...`); a transport failure maps to an
/// `error:` string the gate card surfaces.
#[tauri::command]
pub async fn approve(id: u64) -> String {
    let Ok(connection) = Connection::session().await else {
        return "error: session bus unavailable".to_string();
    };
    let Ok(proxy) = Proxy::new(&connection, AGENT_BUS, AGENT_PATH, AGENT_BUS).await else {
        return "error: AI agent unavailable".to_string();
    };
    proxy
        .call("approve", &(id,))
        .await
        .unwrap_or_else(|e| format!("error: {e}"))
}

/// Undo a completed action (`compensate`): the user pressed `[Undo]` on a
/// silent-done line, keyed by the action's correlation id (the `id` on a
/// `completed_actions` entry). The agent retracts the write, re-running the
/// audit fail-closed first. Returns the agent's status (`retracted` /
/// `nothing-to-undo` / `no-such-receipt` / `not-enabled` / `error: ...`); a
/// transport failure maps to an `error:` string. Only functions when the
/// executor is live; in suggest mode nothing was written, so the agent answers
/// `not-enabled`.
#[tauri::command]
pub async fn undo_action(id: String) -> String {
    let Ok(connection) = Connection::session().await else {
        return "error: session bus unavailable".to_string();
    };
    let Ok(proxy) = Proxy::new(&connection, AGENT_BUS, AGENT_PATH, AGENT_BUS).await else {
        return "error: AI agent unavailable".to_string();
    };
    proxy
        .call("compensate", &(id,))
        .await
        .unwrap_or_else(|e| format!("error: {e}"))
}

/// The agent's working-set shape (`working_set` on the agent): the shape-only
/// introspection of what the agent currently has in scope (AIT-R1), for the
/// transparency drawer's working-set section. Identity/shape only, never user
/// data.
///
/// TWO THINGS THIS DOES NOT DO, both of which the doc used to claim.
///
/// It said `null` on an unreachable agent is read by the drawer as the "not
/// available yet" state. The drawer does `invoke<WorkingSet>` with no parse, so
/// what it receives is the four-character STRING `"null"`, which is truthy, and
/// that branch is never taken. The value here is right; nothing reads it.
///
/// And the shapes disagree underneath that. The engine serves
/// `{status, behaviours[]}` (`daemons/ai-engine-daemon/src/agent_iface.rs`), the
/// drawer declares `{available, held, entityCounts, activeBehaviour,
/// declaredReads}`. Parsing the string here would only move the failure one step
/// later, so it is deliberately NOT parsed until somebody decides which shape is
/// the real one - a call between the drawer and the engine, reported to the
/// planner rather than settled here. `check-invoke-shape` carries both as routed
/// findings so neither can be quietly forgotten.
#[tauri::command]
pub async fn ai_working_set() -> String {
    try_call_string(AGENT_BUS, AGENT_PATH, "working_set")
        .await
        .unwrap_or_else(|| "null".to_string())
}

/// The AI's capability grants for the transparency drawer's Grants feed
/// (`access_grants` on both AI principals): the Living Capability Graph
/// projection of what the assistant (`org.arlen.AI1`) and the background agent
/// (`org.arlen.AIAgent1`) are each allowed to read. Each daemon reports its OWN
/// grants - the knowledge daemon's `access_grants` op is caller-scoped, so the
/// principal is correct by construction - and this merges the two into the one
/// AI-scoped array `readGrants()` renders, each labelled by its `app_id`. A
/// daemon that is unreachable or holds no grant contributes nothing, so a
/// partial view is honest rather than an error. Returns a JSON array (the
/// frontend invokes it as `GrantView[]`); empty when neither principal answers.
///
/// THE TWO-PRINCIPAL PREMISE IS EXPIRING, and it matters before this member is
/// ever served. `daemons/ai-engine-daemon` now owns BOTH names on ONE connection
/// - it is the drop-in for the retired ai-daemon and ai-agent alike, and the
/// identity resolver maps its binary to the single app id `ai-agent` on the
/// principle that an app id is a role, not a binary name. The knowledge daemon
/// scopes `access_grants` by the attested peer, so once the transition finishes
/// both names answer with the SAME grant set and this loop renders every grant
/// twice.
///
/// It is still two principals DURING the transition: the engine's request for
/// `org.arlen.AIAgent1` is graceful and fails while the old agent still owns the
/// name. So collapsing this to one read now would be wrong on a machine that has
/// not finished the cutover, and leaving it is wrong on one that has. Whoever
/// serves the member should decide which world this app is written for, rather
/// than discovering the doubling on screen.
#[tauri::command]
pub async fn ai_access_grants() -> serde_json::Value {
    // Null - NOT an empty array - if EITHER principal cannot be read. The reader
    // (`transparency.ts::readGrants`) documents that a failed read must render
    // honestly and "never as 'no access'", but an `[]` fallback per principal
    // defeated that: an unreachable daemon contributed nothing and the merged
    // result came back as a successful empty list, i.e. "nothing has access".
    // A PARTIAL merge is wrong for the same reason - it under-reports reach while
    // looking complete - so any failure yields unknown rather than a short list.
    let mut grants: Vec<serde_json::Value> = Vec::new();
    for (bus, path) in [(AGENT_BUS, AGENT_PATH), (AI_BUS, AI_PATH)] {
        let Some(json) = try_call_string(bus, path, "access_grants").await else {
            return serde_json::Value::Null;
        };
        let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(&json)
        else {
            return serde_json::Value::Null;
        };
        grants.extend(items);
    }
    serde_json::Value::Array(grants)
}

/// The autonomy-dial state (`action_state` on the agent): `{ action_mode,
/// autonomous_apps, executor_live }`. `null` when the agent is unreachable.
///
/// It used to answer with an inert shape (suggest / none / off) instead. That
/// shape is not safe, it is a reading: the dial is a CONTROL, so it renders the
/// substituted values as the machine's current position, and a person adjusting
/// autonomy would have been told the assistant only suggests without anything
/// having asked it. The dial hides itself for a null state, which is the true
/// answer when nothing was measured.
#[tauri::command]
pub async fn action_state() -> String {
    try_call_string(AGENT_BUS, AGENT_PATH, "action_state")
        .await
        .unwrap_or_else(|| "null".to_string())
}

/// A MASTER SWITCH, AND THIS IS NOT THE WAY TO ONE. The three commands below
/// (`ai_set_action_mode`, `ai_set_autonomous_app`, `ai_set_active`) each dial a
/// member on the agent bus that no interface serves, so none of them has ever
/// done anything, and their docs described the answer they would have parsed.
///
/// They are also not members to add. `action_mode`, `autonomous_apps` and
/// `provider` are three fields of `AiMasterSwitches`, owned by the config-broker
/// from a separate uid precisely so a same-uid process cannot flip them - its
/// module doc names repointing the provider and granting autonomy as the things
/// it exists to stop. And its `ADMITTED_WRITERS` is one entry long:
/// `dev.arlen.settings`. This app may read the switches and may not write them,
/// by design rather than by omission.
///
/// So the honest route for all three is the one this app already uses for the
/// `[ai] enabled` switch: `openAiSettings()` in `transparency.ts`, which opens
/// Settings at its AI section. Settings routes the write through the broker in
/// `commands/config.rs`, which is the only place that write is allowed to
/// happen. Whether these dials belong in this app at all is the question to
/// settle before anything is rebuilt behind them.

/// Set the baseline autonomy mode: `"suggest"` or `"supervised"`. Dead, and see
/// the note above for where this write actually belongs.
#[tauri::command]
pub async fn ai_set_action_mode(mode: String) -> String {
    let Ok(connection) = Connection::session().await else {
        return "error: session bus unavailable".to_string();
    };
    let Ok(proxy) = Proxy::new(&connection, AGENT_BUS, AGENT_PATH, AGENT_BUS).await else {
        return "error: AI agent unavailable".to_string();
    };
    proxy
        .call("ai_set_action_mode", &(mode.as_str(),))
        .await
        .unwrap_or_else(|e| format!("error: {e}"))
}

/// Grant or revoke an app's autonomy: add/remove `app_id` from
/// `AiMasterSwitches.autonomous_apps`. Dead, and broker-owned; see the note on
/// `ai_set_action_mode`.
#[tauri::command]
pub async fn ai_set_autonomous_app(app_id: String, enabled: bool) -> String {
    let Ok(connection) = Connection::session().await else {
        return "error: session bus unavailable".to_string();
    };
    let Ok(proxy) = Proxy::new(&connection, AGENT_BUS, AGENT_PATH, AGENT_BUS).await else {
        return "error: AI agent unavailable".to_string();
    };
    proxy
        .call("ai_set_autonomous_app", &(app_id.as_str(), enabled))
        .await
        .unwrap_or_else(|e| format!("error: {e}"))
}

/// Live-swap the active provider+model. Dead, and broker-owned; see the note on
/// `ai_set_action_mode`.
///
/// This one has a second half worth recording. `provider` is an
/// `AiMasterSwitches` field, but the MODEL is not - it is an ordinary `ai.toml`
/// key. So even from Settings, which is the one admitted writer, a swap is two
/// writes down two different paths: the provider through the broker, the model
/// to the file. Whoever rebuilds this should know that before starting, because
/// a half-applied swap is a machine pointed at a provider that does not serve
/// the model beside it.
#[tauri::command]
pub async fn ai_set_active(provider: String, model: String) -> Result<String, String> {
    let connection = Connection::session()
        .await
        .map_err(|e| format!("session bus unavailable: {e}"))?;
    let proxy = Proxy::new(&connection, AI_BUS, AI_PATH, AI_BUS)
        .await
        .map_err(|e| format!("AI daemon unavailable: {e}"))?;
    proxy
        .call("ai_set_active", &(provider.as_str(), model.as_str()))
        .await
        .map_err(|e| e.to_string())
}

/// Enable or disable a catalogued provider (`ai_provider_set_enabled`). Returns
/// the daemon's `ok` / `error: ...` status string; a transport failure maps to
/// an `error:` string so the manager surfaces it.
#[tauri::command]
pub async fn ai_provider_set_enabled(id: String, enabled: bool) -> String {
    let Ok(connection) = Connection::session().await else {
        return "error: session bus unavailable".to_string();
    };
    let Ok(proxy) = Proxy::new(&connection, AI_BUS, AI_PATH, AI_BUS).await else {
        return "error: AI daemon unavailable".to_string();
    };
    proxy
        .call("ai_provider_set_enabled", &(id.as_str(), enabled))
        .await
        .unwrap_or_else(|e| format!("error: {e}"))
}

/// Test a catalogued provider's connectivity (`ai_provider_test`). Returns the
/// daemon's verdict JSON `{ ok, httpStatus?, network? }`; the daemon GETs the
/// provider's catalogued model-list endpoint through the proxy (no caller URL).
/// A transport failure maps to a `network` verdict so the manager gets the
/// uniform shape rather than an error.
#[tauri::command]
pub async fn ai_provider_test(id: String) -> String {
    let network = |reason: &str| format!(r#"{{"ok":false,"network":"{reason}"}}"#);
    let Ok(connection) = Connection::session().await else {
        return network("session bus unavailable");
    };
    let Ok(proxy) = Proxy::new(&connection, AI_BUS, AI_PATH, AI_BUS).await else {
        return network("AI daemon unavailable");
    };
    proxy
        .call("ai_provider_test", &(id.as_str(),))
        .await
        .unwrap_or_else(|_| network("test failed"))
}

/// Open the Settings app to the AI panel (the transparency off-switch's "manage
/// AI in Settings" link). Launches `arlen-settings --panel ai`, the deep-link
/// Settings parses at startup to land on its AI page. Errors if the binary can
/// not be spawned (not installed / not on PATH).
#[tauri::command]
pub fn open_ai_settings() -> Result<(), String> {
    std::process::Command::new("arlen-settings")
        .args(["--panel", "ai"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| {
            // Settings ships on the image as of 27 August, so "no such file" is
            // now the unusual answer rather than the ordinary one - it means a
            // machine without it, not the machine we build. The sentence stays:
            // it is still what a person needs to read, and "launch settings: No
            // such file or directory (os error 2)" would read as a launch worth
            // retrying.
            if e.kind() == std::io::ErrorKind::NotFound {
                "Settings is not installed on this machine, so it cannot be opened from here"
                    .to_string()
            } else {
                format!("Settings could not be opened: {e}")
            }
        })
}

#[cfg(test)]
mod tests {
    use super::{active_selection_json, sum_usage};

    #[test]
    fn folds_providers_into_one_total() {
        // The proxy meters per provider over a window; the Cost feed wants one
        // cumulative figure, so two providers sum.
        let report = r#"{
            "windowResetsInSecs": 3600,
            "providers": [
                {"id":"ollama","usage":{"promptTokens":100,"completionTokens":40,"totalTokens":140,"requests":3},"cap":null},
                {"id":"openai","usage":{"promptTokens":10,"completionTokens":5,"totalTokens":15,"requests":1},"cap":1000}
            ]
        }"#;
        let out: serde_json::Value = serde_json::from_str(&sum_usage(report).unwrap()).unwrap();
        assert_eq!(out["inputTokens"], 110);
        assert_eq!(out["outputTokens"], 45);
        assert_eq!(out["totalTokens"], 155);
    }

    #[test]
    fn measured_zero_is_a_real_value_not_none() {
        // A reachable ledger with no spend is measured-and-zero - distinct from an
        // unreadable proxy (None -> the caller's null -> "not measured"). Reporting
        // this as null would hide that the assistant genuinely cost nothing yet.
        let report = r#"{"windowResetsInSecs":3600,"providers":[]}"#;
        let out: serde_json::Value = serde_json::from_str(&sum_usage(report).unwrap()).unwrap();
        assert_eq!(out["totalTokens"], 0);
        assert_eq!(out["inputTokens"], 0);
    }

    #[test]
    fn active_selection_needs_both_provider_and_model() {
        // The live ai.toml shape: [ai].provider + [provider].model.
        let doc = "[ai]\nprovider = \"ollama-default\"\n[provider]\nmodel = \"qwen2.5:7b\"\n"
            .parse::<toml::Table>()
            .unwrap();
        let out: serde_json::Value = serde_json::from_str(&active_selection_json(&doc)).unwrap();
        assert_eq!(out["provider"], "ollama-default");
        assert_eq!(out["model"], "qwen2.5:7b");
    }

    #[test]
    fn a_half_configured_selection_reads_as_none() {
        // Provider set but no model (or vice versa) is not something the picker can
        // act on, so it is {} - never a partial the UI would show as active.
        let provider_only = "[ai]\nprovider = \"ollama-default\"\n".parse::<toml::Table>().unwrap();
        assert_eq!(active_selection_json(&provider_only), "{}");
        let model_only = "[provider]\nmodel = \"qwen2.5:7b\"\n".parse::<toml::Table>().unwrap();
        assert_eq!(active_selection_json(&model_only), "{}");
        let empty = "[ai]\nenabled = true\n".parse::<toml::Table>().unwrap();
        assert_eq!(active_selection_json(&empty), "{}");
    }

    #[test]
    #[test]
    fn malformed_report_is_none_so_the_feed_says_not_measured() {
        // The honesty rule: an unparseable report must not become a fabricated
        // zero. None flows to the caller's null, which the drawer renders as "not
        // measured" rather than "0 tokens used so far".
        assert!(sum_usage("not json").is_none());
        assert!(sum_usage("{}").is_none()); // no providers array
        assert!(sum_usage(r#"{"providers":"x"}"#).is_none());
    }
}
