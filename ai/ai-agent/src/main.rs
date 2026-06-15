//! `arlen-ai-agent` daemon entry point.
//!
//! Wires the library into a running daemon. The daemon does nothing, and
//! exits, unless at least one behaviour is enabled (Foundation §5.5).
//!
//! Two lifecycle properties matter for a security-relevant daemon:
//!
//! - **Settings changes apply live.** The daemon watches `ai.toml` and, on
//!   any change, tears the whole pipeline down and rebuilds it from the
//!   fresh config. Disabling a behaviour or lowering the read tier therefore
//!   takes effect without a restart, rather than leaving already-loaded
//!   behaviours running under stale grants. A malformed or removed config
//!   reloads to the safe defaults (nothing enabled) and the daemon exits.
//! - **Boot order is forgiving.** A late or briefly-unavailable Event Bus at
//!   session start does not kill the daemon: the initial subscription retries
//!   with backoff until the bus appears (or a shutdown signal arrives).

use std::path::Path;
use std::sync::mpsc::TryRecvError;
use std::time::Duration;

use tokio::sync::watch;

use arlen_ai_agent::behaviour::{BehaviourKind, ReadScope};
use arlen_ai_agent::config::{AgentConfig, ProviderSettings};
use arlen_ai_agent::dbus::{
    new_status_handle, set_status, AgentInterface, LoopStatus, ManualInvoke, StatusHandle,
    AGENT_OBJECT_PATH,
};
use arlen_ai_agent::engine::{
    reads_satisfied, DispatchOutcome, Dispatcher, ExecutionResult, ScreeningMode,
};
use arlen_ai_agent::gate::Gate;
use arlen_ai_agent::slice::{FsPathResolver, ProcMountsPolicy};
use arlen_ai_agent::executor::{ActionReceipt, Compensator, LiveExecutor};
use arlen_ai_agent::graph::{UnixGraph, UnixRelationWriter, DEFAULT_GRAPH_SOCKET};
use arlen_ai_agent::handlers::builtin_handlers;
use arlen_ai_agent::receipt_store::{ReceiptStore, RetainedReceipt};
use arlen_ai_agent::loader::{ai_config_path, behaviour_sources, load};
use arlen_ai_agent::seams::{AgentEvent, GraphHandle, NullObserver, SystemClock, TriggerSource};
use arlen_ai_agent::source::{subscription_types, EventBusSource, DEFAULT_CONSUMER_SOCKET};
use std::sync::{Arc, Mutex};

use arlen_ai_classifier::{ClassifierPolicy, InjectionClassifier};
use arlen_ai_core::audit::{AuditSink, LedgerAuditSink};
use arlen_ai_core::capability::{AccessTier, Capability};
use arlen_ai_core::provider::AIProvider;
use arlen_ai_providers::proxied::{ProxiedConfig, ProxiedProvider};
use os_sdk::config::{Config, ConfigWatcher};
use zbus::Connection;

/// The well-known D-Bus name the agent owns so `ai-proxy` peer-authorises its
/// completion forwards (Foundation §8.4.6: outbound LLM traffic transits the
/// proxy, which checks the caller owns this name).
const AGENT_BUS_NAME: &str = "org.arlen.AIAgent1";

/// How many execution receipts the live-session undo store retains. Beyond
/// this the oldest writes age out of undo-ability (a persisted receipt log is
/// the separate undo-log increment); bounded so a long-running daemon's memory
/// stays flat.
const RECEIPT_CAPACITY: usize = 256;

/// The largest single observation result spilled to disk (B-spill). A larger
/// result is skipped (its ref stays identity-only), bounding any one file.
const SPILL_MAX_BYTES: usize = 1024 * 1024;

/// The per-run observation spill directory under the runtime dir (tmpfs), or a
/// temp-dir fallback. Process-scoped working memory, not durable state.
fn spill_dir() -> std::path::PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("arlen/ai-agent/spill")
}

/// Backoff bounds for the initial Event Bus subscription retry.
const SUBSCRIBE_BACKOFF_INITIAL: Duration = Duration::from_millis(500);
const SUBSCRIBE_BACKOFF_MAX: Duration = Duration::from_secs(30);

/// How often the config watcher is polled. A settings change is picked up
/// within this interval; it is well below any human-perceptible delay and
/// keeps the watcher in the async scope rather than parking a thread.
const CONFIG_POLL_INTERVAL: Duration = Duration::from_millis(300);

/// Why an epoch ended.
#[derive(Debug, PartialEq, Eq)]
enum EpochEnd {
    /// Config changed (or the watch was lost): rebuild everything from fresh
    /// settings, including re-subscribing the event source.
    Reload,
    /// A shutdown signal arrived: stop the daemon.
    Shutdown,
    /// A pending provider's session bus recovered: rebuild only the provider
    /// and dispatcher, keeping the existing subscription (and its buffered
    /// events) so re-arming agent behaviours never drops delivered work.
    RearmProvider,
    /// The singleton name `org.arlen.AIAgent1` is owned by another instance:
    /// stop the daemon. This instance must not subscribe or dispatch, or it
    /// would duplicate triggers (and, with the executor live, duplicate audited
    /// graph writes) against the real owner.
    Superseded,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // A single OS-signal listener fans a shutdown request out to every part
    // of the loop through a watch channel (idempotent: once true it stays
    // true, so any later wait resolves immediately).
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    tokio::spawn(async move {
        shutdown_signal().await;
        let _ = shutdown_tx.send(true);
    });

    // Collaborators that never change at runtime, built once.
    let handlers = builtin_handlers();
    let audit = LedgerAuditSink::at_default_socket();
    let observer = NullObserver;
    let graph = UnixGraph::new(graph_socket());
    let ai_path = ai_config_path();

    // The session-bus connection a configured provider forwards on, owned
    // once for the process and reused across epochs (owning the name per
    // epoch would thrash). Established lazily, only when a provider is
    // configured, and retried on a later epoch if the bus was not yet
    // reachable, so a late session bus or a transient failure is recovered on
    // the next config change rather than disabling agent behaviours forever.
    let mut connection: Option<Connection> = None;

    // Shared cell for the live loop status the D-Bus interface reports.
    // Initialised Idle; the dispatch loop flips it to Busy while handling an
    // event and back to Idle while waiting.
    let status = new_status_handle();

    run(
        Collaborators {
            handlers: &handlers,
            audit: &audit,
            observer: &observer,
            graph: &graph,
            ai_path: &ai_path,
            status: &status,
        },
        &mut connection,
        shutdown_rx,
    )
    .await
}

/// The startup outcome of provisioning the prompt-injection classifier (S17),
/// owned by `main` for the process lifetime. Distinguishes a deliberately
/// unconfigured classifier from one that was configured but could not load, so
/// the latter fails closed instead of silently disabling screening.
enum ProvisionedScreening {
    /// A classifier loaded and will screen external content. Only constructed in
    /// the `onnx` build; matched (and mapped to `ScreeningMode::On`) in all
    /// builds, so the default build still maps it but never produces it.
    #[cfg_attr(not(feature = "onnx"), allow(dead_code))]
    Classifier(Arc<dyn InjectionClassifier>, ClassifierPolicy),
    /// A `[classifier]` was configured but could not be loaded (missing model,
    /// bad path, invalid export, invalid thresholds) or cannot be honoured by
    /// this binary (no `onnx` feature). External-content agent loops fail closed.
    Unavailable,
    /// No classifier is configured (the default build, or no `[classifier]`
    /// section). External content flows sanitised, under the gate's
    /// mandatory-confirmation containment.
    NotConfigured,
}

/// Whether a `[classifier]` section's thresholds are semantically valid: both
/// finite, within `0.0..=1.0`, and ordered (`warn_at <= block_at`). A finite
/// but out-of-range value (a typo like `block_at = 90`) would otherwise be
/// silently clamped by [`ClassifierPolicy::new`] to a threshold that blocks
/// almost nothing, a fail-open weakening. An invalid threshold set fails closed
/// instead (the classifier is treated as unavailable). Pure and always
/// compiled, so it is unit-tested without the `onnx` model.
fn classifier_thresholds_valid(warn_at: f32, block_at: f32) -> bool {
    warn_at.is_finite()
        && block_at.is_finite()
        && (0.0..=1.0).contains(&warn_at)
        && (0.0..=1.0).contains(&block_at)
        && warn_at <= block_at
}

