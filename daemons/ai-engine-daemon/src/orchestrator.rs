//! The autonomous-curator orchestrator (`pi-agent-adoption.md` §E), re-homed
//! from the native `ai-agent` into the engine daemon.
//!
//! The trigger spine is OURS: pi is request-driven, so the daemon subscribes the
//! event bus, coalesces a fire-storm, deterministically decides whether to act,
//! and drives either a daemon-direct deterministic curation or a bounded
//! ephemeral pi run. This module is built in increments; it starts with the
//! COALESCER, the piece §E flags as "doubly critical with pi" - never one pi-run
//! spawn per storm event.
//!
//! The coalescer is re-homed verbatim from the old `ai-agent` engine loop (gap
//! G1). It stays a pure, clock-injected, bounded structure so the whole dispatch
//! decision is testable without the event bus or a pi process.

use arlen_ai_skills::behaviour::{BehaviourKind, TriggerKind};
use arlen_ai_skills::loader::LoadedBehaviour;
use arlen_ai_skills::router::matching_behaviours;
use os_sdk::proto::{Event, FileOpenedPayload, WindowFocusedPayload};
use os_sdk::{EventConsumer, SubscribeError, UnixEventConsumer};
use prost::Message as _;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::{Duration, SystemTime};

/// The Event Bus consumer socket the orchestrator subscribes on. The daemon
/// resolves the real path from `ARLEN_CONSUMER_SOCKET` (this is the fallback).
pub const DEFAULT_CONSUMER_SOCKET: &str = "/run/arlen/event-bus-consumer.sock";

/// A decoded trigger event the orchestrator dispatches on: the event type, the
/// payload fields the router filters read, and the external-content origin flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerEvent {
    /// The bus event type, e.g. `file.opened`.
    pub event_type: String,
    /// The router-readable payload fields (path, app_id, ...).
    pub fields: BTreeMap<String, String>,
    /// Whether the event carries external (spoofable/attacker-influenced)
    /// content; escalates every triggered action to a confirmation.
    pub external_content: bool,
}

/// The Event Bus type filters to subscribe to: the event pattern of every
/// ENABLED, event-triggered behaviour. Schedule and manual triggers are not bus
/// subscriptions, so they contribute nothing here.
pub fn subscription_types(behaviours: &[LoadedBehaviour]) -> Vec<String> {
    let mut types: BTreeSet<String> = BTreeSet::new();
    for lb in behaviours {
        if lb.status.is_enabled() && lb.behaviour.manifest.trigger.kind == TriggerKind::Event {
            if let Some(event) = &lb.behaviour.manifest.trigger.event {
                types.insert(event.clone());
            }
        }
    }
    types.into_iter().collect()
}

/// Decode a bus `Event` envelope into a [`TriggerEvent`]: the type, the payload
/// fields the router/filters read, and the external-content flag. A payload that
/// fails to decode yields an event with no fields (rather than dropping it);
/// filters then fail closed on the missing fields.
pub fn decode_event(ev: Event) -> TriggerEvent {
    let mut fields = BTreeMap::new();
    match ev.r#type.as_str() {
        "file.opened" => {
            if let Ok(p) = FileOpenedPayload::decode(ev.payload.as_slice()) {
                fields.insert("path".to_string(), p.path);
                fields.insert("app_id".to_string(), p.app_id);
            }
        }
        "window.focused" => {
            if let Ok(p) = WindowFocusedPayload::decode(ev.payload.as_slice()) {
                fields.insert("app_id".to_string(), p.app_id);
                fields.insert("window_title".to_string(), p.window_title);
            }
        }
        // Other event types carry no router-readable fields yet; their payload
        // decoders are added as behaviours need them.
        _ => {}
    }
    TriggerEvent {
        // Fail-safe. `Event.source` is producer-supplied and spoofable (the bus
        // authenticates uid via SO_PEERCRED but not the origin), so it is NOT
        // trusted for the external-content gate: until the bus stamps an
        // authenticated origin class + S18-A tagging lands, every bus event is
        // treated as external, so any action it triggers requires confirmation.
        external_content: true,
        event_type: ev.r#type,
        fields,
    }
}

