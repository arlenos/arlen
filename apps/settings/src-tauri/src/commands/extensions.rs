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
use arlen_extensions::{Extension, ExtensionKind};
use arlen_monitor_reads::observed::{
    all_unmeasured, observed_vs_declared, DeclaredCapability, NotMeasuredReason,
};
use audit_proto::{read_socket_path, ReadClient};
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

/// What a revoke would do, answered before the press.
///
/// The same three lists `RevokeReport` comes back with, and deliberately the
/// same words - `refuses` is built by the function the runner uses, so the plan
/// cannot promise something the press then refuses.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevokePlanView {
    /// What pressing would give up, in the user's words.
    pub revokes: Vec<String>,
    /// What it cannot take back, and why. Empty is the common answer.
    pub refuses: Vec<String>,
    /// What it does not undo, whatever happens.
    pub residue: Vec<String>,
}

/// Plan an extension's revoke without running any of it.
///
/// **The page could already show the cost, and it was blind in two directions.**
/// It reads the DECLARED labels, which is right by construction and cannot see a
/// blanket `network.allow_all` (a grant the strict-narrowing gate cannot shrink
/// one domain at a time) or a system-tier enrolment (where the file this narrows
/// is overridden by one in `/var/lib`). Both surfaced as a FAILURE after the
/// press, which is the wrong moment to learn that a thing cannot be done.
///
/// **What it deliberately does not predict.** A reach the app declared essential
/// comes back `Required` from the daemon, and nothing on disk says which those
/// are - the field does not exist yet, so the refusal is inert-by-absence. A
/// plan that guessed at it would be inventing a refusal; this one stays quiet
/// and the press still reports it if it happens.
///
/// Reads only: it loads profiles and lists grants, and calls no revoke.
#[tauri::command]
pub async fn extensions_revoke_plan(id: String, kind: String) -> Result<RevokePlanView, String> {
    use arlen_extensions::revoke::RevokeStep;

    let rows = extensions_list().await?;
    let target = rows
        .iter()
        .find(|e| e.id == id && format!("{:?}", e.kind).to_lowercase() == kind.to_lowercase())
        .ok_or_else(|| format!("no {kind} named {id} is installed"))?;

    let plan = arlen_extensions::revoke::plan(target);
    let mut view = RevokePlanView {
        revokes: Vec::new(),
        refuses: Vec::new(),
        residue: plan.residue.clone(),
    };
    for step in &plan.steps {
        match step {
            RevokeStep::NarrowProfile { app_id, capabilities } => {
                let Some(profile) = load_profile(app_id) else {
                    // The same sentence the runner would produce, because it is
                    // the same fact: without a profile there is nothing to narrow
                    // and the press would say so too.
                    view.refuses.push(format!("{app_id} has no readable profile"));
                    continue;
                };
                view.refuses
                    .extend(narrowing_refusals(app_id, &profile, capabilities));
                view.revokes.extend(
                    arlen_extensions::revoke::resolve_reaches(&profile, capabilities)
                        .iter()
                        .map(describe),
                );
            }
            RevokeStep::DropConsentGrants { module_id } => {
                view.revokes
                    .push(format!("the consent {module_id} was granted at run time"));
            }
            RevokeStep::RemoveNamespaceGrant { namespace } => {
                view.revokes.push(format!("{namespace} can no longer write"));
            }
        }
    }
    Ok(view)
}

/// What one extension was actually seen doing locally, per declared capability.
///
/// The counterpart to the capability list: that says what it MAY do, this says
/// what the user's own audit ledger recorded it doing. Never "safe" - a grant it
/// has not exercised is a grant it still holds - and never a confident silence
/// where nothing is watching. See `arlen_monitor_reads::observed`.
#[tauri::command]
pub async fn extensions_observed(
    id: String,
    kind: String,
) -> Result<Vec<DeclaredCapability>, String> {
    let rows = extensions_list().await?;
    let target = rows
        .iter()
        .find(|e| e.id == id && format!("{:?}", e.kind).to_lowercase() == kind.to_lowercase())
        .ok_or_else(|| format!("no {kind} named {id} is installed"))?;

    // The ledger records an ACTOR id, which is not always the inventory's id -
    // and when there is none, WHY there is none differs by kind. A module is not
    // audited at all; a bridge is audited under the daemon that submitted its
    // writes, with the bridge named in `node_types`. One sentence for both said
    // "Shell modules are not audited" over a bridge, which is false.
    let Some(actor) = audit_actor(target) else {
        let reason = match target.kind {
            ExtensionKind::Bridge => NotMeasuredReason::NotAttributed,
            _ => NotMeasuredReason::ActorUnknown,
        };
        return Ok(all_unmeasured(&target.capabilities, reason));
    };
    let report = arlen_monitor_reads::access::app_access(
        &ReadClient::new(read_socket_path()),
        ACTIVITY_WINDOW,
    )
    .await;
    Ok(observed_vs_declared(&target.capabilities, &report, &actor))
}