/// The result of parsing the `[classifier]` section of `ai.toml`: absent
/// (deliberately unconfigured), present and valid (with the config), or present
/// but invalid (a typo, an unknown key, a wrong type, or out-of-range
/// thresholds). Pure and always compiled, so the parse/validation is unit-tested
/// without the `onnx` model; only the model *load* is feature-gated.
enum ClassifierProvision {
    // The config is read only in the `onnx` build (to load the model); the
    // default build matches the variant for the fail-closed decision but never
    // reads it.
    Configured(#[cfg_attr(not(feature = "onnx"), allow(dead_code))] arlen_ai_classifier::ClassifierConfig),
    Invalid,
    Absent,
}

/// Parse and validate the `[classifier]` section of an `ai.toml` snapshot.
/// `deny_unknown_fields` makes a misspelled key (e.g. a typoed threshold) a
/// parse error rather than a silently-ignored default, so a broken classifier
/// config fails closed instead of quietly running a weaker screen.
fn parse_classifier_config(ai_text: &str) -> ClassifierProvision {
    use arlen_ai_classifier::ClassifierConfig;

    // `benign_label_index` is deliberately NOT a config field. The ONNX scorer
    // computes injection probability as `1 - softmax[benign_index]`, so a wrong
    // in-range value (e.g. 1 instead of 0) would invert the verdict and silently
    // pass injections. All supported models (Prompt-Guard, ProtectAI DeBERTa)
    // put benign at index 0, so it is hardcoded; with `deny_unknown_fields` any
    // attempt to set it in config is a parse error -> Invalid -> fail closed.
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct RawClassifier {
        model_path: std::path::PathBuf,
        tokenizer_path: std::path::PathBuf,
        #[serde(default = "default_max_tokens")]
        max_tokens: usize,
        #[serde(default = "default_warn")]
        warn_at: f32,
        #[serde(default = "default_block")]
        block_at: f32,
    }
    fn default_max_tokens() -> usize {
        512
    }
    fn default_warn() -> f32 {
        0.5
    }
    fn default_block() -> f32 {
        0.9
    }

    // A parse failure of the *whole* document is not a configured-classifier
    // failure: the daemon's own config load handles a malformed ai.toml
    // (fail-closed, disabled), so treat it as "not configured" rather than
    // blocking.
    let Ok(doc) = toml::from_str::<toml::Table>(ai_text) else {
        return ClassifierProvision::Absent;
    };
    // Distinguish an absent `[classifier]` (deliberately unconfigured, flow)
    // from a present-but-invalid one (a typo or wrong type, fail closed):
    // detect presence on the table, then deserialise the fields.
    let Some(section) = doc.get("classifier") else {
        return ClassifierProvision::Absent;
    };
    let rc: RawClassifier = match section.clone().try_into() {
        Ok(rc) => rc,
        Err(e) => {
            tracing::error!(error = %e, "[classifier] is present but invalid (unknown key, missing or wrong-typed field); external-content agent behaviours will be blocked (fail closed) until fixed");
            return ClassifierProvision::Invalid;
        }
    };
    // Out-of-range or swapped thresholds are a config mistake, not a valid
    // screen: fail closed rather than let the policy clamp them into near-no
    // blocking.
    if !classifier_thresholds_valid(rc.warn_at, rc.block_at) {
        tracing::error!(
            warn_at = rc.warn_at,
            block_at = rc.block_at,
            "[classifier] thresholds are invalid (need finite, 0.0..=1.0, warn_at <= block_at); external-content agent behaviours will be blocked (fail closed) until fixed"
        );
        return ClassifierProvision::Invalid;
    }
    ClassifierProvision::Configured(ClassifierConfig {
        model_path: rc.model_path,
        tokenizer_path: rc.tokenizer_path,
        max_tokens: rc.max_tokens,
        // Hardcoded for the supported model family (benign at index 0); not a
        // config knob, so a typo cannot invert the verdict.
        benign_label_index: 0,
        warn_at: rc.warn_at,
        block_at: rc.block_at,
    })
}

/// Provision the prompt-injection classifier (S17) from an `ai.toml` snapshot,
/// when the `onnx` feature is compiled in.
///
/// The distinction is deliberate (Codex review): a *deliberately* unconfigured
/// classifier ([`ProvisionedScreening::NotConfigured`]) flows external content
/// under the gate's containment, because the model is a Phase-10
/// distro-provisioned artifact and the agent runs (in suggest-mode) before it
/// exists. But a classifier that *was* configured and fails to load or parse is
/// a packaging error or config typo, and degrading it to "no screening" would be
/// fail-open, so it becomes [`ProvisionedScreening::Unavailable`] and the
/// dispatcher blocks external-content agent loops until it is fixed.
///
/// Takes the already-read `ai.toml` text (not a path) so the screening posture
/// is derived from the **same** snapshot as [`AgentConfig`]: reading the file a
/// second time could combine enabled behaviours from one revision with a
/// screening mode from another (a config-race fail-open).
#[cfg(feature = "onnx")]
fn build_screening(ai_text: &str) -> ProvisionedScreening {
    use arlen_ai_classifier::onnx::OnnxClassifier;

    let config = match parse_classifier_config(ai_text) {
        ClassifierProvision::Absent => return ProvisionedScreening::NotConfigured,
        ClassifierProvision::Invalid => return ProvisionedScreening::Unavailable,
        ClassifierProvision::Configured(config) => config,
    };
    match OnnxClassifier::load(&config) {
        Ok(classifier) => {
            tracing::info!("prompt-injection classifier loaded; external content will be screened");
            ProvisionedScreening::Classifier(Arc::new(classifier), config.policy())
        }
        Err(e) => {
            tracing::error!(error = %e, "a [classifier] is configured but failed to load; external-content agent behaviours will be blocked (fail closed) until it is fixed");
            ProvisionedScreening::Unavailable
        }
    }
}

/// Default build: no native ONNX dependency, so no classifier can be loaded.
/// But a `[classifier]` configured in `ai.toml` (valid or not) is an operator
/// intent this binary cannot honour, so it fails closed (Unavailable) rather
/// than silently flowing external content unscreened, a packaging-mismatch
/// fail-open. Absent config flows sanitised under the gate's confirmation
/// containment.
#[cfg(not(feature = "onnx"))]
fn build_screening(ai_text: &str) -> ProvisionedScreening {
    match parse_classifier_config(ai_text) {
        ClassifierProvision::Absent => ProvisionedScreening::NotConfigured,
        _ => {
            tracing::error!(
                "[classifier] is configured but this binary was built without the `onnx` feature; external-content agent behaviours will be blocked (fail closed). Rebuild with --features onnx to screen."
            );
            ProvisionedScreening::Unavailable
        }
    }
}

/// The outcome of trying to own `org.arlen.AIAgent1` and serve its interface.
///
/// The two failure modes are deliberately distinct: a missing session bus is
/// recoverable (run workflow behaviours best-effort and retry in the
/// background), but the name being owned by *another* instance is fatal to this
/// instance, which must not dispatch at all or it would duplicate triggers
/// against the real owner. Collapsing both to a single `None` (the prior bug)
/// let a losing instance keep dispatching workflows.
enum AgentConnection {
    /// Connected and the sole owner of the singleton name. Serve and dispatch.
    Owned(Connection),
    /// No session bus right now. Best-effort: keep running workflow behaviours
    /// and retry the bus in the background.
    BusUnavailable,
    /// The singleton name is owned by another instance. This instance must stop.
    NameTaken,
}

/// Open a session-bus connection, register the `org.arlen.AIAgent1` interface
/// object, and own the well-known name as the sole, non-replaceable owner.
///
/// The interface is registered *before* the name is claimed so a caller that
/// resolves the name always finds an object behind it. The name is requested
/// with `DoNotQueue` and without `AllowReplacement`, so this owner cannot be
/// displaced and the daemon never queues behind another owner. Only primary
/// ownership counts; anything else means a second instance that must not run.
///
/// `status` is the live-status cell the interface reads from. It is updated by
/// the dispatch loop; before the first event it reads `Subscribing`, the
/// correct view of a daemon that is up but not yet receiving triggers.
async fn establish_agent_connection(
    status: StatusHandle,
    compensator: Compensator,
    graph: Arc<dyn GraphHandle>,
    receipts: Arc<Mutex<ReceiptStore<RetainedReceipt>>>,
    manual_tx: tokio::sync::mpsc::Sender<ManualInvoke>,
) -> AgentConnection {
    use zbus::fdo::{RequestNameFlags, RequestNameReply};

    let connection = match Connection::session().await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "no session bus; agent (LLM) behaviours will not run");
            return AgentConnection::BusUnavailable;
        }
    };
    // Register the interface before claiming the name.
    let iface = AgentInterface {
        status,
        compensator,
        graph,
        receipts,
        manual_tx,
    };
    if let Err(e) = connection.object_server().at(AGENT_OBJECT_PATH, iface).await {
        tracing::warn!(error = %e, path = AGENT_OBJECT_PATH, "could not register agent interface; D-Bus behaviour status will not be available");
        // Non-fatal: LLM behaviours can still run, only the status method is
        // unavailable. Do not bail here or a D-Bus registration failure would
        // silently disable the provider.
    }
    match connection
        .request_name_with_flags(AGENT_BUS_NAME, RequestNameFlags::DoNotQueue.into())
        .await
    {
        Ok(RequestNameReply::PrimaryOwner) | Ok(RequestNameReply::AlreadyOwner) => {
            tracing::info!(bus = AGENT_BUS_NAME, path = AGENT_OBJECT_PATH, "agent interface registered");
            AgentConnection::Owned(connection)
        }
        // The name is held by another connection (InQueue/Exists under
        // DoNotQueue): another instance is authoritative. This one must stop.
        Ok(other) => {
            tracing::warn!(name = AGENT_BUS_NAME, ?other, "org.arlen.AIAgent1 is owned by another instance; this instance will stop to avoid duplicate dispatch");
            AgentConnection::NameTaken
        }
        // A bus-level error talking to the name registry: treat as a bus
        // outage (recoverable), not a name collision.
        Err(e) => {
            tracing::warn!(error = %e, name = AGENT_BUS_NAME, "could not reach the bus to own the agent name; will retry");
            AgentConnection::BusUnavailable
        }
    }
}

