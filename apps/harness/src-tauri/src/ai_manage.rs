//! Harness Tauri bridges to the AI daemons: the cost feed, the provider
//! catalogue, the working set, the assistant's grants, the action state and the
//! undo list. Each opens the session bus, calls one member and returns its JSON.
//! A read that did not happen answers `null`, never an empty value a surface
//! would draw as a measurement.
//!
//! Reads only. The switches that decide what the assistant does (mode, provider,
//! autonomy) belong to the config-broker, which admits Settings as its one
//! writer, so this app sends a person there (`open_ai_settings`).

use zbus::{Connection, Proxy};

/// The agent interface: grants, working set, action state and the undo list.
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
/// call answers.
#[tauri::command]
pub async fn ai_providers_list() -> String {
    try_call_string(PROXY_BUS, PROXY_PATH, "list_providers")
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
/// **It returns the shape, not the text of the shape**, and that distinction was a
/// live bug until 13 September. This used to be `-> String`, the drawer does
/// `invoke<WorkingSet>` with no parse, and a scalar arrives as a scalar - so every
/// field the Memory section read came back undefined and the section rendered
/// nothing, silently, on the surface whose whole job is to be believable. The
/// doc's own claim that an unreachable agent reads as "not available yet" was part
/// of the same bug: what the drawer received was the four-character string
/// `"null"`, which is truthy, so that branch was never taken.
///
/// The other half was that the two ends described different things - the engine
/// served `{status, behaviours[]}`, the drawer declares
/// `{available, held, entityCounts, activeBehaviour, declaredReads}` - so parsing
/// here alone would only have moved the failure one step later. The engine answers
/// the drawer's shape now (`daemons/ai-engine-daemon/src/agent_iface.rs`), which is
/// the specified one, and this passes it through as JSON.
///
/// An unreachable agent answers `available: false` rather than a null: the drawer
/// defines that field as "the endpoint answered", so the honest wire value for a
/// daemon that did not answer is the one that says so.
#[tauri::command]
pub async fn ai_working_set() -> serde_json::Value {
    let unavailable = || serde_json::json!({ "available": false });
    let Some(text) = try_call_string(AGENT_BUS, AGENT_PATH, "working_set").await else {
        return unavailable();
    };
    serde_json::from_str(&text).unwrap_or_else(|_| unavailable())
}

/// What the assistant may read, for the transparency drawer (`access_grants` on
/// the agent interface): a JSON array of grant views, or `null` when the read did
/// not happen.
///
/// The engine answers for itself. The knowledge daemon scopes a grant read to the
/// attested caller, so this app asking directly would be shown its own grants;
/// the engine asks instead and passes the list on. One read, because one daemon
/// owns both AI bus names under one app id, and asking both would draw every
/// grant twice.
///
/// `null` and not `[]` on any failure: an empty list says the assistant reaches
/// nothing, which is a statement only a successful read may make.
#[tauri::command]
pub async fn ai_access_grants() -> serde_json::Value {
    let Some(json) = try_call_string(AGENT_BUS, AGENT_PATH, "access_grants").await else {
        return serde_json::Value::Null;
    };
    match serde_json::from_str::<serde_json::Value>(&json) {
        Ok(list @ serde_json::Value::Array(_)) => list,
        _ => serde_json::Value::Null,
    }
}

/// The autonomy-dial state (`action_state` on the agent): `{ action_mode,
/// executor_live }`. `null` when the agent is unreachable.
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