/// The orchestrator's trigger spine: an Event Bus subscription that yields
/// decoded [`TriggerEvent`]s. Wraps the os-sdk consumer (subscribe, framing,
/// auto-reconnect); the decode is pure and unit-tested, the subscription itself
/// is thin I/O exercised against a live bus.
pub struct EventBusSource {
    rx: tokio::sync::mpsc::Receiver<Event>,
}

impl EventBusSource {
    /// Subscribe to the given event-type filters on the consumer socket. Fails if
    /// the bus is unreachable after the SDK's eager-retry budget.
    pub async fn subscribe(
        socket_path: impl Into<String>,
        types: Vec<String>,
    ) -> Result<Self, SubscribeError> {
        let consumer = UnixEventConsumer::new(socket_path);
        let rx = consumer.subscribe(types).await?;
        Ok(Self { rx })
    }

    /// The next decoded trigger event, or `None` when the bus connection closes.
    pub async fn recv(&mut self) -> Option<TriggerEvent> {
        self.rx.recv().await.map(decode_event)
    }
}

/// How a matched behaviour is executed (the §E split).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// Safe reversible DETERMINISTIC curation (a `kind: workflow` behaviour):
    /// runs daemon-direct, silent-immediate, ZERO tokens, NO pi. A workflow
    /// handler is deterministic and makes no model call.
    DeterministicCuration,
    /// A genuine LLM behaviour (`kind: agent`): a BOUNDED EPHEMERAL pi run with a
    /// least-authority tool set, every call gated and every write daemon-executed.
    PiRun,
}

/// One dispatched behaviour and how it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dispatch {
    /// The behaviour name (its coalescing + audit key).
    pub behaviour: String,
    /// Deterministic-curation vs an ephemeral pi run.
    pub route: Route,
}

/// The deterministic dispatch decision for one event: which enabled behaviours
/// matched and survived coalescing, each routed to daemon-direct curation or a
/// pi run, plus the ones that matched but were coalesced (a burst duplicate).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DispatchPlan {
    /// Behaviours to run now, with their route.
    pub dispatched: Vec<Dispatch>,
    /// Behaviours that matched but were coalesced away (for logging/metrics).
    pub coalesced: Vec<String>,
}

/// The deterministic dispatch decision (§E): match the event against the enabled
/// behaviours (the shared router), coalesce a fire-storm per behaviour, and route
/// each survivor by kind. Pure and clock-injected - NO event bus, NO pi spawn -
/// so the whole act-or-not decision is unit-testable. The caller drives the
/// routes: `DeterministicCuration` runs daemon-direct, `PiRun` spawns a bounded
/// ephemeral pi run.
pub fn decide(
    event_type: &str,
    fields: &BTreeMap<String, String>,
    external_content: bool,
    behaviours: &[LoadedBehaviour],
    coalescer: &mut Coalescer,
    now: SystemTime,
) -> DispatchPlan {
    let mut plan = DispatchPlan::default();
    for lb in matching_behaviours(event_type, fields, behaviours) {
        let name = lb.behaviour.manifest.name.clone();
        if coalescer.admit(&name, event_type, fields, external_content, now) {
            let route = match lb.behaviour.manifest.kind {
                BehaviourKind::Workflow => Route::DeterministicCuration,
                BehaviourKind::Agent => Route::PiRun,
            };
            plan.dispatched.push(Dispatch { behaviour: name, route });
        } else {
            plan.coalesced.push(name);
        }
    }
    plan
}

/// Default per-behaviour coalescing window. A burst of identical events for one
/// behaviour within this window fires it once. Short by design: long enough to
/// collapse a "x100 in a second" storm, short enough not to suppress a
/// deliberate re-trigger seconds later.
pub const DEFAULT_COALESCE_WINDOW: Duration = Duration::from_secs(1);

/// Hard cap on the coalescer's tracking map, so a storm of DISTINCT events (many
/// unique paths in one window, e.g. a build or a `find`) cannot grow it without
/// bound. At the cap, stale entries are pruned; if it is still full of fresh
/// distinct events the map is cleared (coalescing forgets recent entries, never
/// dropping a distinct dispatch).
pub const MAX_COALESCE_ENTRIES: usize = 4096;