/// Build the proxied LLM provider for this epoch, **best-effort and
/// non-blocking**: own the bus name (lazily, once) and build the provider, but
/// never wait on the bus. Returns `None` (agent behaviours skip, workflow
/// behaviours still run) when the session bus is unavailable. Building the
/// proxy is lazy in zbus (it does not probe `ai-proxy`), so a build error
/// means the *connection* is bad, not that the proxy is down; the connection
/// is therefore cleared so the background recovery re-establishes a fresh one
/// rather than reusing a dead one. (A down `ai-proxy` with the bus up still
/// builds a provider; its forwards then fail per call and surface as `Failed`
/// loop outcomes, not a build error.) `None` thus always leaves the connection
/// unset, which the caller reads as a recoverable bus outage.
///
/// The connection is established and the singleton name owned elsewhere (the
/// epoch-top eager establish, or the background recovery): a `None` connection
/// here means the session bus is unavailable, so no provider is built and the
/// caller leaves recovery to re-establish it. This function therefore never
/// claims the name itself, keeping the singleton decision in one place.
async fn build_provider(
    settings: &ProviderSettings,
    connection: &mut Option<Connection>,
) -> Option<ProxiedProvider> {
    // Build off the borrow, then act on the result so a failure can clear the
    // connection without overlapping the borrow.
    let built = match connection.as_ref() {
        Some(conn) => Some(ProxiedProvider::with_connection(provider_config(settings), conn).await),
        None => None,
    };
    match built {
        Some(Ok(provider)) => Some(provider),
        Some(Err(e)) => {
            tracing::warn!(error = %e, "could not build the LLM provider; the connection is unhealthy and will be re-established; agent behaviours will not run this epoch");
            *connection = None;
            None
        }
        None => {
            tracing::warn!("a provider is configured but the session bus is unavailable; agent behaviours will not run this epoch (retried in the background)");
            None
        }
    }
}

/// Retry establishing the agent's session-bus connection with backoff until it
/// succeeds, so a configured provider whose bus was late or briefly down comes
/// online without the user touching anything. Returns `true` once the
/// connection is owned (the caller then rebuilds the provider and dispatcher in
/// place via `RearmProvider`, keeping the subscription so agent behaviours
/// re-arm without dropping buffered events). Returns `false` if, during the
/// outage, another instance claimed the singleton name: this instance lost the
/// race and must stop rather than resume dispatching against the new owner.
/// Polled before the event source each
/// iteration (see [`next_dispatch_step`]), so a steady event stream does not
/// starve it. Runs concurrently with dispatch (or idle waiting) and is
/// cancelled by being dropped when a config change, shutdown, or event-driven
/// epoch end fires first.
///
/// This recovers a *build-time* bus outage (the provider could not be
/// constructed at startup, or its connection was unhealthy). A *runtime* loss,
/// where the provider built but a backend later restarts, is handled
/// elsewhere: an `ai-proxy` restart self-recovers, because the proxy is
/// addressed by its well-known name and the bus routes each forward to the
/// current owner, so calls succeed again once it is back (only forwards during
/// the restart fail, surfacing as `Failed` loop outcomes); a session-bus
/// restart (rare, and effectively session-ending) leaves the connection dead
/// until a config reload or supervisor restart. A liveness monitor that
/// re-arms on a dead session connection mid-run needs a connection/proxy
/// liveness probe that does not exist yet, a follow-up.
async fn recover_connection(
    connection: &mut Option<Connection>,
    status: StatusHandle,
    compensator: Compensator,
    graph: Arc<dyn GraphHandle>,
    receipts: Arc<Mutex<ReceiptStore<RetainedReceipt>>>,
    manual_tx: tokio::sync::mpsc::Sender<ManualInvoke>,
) -> bool {
    let mut backoff = SUBSCRIBE_BACKOFF_INITIAL;
    loop {
        tokio::time::sleep(backoff).await;
        if connection.is_some() {
            return true;
        }
        match establish_agent_connection(
            Arc::clone(&status),
            compensator.clone(),
            Arc::clone(&graph),
            Arc::clone(&receipts),
            manual_tx.clone(),
        )
        .await
        {
            AgentConnection::Owned(conn) => {
                *connection = Some(conn);
                return true;
            }
            // Another instance took the singleton name while we were down: we
            // are now the loser and must stop, not resume dispatch.
            AgentConnection::NameTaken => return false,
            // Still no bus: keep retrying with backoff.
            AgentConnection::BusUnavailable => {}
        }
        backoff = (backoff * 2).min(SUBSCRIBE_BACKOFF_MAX);
    }
}

/// Map the resolved provider settings onto the proxy adapter's config.
fn provider_config(settings: &ProviderSettings) -> ProxiedConfig {
    ProxiedConfig {
        name: settings.name.clone(),
        model: settings.model.clone(),
        audit_token: settings.audit_token.clone(),
        context_window: settings.context_window,
    }
}

/// Whether a behaviour can actually run this epoch, mirroring the dispatcher's
/// own eligibility: it must be enabled and its declared read scope satisfied
/// by the configured tier (the dispatcher skips it otherwise), and a
/// `kind: agent` behaviour additionally needs an LLM provider wired (a
/// workflow never does).
fn behaviour_is_runnable(
    enabled: bool,
    kind: BehaviourKind,
    reads: ReadScope,
    read_tier: AccessTier,
    has_provider: bool,
) -> bool {
    enabled
        && reads_satisfied(reads, read_tier)
        && (kind != BehaviourKind::Agent || has_provider)
}

/// Whether an enabled agent behaviour that the configured tier actually allows
/// to run needs an LLM provider this epoch. Over-scoped agents (skipped by the
/// dispatcher anyway) do not count, so a workflow-only epoch is never blocked
/// retrying a provider for a behaviour that could not run regardless.
fn agent_needs_provider(
    enabled: bool,
    kind: BehaviourKind,
    reads: ReadScope,
    read_tier: AccessTier,
) -> bool {
    enabled && kind == BehaviourKind::Agent && reads_satisfied(reads, read_tier)
}

/// The process-lived collaborators the epoch loop borrows on every iteration.
/// Grouped so [`run`]'s signature stays small as the set grows; all fields are
/// cheap shared references, so the struct is passed by value.
struct Collaborators<'a> {
    handlers: &'a arlen_ai_agent::engine::HandlerRegistry,
    audit: &'a LedgerAuditSink,
    observer: &'a NullObserver,
    graph: &'a UnixGraph,
    ai_path: &'a Path,
    /// Live loop-status cell the D-Bus interface reports and the dispatch loop
    /// updates.
    status: &'a StatusHandle,
}

