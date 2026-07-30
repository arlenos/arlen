//! SX-5: the one list of everything that extends the system.
//!
//! Apps, modules and bridges answered separately produce three surfaces that
//! each look complete and none of which is. This is the single query behind
//! the unified view: the shared inventory read from disk, with the module
//! runtime's live answer laid over the modules it knows about.
//!
//! The capability labels come from `arlen-extensions`, which is also what the
//! store uses, so a filter for "things that can reach the internet" means the
//! same thing on both surfaces. That is the whole reason the vocabulary is
//! shared rather than written per view.

use arlen_extensions::inventory::{self, InventoryRoots, LiveModule};
use arlen_extensions::Extension;
use modulesd_proto::{client, Request, Response};

/// Everything installed, with what it was granted and what it is doing.
///
/// The disk read always succeeds - an absent source means that kind is simply
/// empty. The runtime overlay is best-effort on top: if modulesd is not
/// running, modules keep the `unknown` health the disk read gave them rather
/// than the whole call failing. A management surface that goes blank exactly
/// when a daemon is down fails at the moment it is needed.
#[tauri::command]
pub async fn extensions_list() -> Result<Vec<Extension>, String> {
    let roots = InventoryRoots {
        disabled_modules: disabled_modules(),
        ..Default::default()
    };
    let mut rows = inventory::read(&roots);
    if let Some(live) = live_modules().await {
        inventory::overlay_modules(&mut rows, &live);
    }
    Ok(rows)
}

/// The user's disabled list, so a module reads as switched off rather than
/// unknown even when the runtime cannot be reached.
fn disabled_modules() -> std::collections::BTreeSet<String> {
    super::modules::modules_list()
        .into_iter()
        .filter(|m| !m.enabled)
        .map(|m| m.id)
        .collect()
}

/// Ask the runtime what it currently holds, or `None` if it is not reachable.
async fn live_modules() -> Option<Vec<LiveModule>> {
    let request = Request::ListModules {
        id: "extensions-list".to_string(),
    };
    match client::request_once(&client::socket_path(), request).await {
        Ok(Response::ModuleList { modules, .. }) => Some(
            modules
                .into_iter()
                .map(|m| LiveModule {
                    id: m.id,
                    enabled: m.enabled,
                    failed: m.failed,
                    last_error: m.last_error,
                })
                .collect(),
        ),
        // Anything else - unreachable, an error reply, a reply we did not
        // expect - leaves the disk answer standing. It is less informative,
        // never wrong.
        _ => None,
    }
}

/// What actually happened when a revoke ran.
///
/// Per step, not a single verdict: "revoke" is three different operations and
/// a caller that cannot tell which of them succeeded cannot tell the user
/// either. A partial result is normal - an app can lose two of its four grants
/// because the profile changed under it - so this reports rather than
/// collapsing to a bool.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevokeReport {
    /// What was given up, in the user's words.
    pub revoked: Vec<String>,
    /// What could not be, and why.
    pub failed: Vec<String>,
    /// What this does not undo, carried through from the plan.
    pub residue: Vec<String>,
}

/// Revoke what an extension holds, routing each step to the service that owns it.
///
/// The extension is looked up FRESH rather than taken from the caller: a client
/// could otherwise hand back an inventory row it had edited, and the labels on
/// that row are what determine which grants get removed.
#[tauri::command]
pub async fn extensions_revoke(id: String, kind: String) -> Result<RevokeReport, String> {
    let rows = extensions_list().await?;
    let target = rows
        .iter()
        .find(|e| e.id == id && format!("{:?}", e.kind).to_lowercase() == kind.to_lowercase())
        .ok_or_else(|| format!("no {kind} named {id} is installed"))?;

    let plan = arlen_extensions::revoke::plan(target);
    let mut report = RevokeReport {
        revoked: Vec::new(),
        failed: Vec::new(),
        residue: plan.residue.clone(),
    };
    for step in &plan.steps {
        run_step(step, &mut report).await;
    }
    Ok(report)
}

/// Route one step and record what came back.
async fn run_step(step: &arlen_extensions::revoke::RevokeStep, report: &mut RevokeReport) {
    use arlen_extensions::revoke::RevokeStep;

    match step {
        RevokeStep::NarrowProfile { app_id, capabilities } => {
            narrow_profile(app_id, capabilities, report).await;
        }
        RevokeStep::DropConsentGrants { module_id } => {
            drop_consent_grants(module_id, report);
        }
        RevokeStep::RemoveNamespaceGrant { namespace } => {
            match arlen_forage_bridge_install::deprovision_bridge_namespace(namespace) {
                Ok(true) => report.revoked.push(format!("{namespace} can no longer write")),
                // Already gone. The end state is what was asked for, so this is
                // not a failure to report at the user.
                Ok(false) => {}
                Err(e) => report.failed.push(format!("{namespace}: {e}")),
            }
        }
    }
}