/// How many recent audit entries to aggregate over. A window rather than all of
/// history: "has not touched the network lately" is the question a user asks, and
/// reading the whole ledger to answer it would grow slower the longer the machine
/// has been in use.
const ACTIVITY_WINDOW: u64 = 500;

/// The actor id an extension's audited actions carry, or `None` when that is not
/// established for its kind.
///
/// An app audits under its own id. A module's actor is not settled - host calls
/// are audited, but which id they carry has never been pinned - and guessing
/// would produce an empty answer indistinguishable from restraint.
///
/// A bridge is the same case, which this said otherwise until it was checked. The
/// ledger's `actor` is the kernel-attested peer that SUBMITTED the entry
/// (`ingest/mod.rs`), and a bridge submits nothing: its writes are audited by the
/// knowledge daemon, so those entries carry actor `knowledge` and name the bridge
/// in `node_types` instead. `bridge.<namespace>` is the delegated write identity,
/// which no entry's actor ever equals - so asking for it returned an empty answer
/// that read as a bridge doing nothing. It gets the same `None` the module gets,
/// for the same reason, until the aggregation is keyed on something a bridge's
/// entries actually carry.
fn audit_actor(extension: &Extension) -> Option<String> {
    match extension.kind {
        ExtensionKind::App => Some(extension.id.clone()),
        ExtensionKind::Bridge | ExtensionKind::Module => None,
    }
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

/// What a profile narrowing cannot take back, known WITHOUT running it.
///
/// Both cases are decidable from the profile alone, which is the whole reason
/// this is a function rather than two lines inside the runner: a blanket
/// `network.allow_all`, which the strict-narrowing gate cannot shrink one domain
/// at a time, and a system-tier enrolment, where the file this narrows is
/// overridden by one in `/var/lib` and what the app may do is unchanged.
///
/// Called by the runner AND by the plan, so the sentence a person reads before
/// pressing is the same one they would have read after. A plan that says
/// something different from what the press does is worse than no plan.
fn narrowing_refusals(
    app_id: &str,
    profile: &arlen_permissions::PermissionProfile,
    capabilities: &[String],
) -> Vec<String> {
    let mut out = arlen_extensions::revoke::unrevocable(profile, capabilities);
    if arlen_permissions::system_permissions_dir()
        .join(format!("{app_id}.toml"))
        .exists()
    {
        out.push(format!(
            "{app_id} is enrolled at the system tier, which overrides the profile this \
             narrows - what it may do is unchanged until the system profile is changed"
        ));
    }
    out
}

/// Narrow an app's profile, one concrete grant at a time.
async fn narrow_profile(app_id: &str, capabilities: &[String], report: &mut RevokeReport) {
    use arlen_permissions::revoke::{RevokeInitiator, RevokeOutcome, RevokeReach};

    let Some(profile) = load_profile(app_id) else {
        report.failed.push(format!("{app_id} has no readable profile"));
        return;
    };
    // Named before anything runs, so a grant nothing can narrow is reported as
    // such rather than looking like a silent success. The narrowing still runs
    // under a system-tier enrolment: it is not useless, it is what takes effect
    // if the system profile is ever removed.
    report
        .failed
        .extend(narrowing_refusals(app_id, &profile, capabilities));

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
    // No runtime dir means no broker socket to name, which is a reason this
    // revoke could not run rather than a module with no grants.
    let client = match arlen_consent_broker::control_client::ControlClient::at_default_path() {
        Ok(c) => c,
        Err(e) => {
            report.failed.push(format!("consent grants: {e}"));
            return;
        }
    };
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

/// The app's enrolled USER-tier profile, or `None` if it has none or it will not
/// parse.
///
/// Deliberately the user tier alone, because it is what the revoke path narrows -
/// the daemon's revoke resolves `profile_path`, which is this same file. Reading
/// the tiered result here would list reaches this code cannot then remove.
/// `narrow_profile` says so separately when a system profile is what governs.
fn load_profile(app_id: &str) -> Option<arlen_permissions::PermissionProfile> {
    // The TIERED loader, not the user file. Reading `~/.config` directly meant an
    // app enrolled only at the system tier - anything apt-installed - reported "no
    // readable profile" and the revoke gave up before resolving a single reach,
    // while an app with files in both tiers had its reaches resolved against the
    // overlay the system profile overrides. What may be revoked has to be computed
    // from the profile that is actually in force.
    arlen_permissions::load_profile(app_id).ok()
}

/// The knowledge daemon's socket, matching its bind.
fn knowledge_socket() -> String {
    // Both env names, via the SDK: see `about.rs` for what reading only one cost.
    os_sdk::runtime::knowledge_socket_path()
        .to_string_lossy()
        .into_owned()
}