/// The epoch loop. Each iteration is one config epoch: load settings and
/// behaviours, run the dispatcher, and rebuild on the next config change.
async fn run(
    collab: Collaborators<'_>,
    connection: &mut Option<Connection>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<(), Box<dyn std::error::Error>> {
    let Collaborators {
        handlers,
        audit,
        observer,
        graph,
        ai_path,
        status,
    } = collab;
    // Process-lived single-flight gate for classifier scoring (S17), shared
    // into every dispatcher rebuild (config reload / provider rearm) so a
    // scorer that wedged in a prior epoch keeps blocking new scorers until it
    // actually finishes. A fresh per-dispatcher gate would let repeated rearms
    // each spawn a new scorer and exhaust the blocking pool.
    let screen_gate = Arc::new(tokio::sync::Semaphore::new(1));
    // Observation spill store (B-spill), process-lived so it outlives each
    // per-epoch dispatcher: a read observation's full result is persisted here by
    // content-address, so a preview compacted out of the prompt is still
    // recoverable. A per-run directory under the runtime dir (tmpfs) bounds its
    // lifetime; one result over the cap is skipped, not stored.
    let spill_store = arlen_ai_agent::spill::FileSpillStore::new(spill_dir(), SPILL_MAX_BYTES);
    // Execution receipts retained for the daemon's lifetime (across provider
    // re-arms and config reloads) so a later compensate can find the write to
    // undo by its decision correlation id. Populated as Written outcomes are
    // surfaced; read both by the dispatch loop (retain) and the D-Bus
    // `compensate` method, hence shared through an `Arc`.
    let receipts: Arc<Mutex<ReceiptStore<RetainedReceipt>>> =
        Arc::new(Mutex::new(ReceiptStore::new(RECEIPT_CAPACITY)));
    // The owned compensator + graph handle the D-Bus undo path uses. Built once
    // for the process (writer/audit/graph are startup-stable), independent of the
    // config epoch: whether an undo is *permitted* is gated at call time on
    // `executor_live`, not by these being present. Built from the same sockets
    // the dispatch path uses so the compensation re-validates and retracts
    // through the same daemon. `Clone` is a cheap `Arc` bump, so the same
    // compensator threads into both interface-registration sites (the eager
    // establish and the background recovery re-register).
    let compensator = Compensator::new(
        Arc::new(UnixRelationWriter::new(graph_socket())),
        Arc::new(LedgerAuditSink::at_default_socket()) as Arc<dyn AuditSink>,
    );
    let iface_graph: Arc<dyn GraphHandle> = Arc::new(UnixGraph::new(graph_socket()));
    // User-invoke (`run_skill`) channel: the D-Bus interface (re-registered each
    // epoch with a `manual_tx` clone) hands a named-skill request to the live
    // dispatch loop. `manual_tx` is held here for the daemon's life so the
    // channel never closes (its `recv()` never spuriously returns `None`);
    // `manual_rx` is borrowed into each epoch's `dispatch_until_change`. Bounded:
    // a backlog of invokes applies backpressure rather than growing unbounded.
    let (manual_tx, mut manual_rx) = tokio::sync::mpsc::channel::<ManualInvoke>(8);
    loop {
        // At the very top of every epoch, before any (possibly slow) config
        // load, classifier provisioning, or resubscribe, report `subscribing`:
        // the daemon cannot receive triggers until the subscription below is
        // re-established. This resets a stale `idle`/`busy` from the epoch that
        // just ended (reload / source-closed), so a poller never reads a
        // healthy-looking `idle` while the daemon is mid-rebuild.
        set_status(status, LoopStatus::Subscribing);

        // Arm the config watch *before* reading the config, so no settings
        // change can slip through the gap between resolving the config and
        // registering the watch (S16 startup-gap closure). A change that
        // lands between arming and reading is also safe: the read below sees
        // the latest content, and the queued event triggers one harmless
        // extra reload. A malformed config cannot be watched (and reads to
        // the empty defaults), so the daemon exits per §5.5 just below.
        let watcher = match Config::load_path(ai_path).and_then(|c| c.watch()) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(error = %e, "ai.toml is not readable or watchable; exiting");
                return Ok(());
            }
        };

        // Read ai.toml once per epoch and derive BOTH the agent config and the
        // screening posture from that single snapshot. Reading the file twice
        // could combine enabled behaviours from one revision with a screening
        // mode from another (a config-race fail-open), so they must share the
        // exact same text.
        let ai_text = std::fs::read_to_string(ai_path).ok();
        let config = match &ai_text {
            Some(text) => AgentConfig::parse(text),
            None => {
                tracing::info!("no ai.toml found; using safe defaults (agent disabled)");
                AgentConfig::fail_closed()
            }
        };
        let outcome = load(&behaviour_sources(), &config.enabled);
        for err in &outcome.errors {
            tracing::warn!(error = %err, "behaviour failed to load");
        }

        // Provision the injection classifier (S17) from the same snapshot, so a
        // live `ai.toml` change to `[classifier]` takes effect at the next
        // reload rather than only on restart (the daemon applies config live, so
        // the screening posture must track it). The model reload cost is paid
        // only on a config change and only in the `onnx` build; the default
        // build's `build_screening` is a no-op. The owned classifier lives for
        // this epoch; the dispatcher (rebuilt in the inner loop) borrows the mode.
        let provisioned = build_screening(ai_text.as_deref().unwrap_or(""));
        let screening: ScreeningMode = match &provisioned {
            ProvisionedScreening::Classifier(classifier, policy) => {
                ScreeningMode::On(Arc::clone(classifier), *policy)
            }
            ProvisionedScreening::Unavailable => ScreeningMode::FailClosed,
            ProvisionedScreening::NotConfigured => ScreeningMode::Off,
        };

        // Config-scoped collaborators, built once per config epoch and reused
        // across provider rebuilds: a bus recovery rebuilds only the provider
        // and dispatcher (the inner loop below), keeping these and the
        // subscription.
        let read_tier = config.read_tier;
        let capability = Capability::new(read_tier, config.actions);
        // The world-model seams the gate's predict-before-act step reads
        // through: the same graph the handlers use, plus the production path
        // and read-only mount resolvers.
        let paths = FsPathResolver;
        let mounts = ProcMountsPolicy;
        let clock = SystemClock;
        // The relation writer for the live executor, present only when the
        // config opts in (`[agent] executor_live`). Built once per config epoch
        // so the dispatcher (rebuilt per provider epoch below) can borrow it;
        // `None` keeps the daemon in suggest-mode (nothing is ever written).
        let relation_writer =
            config.executor_live.then(|| UnixRelationWriter::new(graph_socket()));
        let needs_provider = outcome.loaded.iter().any(|b| {
            agent_needs_provider(
                b.status.is_enabled(),
                b.behaviour.manifest.kind,
                b.behaviour.manifest.reads,
                read_tier,
            )
        });

        // Foundation §5.5: if nothing *could* run under this config the daemon
        // has no reason to run; exit cleanly before subscribing (the supervisor
        // restarts it when a runnable behaviour is enabled). A behaviour could
        // run when it is enabled, read-tier eligible, and either a workflow or
        // an agent with a provider *configured* (the bus may be down now, but
        // recovery would arm it). Checking this before `subscribe_with_retry`
        // avoids blocking forever on the Event Bus for a config that has no
        // behaviour needing it. This also covers a removed or all-disabled
        // config.
        let could_run = outcome.loaded.iter().any(|b| {
            let enabled = b.status.is_enabled();
            let reads = b.behaviour.manifest.reads;
            enabled
                && reads_satisfied(reads, read_tier)
                && (b.behaviour.manifest.kind != BehaviourKind::Agent || config.provider.is_some())
        });
        if !could_run {
            tracing::info!("no runnable behaviours under this config; the agent has nothing to do, exiting");
            return Ok(());
        }

        // Establish the session-bus connection eagerly here, before subscribing
        // and independent of whether any agent behaviour needs an LLM provider,
        // so the read-only status interface (org.arlen.AIAgent1) is reachable
        // for every running config. A workflow-only epoch and a daemon still
        // retrying the Event Bus subscription both reach this point with no
        // provider; if the connection were established only inside
        // `build_provider` (gated on `needs_provider`) the status method would
        // have no owner to answer while behaviours are loaded and running. Do
        // NOT re-gate this on `needs_provider`. Best-effort: a missing session
        // bus leaves the interface unregistered (a later provider build or a
        // reload re-attempts it) but never blocks workflow dispatch. A name
        // collision is fatal, though: if another instance already owns
        // org.arlen.AIAgent1 this one must stop before subscribing, or both
        // would dispatch the same triggers (and, with the executor live,
        // duplicate audited graph writes).
        if connection.is_none() {
            match establish_agent_connection(
                Arc::clone(status),
                compensator.clone(),
                Arc::clone(&iface_graph),
                Arc::clone(&receipts),
                manual_tx.clone(),
            )
            .await
            {
                AgentConnection::Owned(conn) => *connection = Some(conn),
                // No bus yet: leave the connection unset; recovery re-attempts
                // it in the background while workflow behaviours run. The
                // singleton guard is the D-Bus name, so during a session-bus
                // outage it cannot engage: two instances started while the bus
                // is down would both dispatch until one loses the name on
                // recovery. In the real deployment this does not arise (a
                // systemd user singleton runs one instance, and a dead session
                // bus means the session itself is not up), and the only path to
                // a duplicated *write* (executor_live) is default-off and
                // human-gated. A bus-independent process lock (flock) is the
                // robust closure and belongs to single-instance hardening, not
                // here; keeping workflows running without a bus is deliberate.
                AgentConnection::BusUnavailable => {}
                // Another instance is authoritative: do not subscribe or
                // dispatch. Exit cleanly (the supervisor keeps the real owner).
                AgentConnection::NameTaken => return Ok(()),
            }
        }

        // Subscribe to exactly the event types the enabled behaviours need.
        // The subscription is config-scoped: a provider recovery rebuilds the
        // dispatcher in place (below) without dropping it, so buffered events
        // survive re-arming.
        let types = subscription_types(&outcome.loaded);
        let mut source =
            match subscribe_with_retry(consumer_socket(), types, &watcher, &mut shutdown_rx).await {
                Ok(s) => s,
                // Subscription waits only on config/shutdown, so it never
                // yields Superseded; exit on Shutdown (and, defensively,
                // Superseded), reload otherwise.
                Err(EpochEnd::Shutdown | EpochEnd::Superseded) => return Ok(()),
                Err(EpochEnd::Reload | EpochEnd::RearmProvider) => continue,
            };

        // Provider epoch: build the provider + dispatcher and dispatch; on a bus
        // recovery rebuild only these (keeping the subscription) so agent
        // behaviours re-arm without losing delivered work.
        let epoch_end = loop {
            // Build the provider only when one is configured and an eligible
            // agent behaviour needs it. Best-effort and non-blocking: an
            // unavailable bus leaves agents skipped but never blocks workflow
            // behaviours from running. The provider owns a cheap clone of the
            // connection, so rebuilding it per iteration is fine.
            let provider_holder: Option<ProxiedProvider> = match (needs_provider, &config.provider) {
                (true, Some(settings)) => build_provider(settings, connection).await,
                _ => None,
            };
            let provider: Option<&dyn AIProvider> =
                provider_holder.as_ref().map(|p| p as &dyn AIProvider);

            // Foundation §5.5: a behaviour is runnable when enabled, read-tier
            // eligible, and (for an agent) backed by a provider.
            let mut runnable = 0usize;
            for b in &outcome.loaded {
                let enabled = b.status.is_enabled();
                let kind = b.behaviour.manifest.kind;
                let reads = b.behaviour.manifest.reads;
                if behaviour_is_runnable(enabled, kind, reads, read_tier, provider.is_some()) {
                    runnable += 1;
                } else if agent_needs_provider(enabled, kind, reads, read_tier) {
                    // An eligible agent behaviour kept off only by a missing
                    // provider (an over-scoped one is skipped by the dispatcher
                    // with its own log, not here).
                    tracing::warn!(
                        behaviour = %b.behaviour.manifest.name,
                        "agent behaviour is enabled but no AI provider is available; it will not run"
                    );
                }
            }
            // A provider that is configured and needed but could not be built
            // because the session bus was unavailable (`build_provider` leaves
            // the connection unset on failure). The daemon then stays alive and
            // retries the bus in the background, re-arming agent behaviours when
            // it recovers, rather than exiting (an agent-only config would
            // otherwise restart-loop) or waiting for an unrelated config change.
            let pending_provider =
                needs_provider && config.provider.is_some() && provider.is_none();

            if runnable == 0 && !pending_provider {
                tracing::info!("no runnable behaviours; the agent has nothing to do, exiting");
                return Ok(());
            }

            let gate = Gate::new(&capability, audit, observer, &paths, &mounts);
            // `read_tier` gates which behaviours may read at all: the dispatcher
            // denies the graph to any behaviour whose declared `reads` exceeds
            // it. It does NOT yet constrain the *content* of an allowed
            // behaviour's queries to the tier (mandatory Cypher anchor injection
            // on the current session / active project / lookback window). That
            // finer, value-level enforcement has to live in the knowledge
            // daemon, which does not yet carry a per-query tier on the wire; it
            // is the same documented S16 follow-up the ai-daemon shares, not an
            // agent-local concern. A process-local scope wrapper here would not
            // bind a compromised handler (it could reach the knowledge socket
            // directly), and B1 behaviours are trusted first-party built-ins, so
            // the coarse gate is the boundary today.
            // S17: external-content screening mode, re-applied per epoch (the
            // dispatcher is rebuilt on reload/provider recovery); the classifier
            // itself is the process-lived startup resource owned by `main`.
            let mut dispatcher =
                Dispatcher::new(&outcome.loaded, handlers, graph, read_tier, gate, provider, &clock)
                    .with_screening_mode(screening.clone())
                    .with_screen_gate(Arc::clone(&screen_gate))
                    .with_spill(&spill_store);
            // Opt into executing when configured: the executor shares the gate's
            // capability/path/mount/audit collaborators, so it re-validates and
            // audits a proven write identically to the proof the gate ran.
            if let Some(writer) = relation_writer.as_ref() {
                dispatcher = dispatcher
                    .with_executor(LiveExecutor::new(&capability, &paths, &mounts, writer, audit));
                tracing::warn!(
                    "executor enabled: proven workflow decisions will write to the graph. \
                     Dispatch is still cancellable on reload/shutdown; see [agent] executor_live."
                );
            }

            if runnable > 0 {
                tracing::info!(runnable, "starting agent");
            } else {
                tracing::info!(
                    "a provider is configured but its session bus is unavailable; waiting to enable agent behaviours"
                );
            }

            // Dispatch until the config changes, a shutdown arrives, or (when a
            // provider is pending) its bus recovers. Recovery is observed at the
            // event boundary and rebuilds in place, so it neither starves behind
            // events nor drops buffered ones. While a provider is pending, an
            // agent-trigger event that arrives is dispatched against the
            // provider-less dispatcher and recorded as `Skipped` (transparent,
            // not silently dropped): the agent cannot run a behaviour without a
            // model during the outage, and replaying a now-stale trigger after
            // recovery is generally undesirable. Buffering and replaying agent
            // events across an outage (split workflow/agent subscriptions) is a
            // follow-up if a behaviour ever wants it.
            // Drive a background reconnect whenever the session bus connection
            // is missing, not only when an agent provider is pending: a
            // workflow-only config with the bus down at epoch start would
            // otherwise never (re)register `org.arlen.AIAgent1`, hiding the live
            // status from the dashboard for the whole epoch even after the bus
            // comes back. Recovery is raced against events inside the dispatch
            // loop, so workflow dispatch is never blocked; when it resolves the
            // epoch re-arms with the status interface registered.
            let needs_reconnect = pending_provider || connection.is_none();
            let end = if needs_reconnect {
                dispatch_until_change(
                    &dispatcher,
                    &mut source,
                    &watcher,
                    &mut shutdown_rx,
                    status,
                    &receipts,
                    &mut manual_rx,
                    recover_connection(
                        connection,
                        Arc::clone(status),
                        compensator.clone(),
                        Arc::clone(&iface_graph),
                        Arc::clone(&receipts),
                        manual_tx.clone(),
                    ),
                )
                .await
            } else {
                dispatch_until_change(
                    &dispatcher,
                    &mut source,
                    &watcher,
                    &mut shutdown_rx,
                    status,
                    &receipts,
                    &mut manual_rx,
                    std::future::pending::<bool>(),
                )
                .await
            };

            match end {
                // Bus recovered: rebuild the provider + dispatcher in place,
                // keeping the subscription and its buffered events.
                EpochEnd::RearmProvider => continue,
                // Config change or shutdown: leave the provider epoch.
                other => break other,
            }
        };

        match epoch_end {
            EpochEnd::Shutdown => {
                tracing::info!("shutdown signal received, stopping");
                return Ok(());
            }
            EpochEnd::Superseded => {
                tracing::warn!("another instance owns org.arlen.AIAgent1; stopping to avoid duplicate dispatch");
                return Ok(());
            }
            EpochEnd::Reload => {
                tracing::info!("reloading agent settings");
                // Loop: rebuild the pipeline from the fresh config.
            }
            // Recovery is handled inside the provider epoch above.
            EpochEnd::RearmProvider => {}
        }
    }
}