/// Narrow an app's profile, one concrete grant at a time.
async fn narrow_profile(app_id: &str, capabilities: &[String], report: &mut RevokeReport) {
    use arlen_permissions::revoke::{RevokeInitiator, RevokeOutcome, RevokeReach};

    let Some(profile) = load_profile(app_id) else {
        report.failed.push(format!("{app_id} has no readable profile"));
        return;
    };
    // Named before anything runs, so a grant nothing can narrow is reported as
    // such rather than looking like a silent success.
    report
        .failed
        .extend(arlen_extensions::revoke::unrevocable(&profile, capabilities));

    let client = os_sdk::UnixGraphClient::new(knowledge_socket());
    for reach in arlen_extensions::revoke::resolve_reaches(&profile, capabilities) {
        let request = RevokeReach {
            target_app_id: app_id.to_string(),
            reach: reach.clone(),
            // The user pressed the button. An agent-initiated revoke is a
            // proposal and the daemon refuses it at the apply site.
            initiator: RevokeInitiator::User,
        };
        match client.revoke(&request).await {
            Ok(RevokeOutcome::Revoked) => report.revoked.push(describe(&reach)),
            // Already narrowed, or the profile no longer has it: the end state
            // matches the request.
            Ok(RevokeOutcome::NoChange) | Ok(RevokeOutcome::NotFound) => {}
            // The app declared this reach essential, so removing it would
            // break the app rather than confine it. Refused upstream, and
            // reported as its own thing - "we will not" is a different answer
            // from "we could not".
            Ok(RevokeOutcome::Required) => report.failed.push(format!(
                "{} is essential to this app and cannot be removed",
                describe(&reach)
            )),
            Ok(RevokeOutcome::NotNarrowing) => report
                .failed
                .push(format!("{} could not be narrowed", describe(&reach))),
            Err(e) => report.failed.push(format!("{}: {e}", describe(&reach))),
        }
    }
}

/// Drop every consent grant recorded for a module.
///
/// Grants are attributed to the module rather than to modulesd, so this removes
/// one extension's authority and touches no other's.
fn drop_consent_grants(module_id: &str, report: &mut RevokeReport) {
    let client = arlen_consent_broker::control_client::ControlClient::at_default_path();
    let grants = match client.list_grants() {
        Ok(g) => g,
        Err(e) => {
            report
                .failed
                .push(format!("the consent store could not be read: {e}"));
            return;
        }
    };
    for grant in grants.iter().filter(|g| g.recipient == module_id) {
        match client.revoke_grant(&grant.revocation_handle) {
            Ok(true) => report
                .revoked
                .push(format!("{module_id} lost its {} grant", grant.class.as_key())),
            Ok(false) => {}
            Err(e) => report.failed.push(format!("{module_id}: {e}")),
        }
    }
}

/// A reach in the words a user would recognise.
fn describe(reach: &arlen_permissions::revoke::RevokedReach) -> String {
    use arlen_permissions::revoke::RevokedReach as R;
    match reach {
        R::Read { entity_pattern } => format!("reading {entity_pattern}"),
        R::Write { entity_pattern } => format!("writing {entity_pattern}"),
        R::Relation { from, to, relation_type } => format!("linking {from} to {to} as {relation_type}"),
        R::InstanceAll => "reach across other apps' data".to_string(),
        R::NetworkDomain { domain } => format!("connecting to {domain}"),
        R::ClipboardCap { cap } => format!("clipboard {cap}"),
        R::NotificationsOff => "sending notifications".to_string(),
        R::InputCap { cap } => format!("input {cap}"),
        R::SearchCap { cap } => format!("search {cap}"),
        R::IntentsCap { cap } => format!("intents {cap}"),
        R::FilesystemDir { dir } => format!("your {dir} folder"),
        R::FilesystemPath { path } => format!("the path {path}"),
        R::EventBusSubscribe { pattern } => format!("watching {pattern} events"),
        R::EventBusPublish { pattern } => format!("publishing {pattern} events"),
        R::SystemCap { cap } => format!("system {cap}"),
    }
}

/// The app's enrolled profile, or `None` if it has none or it will not parse.
fn load_profile(app_id: &str) -> Option<arlen_permissions::PermissionProfile> {
    let path = arlen_permissions::profile_path(app_id).ok()?;
    toml::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// The knowledge daemon's socket, matching its bind.
fn knowledge_socket() -> String {
    if let Some(s) = std::env::var_os("ARLEN_DAEMON_SOCKET") {
        return s.to_string_lossy().into_owned();
    }
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return format!("{}/arlen/knowledge.sock", dir.to_string_lossy());
    }
    "/run/arlen/knowledge.sock".to_string()
}