/// Per-behaviour burst coalescer. Collapses a storm of identical
/// `(behaviour, event_type, fields, external_content)` events into one dispatch
/// per [`window`](Self::new), so an autonomous behaviour fires once per burst
/// rather than once per event. Doubly critical under pi: a storm must never fan
/// out into one bounded pi run per event.
pub struct Coalescer {
    window: Duration,
    /// Per-process random seed for the key digest, so a producer cannot craft a
    /// colliding key without knowing it.
    hasher: std::collections::hash_map::RandomState,
    /// Key digest to the time that (behaviour, event) was last admitted.
    seen: HashMap<u64, SystemTime>,
}

impl Coalescer {
    /// A coalescer with the given burst window.
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            hasher: std::collections::hash_map::RandomState::new(),
            seen: HashMap::new(),
        }
    }

    /// The fixed-size digest a dispatch is coalesced on: the behaviour, the event
    /// type, the (sorted) fields and the external-content bit, hashed under a
    /// per-process seed so a producer cannot forge a colliding key.
    fn digest(
        &self,
        behaviour: &str,
        event_type: &str,
        fields: &BTreeMap<String, String>,
        external_content: bool,
    ) -> u64 {
        use std::hash::{BuildHasher as _, Hash as _, Hasher as _};
        let mut h = self.hasher.build_hasher();
        behaviour.hash(&mut h);
        event_type.hash(&mut h);
        // A BTreeMap hashes in sorted key order, so the same fields always digest
        // identically.
        fields.hash(&mut h);
        external_content.hash(&mut h);
        h.finish()
    }

    /// Decide whether to dispatch the keyed event at `now`, recording the time
    /// when it admits. Returns `true` (dispatch) when the key is new or its last
    /// dispatch is older than the window; `false` (coalesce) when a dispatch
    /// happened within the window. The window is measured from the first dispatch
    /// of a burst, not extended by each coalesced duplicate, so a sustained stream
    /// fires once per window rather than being suppressed forever.
    pub fn admit(
        &mut self,
        behaviour: &str,
        event_type: &str,
        fields: &BTreeMap<String, String>,
        external_content: bool,
        now: SystemTime,
    ) -> bool {
        let key = self.digest(behaviour, event_type, fields, external_content);
        let window = self.window;
        // Bound cost and memory. The common case (a small map) does no scan:
        // expiry is lazy, per key, on access below. Only when the map has grown to
        // the cap do we prune stale entries in one pass; if it is still full of
        // fresh distinct events, clear it entirely. Clearing only forgets recent
        // entries, so at worst a few duplicates slip through afterwards
        // (over-dispatch, never a dropped distinct event), while per-event cost
        // stays amortised O(1) and the map stays bounded under a hostile producer.
        if self.seen.len() >= MAX_COALESCE_ENTRIES {
            self.seen.retain(|_, last| {
                now.duration_since(*last)
                    .map(|elapsed| elapsed < window)
                    .unwrap_or(false)
            });
            if self.seen.len() >= MAX_COALESCE_ENTRIES {
                self.seen.clear();
            }
        }
        match self.seen.entry(key) {
            std::collections::hash_map::Entry::Occupied(mut slot) => {
                // Within the window: coalesce, without refreshing (the window is
                // measured from the first dispatch of a burst, not extended by each
                // duplicate). Expired, or future-stamped after a backwards clock
                // move: treat as stale, refresh to `now` and admit, rather than
                // suppressing the event past the window.
                let within = now
                    .duration_since(*slot.get())
                    .map(|elapsed| elapsed < window)
                    .unwrap_or(false);
                if within {
                    false
                } else {
                    slot.insert(now);
                    true
                }
            }
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(now);
                true
            }
        }
    }

    /// The number of tracked keys (for tests / introspection).
    #[cfg(test)]
    pub fn tracked(&self) -> usize {
        self.seen.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn a_burst_of_identical_events_fires_once_per_window() {
        let mut c = Coalescer::new(Duration::from_secs(1));
        let t0 = SystemTime::UNIX_EPOCH;
        let f = fields(&[("path", "/a.rs")]);
        // First admits; the rest of the burst within the window coalesce.
        assert!(c.admit("auto-tag", "file.opened", &f, false, t0));
        assert!(!c.admit("auto-tag", "file.opened", &f, false, t0 + Duration::from_millis(100)));
        assert!(!c.admit("auto-tag", "file.opened", &f, false, t0 + Duration::from_millis(900)));
        // Past the window (measured from the first dispatch), it fires again.
        assert!(c.admit("auto-tag", "file.opened", &f, false, t0 + Duration::from_millis(1100)));
    }

    #[test]
    fn distinct_events_each_dispatch() {
        let mut c = Coalescer::new(Duration::from_secs(1));
        let t = SystemTime::UNIX_EPOCH;
        // Different fields, different behaviour, and different external bit are all
        // distinct keys - none coalesce against the others.
        assert!(c.admit("auto-tag", "file.opened", &fields(&[("path", "/a.rs")]), false, t));
        assert!(c.admit("auto-tag", "file.opened", &fields(&[("path", "/b.rs")]), false, t));
        assert!(c.admit("summarise", "file.opened", &fields(&[("path", "/a.rs")]), false, t));
        assert!(c.admit("auto-tag", "file.opened", &fields(&[("path", "/a.rs")]), true, t));
    }

    #[test]
    fn field_order_does_not_affect_the_key() {
        let mut c = Coalescer::new(Duration::from_secs(1));
        let t = SystemTime::UNIX_EPOCH;
        assert!(c.admit("b", "e", &fields(&[("a", "1"), ("b", "2")]), false, t));
        // The same fields in the other insertion order digest identically -> coalesce.
        assert!(!c.admit("b", "e", &fields(&[("b", "2"), ("a", "1")]), false, t));
    }

    #[test]
    fn a_backwards_clock_move_admits_rather_than_suppresses() {
        let mut c = Coalescer::new(Duration::from_secs(1));
        let f = fields(&[("path", "/a.rs")]);
        let t1 = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
        assert!(c.admit("b", "e", &f, false, t1));
        // A now EARLIER than the recorded time (clock stepped back): the duration
        // is an error, treated as stale -> admit, never suppress past the window.
        assert!(c.admit("b", "e", &f, false, SystemTime::UNIX_EPOCH + Duration::from_secs(5)));
    }

    #[test]
    fn a_sustained_distinct_storm_stays_bounded() {
        let mut c = Coalescer::new(Duration::from_secs(1));
        let t = SystemTime::UNIX_EPOCH;
        // Push well past the cap with distinct keys, all within one window.
        for i in 0..(MAX_COALESCE_ENTRIES + 500) {
            let f = fields(&[("path", &format!("/f{i}.rs"))]);
            c.admit("b", "e", &f, false, t);
        }
        // The map never exceeds the cap (pruned/cleared), so a hostile producer
        // cannot grow it without bound.
        assert!(c.tracked() <= MAX_COALESCE_ENTRIES);
    }

    use arlen_ai_skills::loader::{DisableReason, LoadedBehaviour, Provenance, Status};

    fn behaviour(name: &str, kind: &str, extra: &str, event: &str, status: Status) -> LoadedBehaviour {
        let src = format!(
            "---\nname: {name}\ndescription: d\nkind: {kind}\n{extra}trigger:\n  type: event\n  event: {event}\n---\nDo the thing.\n"
        );
        LoadedBehaviour {
            behaviour: arlen_ai_skills::behaviour::parse(&src).expect("valid SKILL.md"),
            provenance: Provenance::BuiltIn,
            dir: std::path::PathBuf::from("/test").join(name),
            status,
        }
    }

    fn workflow(name: &str, event: &str) -> LoadedBehaviour {
        behaviour(name, "workflow", "handler: h\n", event, Status::Enabled)
    }

    fn agent(name: &str, event: &str) -> LoadedBehaviour {
        // A complete valid agent frontmatter (an agent behaviour requires reads,
        // mode, budget, a terminal condition and a body), mirroring meeting-prep.
        let src = format!(
            "---\nname: {name}\ndescription: d\nkind: agent\nreads: project\nmode: suggest\n\
             trigger:\n  type: event\n  event: {event}\nbudget:\n  max_steps: 10\n  \
             max_tokens: 12000\n  max_wall_ms: 15000\nterminal:\n  done: silent\n---\nDo the thing.\n"
        );
        LoadedBehaviour {
            behaviour: arlen_ai_skills::behaviour::parse(&src).expect("valid agent SKILL.md"),
            provenance: Provenance::BuiltIn,
            dir: std::path::PathBuf::from("/test").join(name),
            status: Status::Enabled,
        }
    }

    #[test]
    fn decide_routes_workflow_direct_and_agent_to_pi() {
        let behaviours = vec![workflow("auto-tag", "file.opened"), agent("meeting-prep", "file.opened")];
        let mut c = Coalescer::new(Duration::from_secs(1));
        let plan = decide("file.opened", &fields(&[("path", "/a.rs")]), false, &behaviours, &mut c, SystemTime::UNIX_EPOCH);
        assert_eq!(plan.dispatched.len(), 2);
        let route = |n: &str| plan.dispatched.iter().find(|d| d.behaviour == n).map(|d| d.route);
        assert_eq!(route("auto-tag"), Some(Route::DeterministicCuration));
        assert_eq!(route("meeting-prep"), Some(Route::PiRun));
        assert!(plan.coalesced.is_empty());
    }

    #[test]
    fn decide_skips_non_matching_and_disabled_behaviours() {
        let behaviours = vec![
            workflow("auto-tag", "file.opened"),
            workflow("other", "window.focused"), // wrong event type
            behaviour("off", "workflow", "handler: h\n", "file.opened", Status::Disabled(DisableReason::NotEnabledInSettings)),
        ];
        let mut c = Coalescer::new(Duration::from_secs(1));
        let plan = decide("file.opened", &fields(&[("path", "/a.rs")]), false, &behaviours, &mut c, SystemTime::UNIX_EPOCH);
        // Only the enabled, matching workflow dispatches.
        assert_eq!(plan.dispatched.len(), 1);
        assert_eq!(plan.dispatched[0].behaviour, "auto-tag");
    }

    #[test]
    fn decide_coalesces_a_burst_per_behaviour() {
        let behaviours = vec![workflow("auto-tag", "file.opened")];
        let mut c = Coalescer::new(Duration::from_secs(1));
        let f = fields(&[("path", "/a.rs")]);
        let t = SystemTime::UNIX_EPOCH;
        let first = decide("file.opened", &f, false, &behaviours, &mut c, t);
        assert_eq!(first.dispatched.len(), 1);
        // The same event again within the window is coalesced, not re-dispatched.
        let second = decide("file.opened", &f, false, &behaviours, &mut c, t + Duration::from_millis(200));
        assert!(second.dispatched.is_empty());
        assert_eq!(second.coalesced, vec!["auto-tag".to_string()]);
    }

    #[test]
    fn decode_event_reads_file_opened_fields_and_flags_external() {
        let payload = FileOpenedPayload {
            path: "~/Repositories/foo.rs".to_string(),
            app_id: "org.arlen.editor".to_string(),
            flags: 0,
        }
        .encode_to_vec();
        let ev = Event {
            id: "e1".to_string(),
            r#type: "file.opened".to_string(),
            source: "ebpf".to_string(),
            payload,
            ..Default::default()
        };
        let t = decode_event(ev);
        assert_eq!(t.event_type, "file.opened");
        assert_eq!(t.fields.get("path").unwrap(), "~/Repositories/foo.rs");
        assert_eq!(t.fields.get("app_id").unwrap(), "org.arlen.editor");
        // Fail-safe: every bus event is external until an authenticated origin lands.
        assert!(t.external_content);
    }

    #[test]
    fn decode_event_unknown_type_has_no_fields() {
        let ev = Event { id: "e2".to_string(), r#type: "process.started".to_string(), source: "x".to_string(), ..Default::default() };
        let t = decode_event(ev);
        assert_eq!(t.event_type, "process.started");
        assert!(t.fields.is_empty());
        assert!(t.external_content);
    }

    #[test]
    fn subscription_types_are_the_enabled_event_triggers() {
        let behaviours = vec![
            workflow("auto-tag", "file.opened"),
            agent("meeting-prep", "calendar.event.upcoming"),
            behaviour("off", "workflow", "handler: h\n", "window.focused", Status::Disabled(DisableReason::NotEnabledInSettings)),
        ];
        let mut types = subscription_types(&behaviours);
        types.sort();
        // Only the enabled event-triggered behaviours contribute; the disabled one
        // does not subscribe.
        assert_eq!(types, vec!["calendar.event.upcoming".to_string(), "file.opened".to_string()]);
    }
}