/// What the dispatch loop should do next, decided by [`next_dispatch_step`].
enum DispatchStep {
    /// End the epoch (a config change or shutdown).
    End(EpochEnd),
    /// A pending provider's bus recovered; reload to re-arm agent behaviours.
    Recovered,
    /// Process this event.
    Event(AgentEvent),
    /// The event source closed permanently; rebuild to recover.
    SourceClosed,
    /// A user-invoke (`run_skill`) request: run the named skill on the live
    /// dispatcher, reply, and continue the epoch (not an epoch end).
    Manual(ManualInvoke),
}

/// Pick the next dispatch action with a `biased` priority: a **config change /
/// shutdown** wins first (revocation safety), then **provider recovery**, then
/// the **next event**. Recovery is checked before the event source so it is
/// polled (and so progresses) every iteration rather than being starved behind
/// a busy event stream; it is safe to prefer it over a buffered event because
/// recovery only rebuilds the provider/dispatcher in place (`RearmProvider`),
/// keeping the subscription, so an event buffered now is still delivered after
/// the rebuild. Kept separate from the loop so the ordering is unit-testable
/// without a live event source.
async fn next_dispatch_step(
    config_change: impl std::future::Future<Output = EpochEnd>,
    recovery: impl std::future::Future<Output = bool>,
    manual: impl std::future::Future<Output = Option<ManualInvoke>>,
    next_event: impl std::future::Future<Output = Option<AgentEvent>>,
) -> DispatchStep {
    tokio::select! {
        biased;
        end = config_change => DispatchStep::End(end),
        // `recovery` resolves `true` when the bus is back and owned (re-arm), or
        // `false` when another instance took the singleton name during the
        // outage (this instance must stop).
        owned = recovery => if owned {
            DispatchStep::Recovered
        } else {
            DispatchStep::End(EpochEnd::Superseded)
        },
        // A user invoke. Polled before the event source so an explicit user
        // action is not starved behind a busy event stream; it is rare (a
        // human action) so it cannot in turn starve events. `None` is
        // unreachable while the outer `manual_tx` is held for the daemon's
        // life; a benign reload covers the impossible closed-channel case
        // rather than busy-looping on an always-ready `None`.
        req = manual => match req {
            Some(invoke) => DispatchStep::Manual(invoke),
            None => DispatchStep::End(EpochEnd::Reload),
        },
        maybe = next_event => match maybe {
            Some(event) => DispatchStep::Event(event),
            None => DispatchStep::SourceClosed,
        },
    }
}

/// Dispatch events until the config changes, a shutdown signal arrives, or the
/// `recovery` future resolves (a pending provider's bus came back, signalling
/// a reload to re-arm agent behaviours). Pass `std::future::pending()` when
/// there is nothing to recover.
///
/// `biased` checks the config watcher before pulling the next event, and a
/// revocation that lands between subscribing and acting is honored before the
/// event is dispatched. So a settings change always wins over processing a
/// further event under the old grants (at most the one event already in
/// flight finishes under the previous settings). Recovery is checked only here
/// at the event boundary, never against an active dispatch, so a bus recovery
/// (unlike a revocation, not a safety reason to abort) cannot drop an in-flight
/// workflow event.
#[allow(clippy::too_many_arguments)]
async fn dispatch_until_change(
    dispatcher: &Dispatcher<'_>,
    source: &mut EventBusSource,
    watcher: &ConfigWatcher,
    shutdown_rx: &mut watch::Receiver<bool>,
    status: &StatusHandle,
    receipts: &std::sync::Mutex<ReceiptStore<RetainedReceipt>>,
    manual_rx: &mut tokio::sync::mpsc::Receiver<ManualInvoke>,
    recovery: impl std::future::Future<Output = bool>,
) -> EpochEnd {
    tokio::pin!(recovery);
    loop {
        // Idle while blocked waiting for the next trigger.
        set_status(status, LoopStatus::Idle);
        // Wait for the next event, ending the epoch on a config change,
        // shutdown, or provider recovery before any further dispatch.
        let event = match next_dispatch_step(
            wait_config_change(watcher, shutdown_rx),
            &mut recovery,
            manual_rx.recv(),
            source.recv(),
        )
        .await
        {
            DispatchStep::End(end) => return end,
            // The provider's bus is back: re-arm by rebuilding the provider and
            // dispatcher in place, keeping this subscription (and its buffered
            // events) rather than dropping them.
            DispatchStep::Recovered => {
                tracing::info!("session bus recovered; re-arming agent behaviours");
                return EpochEnd::RearmProvider;
            }
            // The SDK consumer reconnects internally, so a closed source means
            // it is permanently gone; rebuild to recover.
            DispatchStep::SourceClosed => {
                tracing::warn!("event source closed; rebuilding");
                return EpochEnd::Reload;
            }
            // A user invoke: run the named skill on the LIVE dispatcher
            // (current provider/config/grants), retain any write receipt and log
            // the outcomes exactly as the event path does, reply to the caller,
            // then continue the epoch. Not raced against a config change: a
            // manual run is an explicit, bounded user action (an agent skill is
            // bounded by its own budget), so it runs to completion rather than
            // being abortable mid-run like an autonomous event dispatch.
            DispatchStep::Manual(invoke) => {
                set_status(status, LoopStatus::Busy);
                let summary = match dispatcher.dispatch_named(&invoke.name).await {
                    Some(outcomes) => {
                        for outcome in &outcomes {
                            log_dispatch_outcome(outcome);
                            retain_receipt(receipts, outcome);
                        }
                        summarize_manual_run(&invoke.name, &outcomes)
                    }
                    None => format!("no-such-skill: {}", invoke.name),
                };
                let _ = invoke.respond.send(summary);
                continue;
            }
            DispatchStep::Event(event) => event,
        };
        // A change that landed between subscribing and now is honored before
        // the event is dispatched.
        if matches!(watcher.try_recv(), Ok(()) | Err(TryRecvError::Disconnected)) {
            return EpochEnd::Reload;
        }
        // Busy while handling the event: matching, screening, and (for a
        // `kind: agent` behaviour) the whole bounded loop happen inside the
        // dispatch below.
        set_status(status, LoopStatus::Busy);
        // Race the dispatch (which, for a `kind: agent` behaviour, may run a
        // whole bounded loop) against a config change or shutdown, so a
        // revocation aborts an in-flight agent loop at its next await rather
        // than letting it run to its budget under stale grants. Dropping the
        // dispatch future cancels it cleanly: suggest-mode executes nothing,
        // so no partial action is left behind, and the gate audits before it
        // decides, so a dropped step leaves a record but no surfaced action.
        if let Some(end) = dispatch_or_reload(
            dispatcher.dispatch(&event),
            wait_config_change(watcher, shutdown_rx),
            receipts,
        )
        .await
        {
            // Aborting mid-dispatch ends the epoch. Leave status as-is: the
            // outer loop resets it to `subscribing` at its top before any
            // rebuild, which is the honest state for a daemon about to
            // resubscribe (setting `idle` here would be the stale-idle bug).
            return end;
        }
    }
}

/// Run `dispatch` to completion, logging its outcomes, unless `abort` (a
/// config change or shutdown) resolves first. Returns `Some(end)` when aborted
/// (the dispatch future is dropped, cancelling it), `None` when the dispatch
/// completed. `biased` so a pending revocation wins over finishing the event.
///
/// Revocation contract: dropping the dispatch future stops the in-flight
/// agent loop at its next await, so no further provider call or gate decision
/// is made under the old grants, and (suggest-mode) nothing is executed. One
/// caveat is inherent to cancelling a future, not specific to this code: a
/// provider call already inside `complete` may have sent its proxy forward, so
/// that single LLM egress can still complete upstream under the old prompt and
/// grants; its response is then discarded with the dropped future. Aborting
/// that already-sent egress needs proxy-side, correlation-id-keyed
/// cancellation (an `ai-proxy` feature), a deliberate follow-up.
async fn dispatch_or_reload(
    dispatch: impl std::future::Future<Output = Vec<DispatchOutcome>>,
    abort: impl std::future::Future<Output = EpochEnd>,
    receipts: &std::sync::Mutex<ReceiptStore<RetainedReceipt>>,
) -> Option<EpochEnd> {
    tokio::select! {
        biased;
        end = abort => Some(end),
        outcomes = dispatch => {
            for outcome in &outcomes {
                log_dispatch_outcome(outcome);
                retain_receipt(receipts, outcome);
            }
            None
        }
    }
}

/// Retain the execution receipt of a real write, keyed by the decision's
/// correlation id, so a later compensate can find the write to undo. Only a
/// `Written` outcome carries a receipt; a failed or indeterminate write does
/// not (an indeterminate one is reconciled on the next run, not undone). A
/// poisoned lock is ignored: losing a receipt only forgoes an undo, never
/// corrupts state.
fn retain_receipt(
    receipts: &std::sync::Mutex<ReceiptStore<RetainedReceipt>>,
    outcome: &DispatchOutcome,
) {
    if let DispatchOutcome::Decided {
        behaviour,
        executed: Some(ExecutionResult::Written(receipt)),
        ..
    } = outcome
    {
        if let Ok(mut store) = receipts.lock() {
            store.record(
                receipt.correlation_id().to_string(),
                RetainedReceipt {
                    // Store the compensate-ready receipt: a graph write wrapped
                    // as ActionReceipt::Graph, the exact type compensate consumes.
                    receipt: ActionReceipt::Graph(receipt.clone()),
                    behaviour: behaviour.clone(),
                },
            );
        }
    }
}

/// A short status summary for a user-invoke (`run_skill`) reply: the skill
/// name, the number of outcomes, and how many were a gate decision (the
/// meaningful "it proposed/acted" signal). Deliberately coarse — the per-outcome
/// detail is logged via [`log_dispatch_outcome`], and the rich display is the
/// harness's surface; this is just an at-a-glance acknowledgement to the caller.
fn summarize_manual_run(name: &str, outcomes: &[DispatchOutcome]) -> String {
    let decided = outcomes
        .iter()
        .filter(|o| matches!(o, DispatchOutcome::Decided { .. }))
        .count();
    format!(
        "ran: {name} ({} outcome(s), {decided} decision(s))",
        outcomes.len()
    )
}

/// Surface a dispatch outcome. Decisions are also recorded in the audit
/// ledger by the gate; this is the operational view, so a refused, failed,
/// or skipped behaviour is visible rather than silent. (Proposed actions do
/// not yet have a downstream consumer: the suggestion surface and action
/// executor are later phases; suggest-mode decisions are audited regardless.)
fn log_dispatch_outcome(outcome: &DispatchOutcome) {
    match outcome {
        DispatchOutcome::Decided {
            behaviour,
            action,
            decision,
            reason,
            audit_index,
            plan,
            dry_run,
            executed,
        } => {
            tracing::info!(
                behaviour = %behaviour,
                summary = %action.summary,
                ?decision,
                ?reason,
                audit_index = *audit_index,
                "behaviour decision gated and audited"
            );
            // The concrete effect and how to undo it, so the decision's plan is
            // visible. Content-free: schema bind-name effects (e.g.
            // AssertEdge file-FILE_PART_OF-project), never the operands. `None`
            // for an unregistered tool.
            if let Some(plan) = plan {
                tracing::info!(
                    behaviour = %behaviour,
                    effects = ?plan.effects,
                    undo = ?plan.compensation,
                    "decision execution plan"
                );
            }
            // The dry-run executor's concrete planned write and its strict-create
            // absence condition, for a proven PreviewThenExecute decision.
            // Content-bearing (it holds the operand ids), so this is the local
            // activity log, not the content-free audit.
            if let Some(report) = dry_run {
                tracing::info!(
                    behaviour = %behaviour,
                    write = ?report.write,
                    conditional_on_absent_edge = report.conditional_on_absent_edge,
                    "decision dry-run write plan"
                );
            }
            // The live executor's result, when the daemon opted into executing.
            // A failure is surfaced at warn so a dropped mutation is visible for
            // recovery, not erased.
            match executed {
                Some(ExecutionResult::Written(outcome)) => tracing::info!(
                    behaviour = %behaviour,
                    ?outcome,
                    "live executor wrote the relation"
                ),
                Some(ExecutionResult::Failed(reason)) => tracing::warn!(
                    behaviour = %behaviour,
                    reason = %reason,
                    "live executor did not complete the write"
                ),
                Some(ExecutionResult::Indeterminate { reason, .. }) => tracing::warn!(
                    behaviour = %behaviour,
                    reason = %reason,
                    "live executor write outcome is unknown; reconciled on the next run"
                ),
                None => {}
            }
        }
        DispatchOutcome::Refused { behaviour, reason } => {
            tracing::warn!(behaviour = %behaviour, reason = %reason, "behaviour action refused")
        }
        DispatchOutcome::Failed { behaviour, reason } => {
            tracing::warn!(behaviour = %behaviour, reason = %reason, "behaviour handler failed")
        }
        DispatchOutcome::Terminal { behaviour, outcome } => {
            tracing::debug!(behaviour = %behaviour, outcome = %outcome, "behaviour reached a terminal condition")
        }
        DispatchOutcome::Skipped { behaviour, reason } => {
            tracing::debug!(behaviour = %behaviour, reason = %reason, "behaviour skipped")
        }
        DispatchOutcome::Coalesced { behaviour } => {
            tracing::debug!(behaviour = %behaviour, "behaviour dispatch coalesced (burst)")
        }
        DispatchOutcome::Blocked { behaviour, reason } => {
            tracing::warn!(behaviour = %behaviour, reason = %reason, "external content blocked before reaching the model")
        }
    }
}

/// Subscribe to the Event Bus, retrying with backoff until it is reachable.
/// A config change during the wait aborts to a rebuild (so the daemon does
/// not subscribe with the old, possibly-revoked settings), and a shutdown
/// signal stops it. Either is returned as `Err(EpochEnd)`.
async fn subscribe_with_retry(
    socket: String,
    types: Vec<String>,
    watcher: &ConfigWatcher,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> Result<EventBusSource, EpochEnd> {
    let mut backoff = SUBSCRIBE_BACKOFF_INITIAL;
    loop {
        tokio::select! {
            biased;
            end = wait_config_change(watcher, shutdown_rx) => return Err(end),
            res = EventBusSource::subscribe(socket.clone(), types.clone()) => match res {
                Ok(source) => return Ok(source),
                Err(e) => tracing::warn!(error = %e, "event bus unavailable, retrying"),
            },
        }
        tokio::select! {
            _ = tokio::time::sleep(backoff) => {}
            end = wait_config_change(watcher, shutdown_rx) => return Err(end),
        }
        backoff = (backoff * 2).min(SUBSCRIBE_BACKOFF_MAX);
    }
}

/// Resolve when `ai.toml` changes (or the watch is lost), or when a shutdown
/// signal arrives. Polls the watcher so it stays in the async scope.
async fn wait_config_change(
    watcher: &ConfigWatcher,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> EpochEnd {
    loop {
        match watcher.try_recv() {
            Ok(()) => return EpochEnd::Reload,
            Err(TryRecvError::Empty) => {}
            // A lost watch means we can no longer observe revocations, so
            // rebuild (which re-establishes the watch, fail-closed).
            Err(TryRecvError::Disconnected) => return EpochEnd::Reload,
        }
        if sleep_or_shutdown(CONFIG_POLL_INTERVAL, shutdown_rx).await {
            return EpochEnd::Shutdown;
        }
    }
}

/// Sleep for `dur`, returning `true` if a shutdown signal arrived first.
async fn sleep_or_shutdown(dur: Duration, shutdown_rx: &mut watch::Receiver<bool>) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(dur) => false,
        _ = shutdown_requested(shutdown_rx) => true,
    }
}

/// Resolve once a shutdown has been signalled. Resolves immediately if it
/// already has, and also if the signal plumbing is gone (fail toward stop).
async fn shutdown_requested(shutdown_rx: &mut watch::Receiver<bool>) {
    let _ = shutdown_rx.wait_for(|&stop| stop).await;
}

fn consumer_socket() -> String {
    std::env::var("ARLEN_CONSUMER_SOCKET").unwrap_or_else(|_| DEFAULT_CONSUMER_SOCKET.to_string())
}

/// Resolve the knowledge daemon socket: this agent's own `ARLEN_KNOWLEDGE_SOCKET`
/// override first, then `ARLEN_DAEMON_SOCKET` (the knowledge daemon's OWN bind
/// env), then the system default. The `ARLEN_DAEMON_SOCKET` fallback (which
/// knowledge-mcp and the ai-daemon already honour) means a launcher that sets
/// only the daemon's canonical bind env reaches the agent too, instead of the
/// agent silently dialing the default — the bug that recurred in the dev stack
/// and the integration harness when only one of the two env names was set.
fn graph_socket() -> String {
    std::env::var("ARLEN_KNOWLEDGE_SOCKET")
        .or_else(|_| std::env::var("ARLEN_DAEMON_SOCKET"))
        .unwrap_or_else(|_| DEFAULT_GRAPH_SOCKET.to_string())
}

/// Resolve when Ctrl-C or SIGTERM arrives, so the daemon stops cleanly.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            sig.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(feature = "onnx"))]
    #[test]
    fn non_onnx_build_fails_closed_when_a_classifier_is_configured() {
        // No [classifier] table: deliberately unconfigured, flows.
        assert!(matches!(
            build_screening("[ai]\nenabled = true\n"),
            ProvisionedScreening::NotConfigured
        ));
        // [classifier] present but this binary cannot honour it: fail closed,
        // not silently Off.
        assert!(matches!(
            build_screening(
                "[ai]\nenabled = true\n\n[classifier]\nmodel_path = \"/m\"\ntokenizer_path = \"/t\"\n"
            ),
            ProvisionedScreening::Unavailable
        ));
    }

    #[test]
    fn parse_classifier_config_distinguishes_absent_valid_and_invalid() {
        use ClassifierProvision::{Absent, Configured, Invalid};
        let valid = "[classifier]\nmodel_path = \"/m\"\ntokenizer_path = \"/t\"\n";
        // Absent: no [classifier] table.
        assert!(matches!(parse_classifier_config("[ai]\nenabled = true\n"), Absent));
        // Valid: required fields present, defaults for the rest.
        assert!(matches!(parse_classifier_config(valid), Configured(_)));
        // Invalid: a misspelled key (deny_unknown_fields) must fail closed,
        // not silently run with the default threshold.
        let typo = "[classifier]\nmodel_path = \"/m\"\ntokenizer_path = \"/t\"\nblock_threshld = 0.5\n";
        assert!(matches!(parse_classifier_config(typo), Invalid));
        // Invalid: a missing required field.
        assert!(matches!(
            parse_classifier_config("[classifier]\nmodel_path = \"/m\"\n"),
            Invalid
        ));
        // Invalid: out-of-range threshold.
        let bad_threshold =
            "[classifier]\nmodel_path = \"/m\"\ntokenizer_path = \"/t\"\nblock_at = 90.0\n";
        assert!(matches!(parse_classifier_config(bad_threshold), Invalid));
        // Invalid: benign_label_index is not a config knob (hardcoded to 0 for
        // the supported models); setting it must fail closed, not invert the
        // verdict via a wrong index.
        let label_index =
            "[classifier]\nmodel_path = \"/m\"\ntokenizer_path = \"/t\"\nbenign_label_index = 1\n";
        assert!(matches!(parse_classifier_config(label_index), Invalid));
    }

    #[test]
    fn classifier_thresholds_validation_fails_closed_on_bad_config() {
        // Valid: ordered and in range.
        assert!(classifier_thresholds_valid(0.5, 0.9));
        assert!(classifier_thresholds_valid(0.0, 1.0));
        assert!(classifier_thresholds_valid(0.7, 0.7));
        // Out of range (the `block_at = 90` typo that would otherwise clamp to a
        // near-no-blocking 1.0).
        assert!(!classifier_thresholds_valid(0.5, 90.0));
        assert!(!classifier_thresholds_valid(-0.1, 0.9));
        // Swapped.
        assert!(!classifier_thresholds_valid(0.9, 0.5));
        // Non-finite.
        assert!(!classifier_thresholds_valid(f32::NAN, 0.9));
        assert!(!classifier_thresholds_valid(0.5, f32::INFINITY));
    }

    #[test]
    fn a_provider_makes_an_eligible_agent_behaviour_runnable() {
        use BehaviourKind::{Agent, Workflow};
        let ok = AccessTier::Full; // satisfies any read scope
        // An enabled agent behaviour runs only with a provider; a workflow runs
        // either way; a disabled behaviour never runs.
        assert!(!behaviour_is_runnable(true, Agent, ReadScope::Minimal, ok, false));
        assert!(behaviour_is_runnable(true, Agent, ReadScope::Minimal, ok, true));
        assert!(behaviour_is_runnable(true, Workflow, ReadScope::Minimal, ok, false));
        assert!(!behaviour_is_runnable(false, Agent, ReadScope::Minimal, ok, true));
        // An over-scoped behaviour (read scope exceeds the tier) never runs,
        // even with a provider, matching the dispatcher's own skip.
        assert!(!behaviour_is_runnable(true, Agent, ReadScope::Full, AccessTier::Minimal, true));
        assert!(!behaviour_is_runnable(true, Workflow, ReadScope::Full, AccessTier::Minimal, false));
    }

    #[test]
    fn only_an_eligible_agent_behaviour_needs_a_provider() {
        use BehaviourKind::{Agent, Workflow};
        let ok = AccessTier::Full;
        assert!(agent_needs_provider(true, Agent, ReadScope::Minimal, ok));
        // Workflow never needs one; disabled never needs one; an over-scoped
        // agent (skipped anyway) must not make the daemon block on the bus.
        assert!(!agent_needs_provider(true, Workflow, ReadScope::Minimal, ok));
        assert!(!agent_needs_provider(false, Agent, ReadScope::Minimal, ok));
        assert!(!agent_needs_provider(true, Agent, ReadScope::Full, AccessTier::Minimal));
    }

    #[test]
    fn provider_config_maps_every_setting_onto_the_proxy_config() {
        let settings = ProviderSettings {
            name: "ollama-default".to_string(),
            model: "llama3:8b".to_string(),
            context_window: 131072,
            audit_token: "tok-xyz".to_string(),
        };
        let cfg = provider_config(&settings);
        assert_eq!(cfg.name, "ollama-default");
        assert_eq!(cfg.model, "llama3:8b");
        assert_eq!(cfg.context_window, 131072);
        assert_eq!(cfg.audit_token, "tok-xyz");
    }

    #[tokio::test]
    async fn a_config_change_aborts_an_in_flight_dispatch() {
        // A never-completing dispatch (stands in for a long agent loop) is
        // abandoned the moment a config change is observed.
        let receipts = std::sync::Mutex::new(ReceiptStore::<RetainedReceipt>::new(8));
        let result = dispatch_or_reload(
            std::future::pending::<Vec<DispatchOutcome>>(),
            std::future::ready(EpochEnd::Reload),
            &receipts,
        )
        .await;
        assert!(matches!(result, Some(EpochEnd::Reload)));
    }

    #[tokio::test]
    async fn a_completed_dispatch_continues_the_epoch() {
        // With no config change pending, the dispatch completes and the epoch
        // continues (no abort).
        let receipts = std::sync::Mutex::new(ReceiptStore::<RetainedReceipt>::new(8));
        let result = dispatch_or_reload(
            std::future::ready(Vec::<DispatchOutcome>::new()),
            std::future::pending::<EpochEnd>(),
            &receipts,
        )
        .await;
        assert!(result.is_none());
    }

    fn an_event() -> AgentEvent {
        AgentEvent {
            id: "e1".to_string(),
            event_type: "file.opened".to_string(),
            fields: std::collections::BTreeMap::new(),
            external_content: false,
        }
    }

    #[tokio::test]
    async fn recovery_is_taken_before_a_buffered_event() {
        use std::future::{pending, ready};
        // A ready recovery and a buffered event: recovery wins (so it is never
        // starved behind events). This is safe because recovery rebuilds in
        // place (`RearmProvider`), keeping the subscription, so the buffered
        // event is still delivered after the rebuild.
        let step = next_dispatch_step(
            pending::<EpochEnd>(),
            ready(true),
            pending::<Option<ManualInvoke>>(),
            ready(Some(an_event())),
        )
        .await;
        assert!(matches!(step, DispatchStep::Recovered));
    }

    #[tokio::test]
    async fn an_event_is_taken_when_recovery_is_pending() {
        use std::future::{pending, ready};
        let step = next_dispatch_step(
            pending::<EpochEnd>(),
            pending::<bool>(),
            pending::<Option<ManualInvoke>>(),
            ready(Some(an_event())),
        )
        .await;
        assert!(matches!(step, DispatchStep::Event(_)));
    }

    #[tokio::test]
    async fn a_config_change_wins_over_recovery_and_a_buffered_event() {
        use std::future::{pending, ready};
        // Revocation safety: a config change beats both a ready recovery and a
        // buffered event.
        let step = next_dispatch_step(
            ready(EpochEnd::Reload),
            ready(true),
            pending::<Option<ManualInvoke>>(),
            ready(Some(an_event())),
        )
        .await;
        assert!(matches!(step, DispatchStep::End(EpochEnd::Reload)));
    }

    #[tokio::test]
    async fn a_lost_name_race_during_recovery_supersedes_the_epoch() {
        use std::future::{pending, ready};
        // recover_connection resolves `false` when another instance took the
        // singleton name during the bus outage: the epoch must end as
        // Superseded (the daemon then exits) rather than re-arm or dispatch.
        let step = next_dispatch_step(
            pending::<EpochEnd>(),
            ready(false),
            pending::<Option<ManualInvoke>>(),
            ready(Some(an_event())),
        )
        .await;
        assert!(matches!(step, DispatchStep::End(EpochEnd::Superseded)));
    }

    #[tokio::test]
    async fn a_manual_invoke_is_taken_when_pending_recovery_and_no_event() {
        use std::future::pending;
        // A user invoke is picked (it is not starved behind the event source).
        let (respond, _rx) = tokio::sync::oneshot::channel();
        let invoke = ManualInvoke { name: "tidy".to_string(), respond };
        let step = next_dispatch_step(
            pending::<EpochEnd>(),
            pending::<bool>(),
            std::future::ready(Some(invoke)),
            pending::<Option<AgentEvent>>(),
        )
        .await;
        assert!(matches!(step, DispatchStep::Manual(_)));
    }

    #[tokio::test]
    async fn a_config_change_wins_over_a_manual_invoke() {
        use std::future::{pending, ready};
        // Revocation safety holds for the manual arm too: a config change beats
        // a ready user invoke.
        let (respond, _rx) = tokio::sync::oneshot::channel();
        let invoke = ManualInvoke { name: "tidy".to_string(), respond };
        let step = next_dispatch_step(
            ready(EpochEnd::Reload),
            pending::<bool>(),
            ready(Some(invoke)),
            pending::<Option<AgentEvent>>(),
        )
        .await;
        assert!(matches!(step, DispatchStep::End(EpochEnd::Reload)));
    }
}
