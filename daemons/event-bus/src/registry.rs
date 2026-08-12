use crate::proto::Event;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, warn};

/// How many events can queue up per consumer before we start dropping.
/// A consumer that cannot keep up will lose events rather than stalling the bus.
const CONSUMER_BUFFER: usize = 1024;

/// UID filter for a consumer. Determines which user's events are delivered.
#[derive(Debug, Clone, PartialEq)]
pub enum UidFilter {
    /// Receive events from all users (system consumers like graph-writer).
    All,
    /// Receive events only from this specific user.
    Exact(u32),
}

impl UidFilter {
    /// Parse a UID filter from the registration line.
    /// "*" means all users, a number means that specific UID.
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s == "*" {
            Ok(UidFilter::All)
        } else {
            s.parse::<u32>()
                .map(UidFilter::Exact)
                .map_err(|e| format!("invalid UID filter '{s}': {e}"))
        }
    }

    /// Check whether an event with the given UID passes this filter.
    /// System events (uid=0) always pass regardless of filter.
    pub fn accepts(&self, event_uid: u32) -> bool {
        // System events (uid=0) are delivered to all consumers.
        if event_uid == 0 {
            return true;
        }
        match self {
            UidFilter::All => true,
            UidFilter::Exact(uid) => event_uid == *uid,
        }
    }
}

/// A registered consumer: an async task that reads from a Unix socket
/// and wants to receive a filtered subset of events.
struct ConsumerEntry {
    id: String,
    /// Event type prefixes this consumer subscribed to.
    /// "file.opened" matches only that type.
    /// "file." and "file.*" both match all file events (prefix match); the
    /// second is the spelling a permission profile's `[event_bus]` scope uses,
    /// accepted here so a consumer can register the patterns it declared.
    /// "*" matches everything.
    subscribed_types: Vec<String>,
    /// UID filter: which user's events this consumer receives.
    uid_filter: UidFilter,
    /// The sending half of the per-consumer channel.
    sender: mpsc::Sender<Event>,
}

impl ConsumerEntry {
    fn matches(&self, event_type: &str) -> bool {
        self.subscribed_types.iter().any(|sub| {
            if sub == "*" {
                true
            } else if let Some(prefix) = sub.strip_suffix(".*") {
                // "file.*" is how a permission profile spells the same
                // wildcard (`pattern_matches` in sdk/permissions strips
                // exactly this suffix). The two surfaces used INVERTED
                // spellings: a profile's only wildcard form was a dead
                // literal here, matching no event type, so a consumer
                // registered from its own declared scope silently received
                // nothing. Accepting it costs nothing - no event type is
                // literally named "file.*" - and removes a trap that fails
                // quietly in the direction of no delivery.
                event_type.starts_with(prefix) && event_type[prefix.len()..].starts_with('.')
            } else if let Some(prefix) = sub.strip_suffix('.') {
                // "file." matches "file.opened", "file.closed", etc.
                event_type.starts_with(prefix)
            } else {
                sub == event_type
            }
        })
    }
}

/// Shared registry of all active consumers.
/// Wrapped in `Arc<RwLock<...>>` so it can be shared across async tasks.
/// Whether a topic keeps its last message for late subscribers.
///
/// `<domain>.state` - `power.state`, `network.state`, `audio.state` today. The
/// suffix is the whole rule, so a new state topic gets late-joiner semantics by
/// being named one, with nothing to register.
///
/// **Event topics keep nothing.** `file.opened` is something that happened;
/// replaying it to a subscriber who was not there would be telling them a file
/// is being opened now. A state topic is a snapshot of how things ARE, and the
/// only reason a subscriber misses it is that nothing has changed since they
/// connected - which is exactly when they most need to be told.
pub fn is_state_topic(topic: &str) -> bool {
    topic.ends_with(".state")
}

pub struct ConsumerRegistry {
    consumers: RwLock<Vec<ConsumerEntry>>,
    /// The last message of each state topic, replaced not accumulated.
    ///
    /// One per topic. A history is the graph's job; this is here so a consumer
    /// that connects between two changes is not told nothing at all.
    retained: RwLock<std::collections::HashMap<String, Event>>,
}

impl ConsumerRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            consumers: RwLock::new(Vec::new()),
            retained: RwLock::new(std::collections::HashMap::new()),
        })
    }

    /// Register a new consumer and return the receiving end of its channel.
    pub async fn register(
        self: &Arc<Self>,
        id: String,
        subscribed_types: Vec<String>,
        uid_filter: UidFilter,
    ) -> mpsc::Receiver<Event> {
        let (sender, receiver) = mpsc::channel(CONSUMER_BUFFER);
        let entry = ConsumerEntry {
            id: id.clone(),
            subscribed_types,
            uid_filter,
            sender,
        };

        // RETAINED FIRST, THEN CONSUMERS - the same order `dispatch` uses, and the
        // reason is deadlock rather than correctness. `dispatch` takes the
        // retained write lock, releases it, and only then takes the consumer read
        // lock; that release is what keeps the two paths from waiting on each
        // other. Relying on it means a later refactor holding the retained guard
        // across the delivery loop - to avoid a clone, say - would invert the
        // order against this function and deadlock the bus. Taking them in one
        // order everywhere costs nothing and removes the trap.
        //
        // Both are held across the replay AND the push, which is the point of
        // taking them here rather than on the push alone: without it a live change
        // arriving between the two either lands before the retained snapshot -
        // leaving the consumer holding stale state that looks newer - or lands
        // while the entry is not yet in the list and is missed outright.
        let retained = self.retained.read().await;
        let mut consumers = self.consumers.write().await;

        // Retained delivery is filtered by the SAME two checks `dispatch` makes,
        // which is what keeps it from being a way in. It cannot be more permissive
        // than a live delivery, because `subscribed_types` has already been through
        // the subscribe-scope gate in the socket handler by the time it arrives
        // here - a pattern the caller may not have was filtered out before this.
        for event in retained.values() {
            if !entry.matches(&event.r#type) || !entry.uid_filter.accepts(event.uid) {
                continue;
            }
            // The event keeps its ORIGINAL timestamp. It is last-known, never
            // current, and a consumer that reads it as current will paint stale
            // state as live - the timestamp is how it tells the difference.
            if entry.sender.try_send(event.clone()).is_err() {
                warn!(consumer_id = %id, topic = %event.r#type, "retained snapshot not delivered");
            }
        }

        consumers.push(entry);
        drop(consumers);
        drop(retained);
        debug!(consumer_id = %id, "consumer registered");
        receiver
    }

    /// Unregister a consumer by ID. Called when the consumer disconnects.
    pub async fn unregister(self: &Arc<Self>, id: &str) {
        let mut consumers = self.consumers.write().await;
        consumers.retain(|c| c.id != id);
        debug!(consumer_id = %id, "consumer unregistered");
    }

    /// Dispatch an event to all matching consumers.
    /// Checks both event type pattern AND UID filter.
    pub async fn dispatch(self: &Arc<Self>, event: &Event) {
        // Retained before delivered, so a consumer registering at this instant
        // gets this snapshot from one side or the other and never neither.
        if is_state_topic(&event.r#type) {
            self.retained
                .write()
                .await
                .insert(event.r#type.clone(), event.clone());
        }

        let consumers = self.consumers.read().await;

        for consumer in consumers.iter() {
            // Check event type pattern match.
            if !consumer.matches(&event.r#type) {
                continue;
            }

            // Check UID filter.
            if !consumer.uid_filter.accepts(event.uid) {
                continue;
            }

            match consumer.sender.try_send(event.clone()) {
                Ok(()) => {
                    debug!(
                        consumer_id = %consumer.id,
                        event_type = %event.r#type,
                        "dispatched event"
                    );
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    warn!(
                        consumer_id = %consumer.id,
                        event_type = %event.r#type,
                        "consumer buffer full, dropping event"
                    );
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    warn!(
                        consumer_id = %consumer.id,
                        "consumer channel closed unexpectedly"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn both_wildcard_spellings_select_the_same_events() {
        // The registry and sdk/permissions historically disagreed about how a
        // wildcard is written: "file." here, "file.*" in a profile. Each was a
        // dead literal in the other, so a consumer that registered the patterns
        // its own `[event_bus].subscribe` scope declared matched nothing and got
        // no events, with no error anywhere. Both spellings must select the same
        // set, or that silence comes back.
        let (tx, _rx) = mpsc::channel(1);
        let entry = |subs: Vec<&str>| ConsumerEntry {
            id: "c".to_string(),
            subscribed_types: subs.into_iter().map(String::from).collect(),
            uid_filter: UidFilter::All,
            sender: tx.clone(),
        };
        for spelling in ["file.", "file.*"] {
            let e = entry(vec![spelling]);
            assert!(e.matches("file.opened"), "{spelling} must match file.opened");
            assert!(e.matches("file.written"), "{spelling} must match file.written");
            assert!(!e.matches("window.focused"), "{spelling} must not match another namespace");
        }
        // A neighbouring namespace sharing the prefix is not swept in: "file.*"
        // must not match "filesystem.mounted".
        assert!(!entry(vec!["file.*"]).matches("filesystem.mounted"));
        // Exact types stay exact.
        assert!(entry(vec!["file.opened"]).matches("file.opened"));
        assert!(!entry(vec!["file.opened"]).matches("file.written"));
    }

    use super::*;

    fn make_event(event_type: &str, uid: u32) -> Event {
        Event {
            id: "01950000-0000-7000-8000-000000000001".to_string(),
            r#type: event_type.to_string(),
            timestamp: 1_000_000,
            source: "test".to_string(),
            pid: 1,
            origin: "session-test".to_string(),
            payload: vec![],
            uid,
            project_id: String::new(),
            authenticated_origin: String::new(),
        }
    }

    #[test]
    fn exact_match() {
        let entry = ConsumerEntry {
            id: "test".to_string(),
            subscribed_types: vec!["file.opened".to_string()],
            uid_filter: UidFilter::All,
            sender: mpsc::channel(1).0,
        };
        assert!(entry.matches("file.opened"));
        assert!(!entry.matches("file.closed"));
        assert!(!entry.matches("window.focused"));
    }

    #[test]
    fn prefix_match() {
        let entry = ConsumerEntry {
            id: "test".to_string(),
            subscribed_types: vec!["file.".to_string()],
            uid_filter: UidFilter::All,
            sender: mpsc::channel(1).0,
        };
        assert!(entry.matches("file.opened"));
        assert!(entry.matches("file.closed"));
        assert!(!entry.matches("window.focused"));
    }

    #[test]
    fn wildcard_match() {
        let entry = ConsumerEntry {
            id: "test".to_string(),
            subscribed_types: vec!["*".to_string()],
            uid_filter: UidFilter::All,
            sender: mpsc::channel(1).0,
        };
        assert!(entry.matches("file.opened"));
        assert!(entry.matches("window.focused"));
        assert!(entry.matches("anything"));
    }

    #[tokio::test]
    async fn dispatch_reaches_matching_consumer() {
        let registry = ConsumerRegistry::new();
        let mut receiver = registry
            .register("consumer-1".to_string(), vec!["file.opened".to_string()], UidFilter::All)
            .await;

        registry.dispatch(&make_event("file.opened", 1000)).await;

        let event_received = receiver.try_recv().expect("should have received event");
        assert_eq!(event_received.r#type, "file.opened");
    }

    #[tokio::test]
    async fn an_empty_subscription_list_receives_nothing() {
        // LOAD-BEARING for the `[event_bus].subscribe` enforce path: when a
        // consumer's every requested pattern is outside its declared scope, the
        // socket layer registers it with an EMPTY pattern list. `matches` is an
        // `any()` over that list, so empty matches nothing - deny-all is
        // fail-CLOSED. A refactor that ever treats an empty list as a wildcard
        // would silently turn a fully-denied consumer into one that hears
        // everything, so pin the invariant here.
        let registry = ConsumerRegistry::new();
        let mut receiver = registry
            .register("denied-consumer".to_string(), vec![], UidFilter::All)
            .await;

        registry.dispatch(&make_event("file.opened", 1000)).await;
        registry.dispatch(&make_event("window.focused", 1000)).await;

        assert!(
            receiver.try_recv().is_err(),
            "a consumer with no permitted patterns must receive nothing"
        );
    }

    #[tokio::test]
    async fn dispatch_skips_non_matching_consumer() {
        let registry = ConsumerRegistry::new();
        let mut receiver = registry
            .register("consumer-1".to_string(), vec!["window.".to_string()], UidFilter::All)
            .await;

        registry.dispatch(&make_event("file.opened", 1000)).await;

        assert!(receiver.try_recv().is_err(), "should not have received event");
    }

    // ── UID Filtering Tests ──

    #[tokio::test]
    async fn test_uid_filtering_same_user() {
        let registry = ConsumerRegistry::new();
        let mut receiver = registry
            .register("c1".to_string(), vec!["*".to_string()], UidFilter::Exact(1000))
            .await;

        registry.dispatch(&make_event("file.opened", 1000)).await;

        assert!(receiver.try_recv().is_ok(), "same UID should be delivered");
    }

    #[tokio::test]
    async fn test_uid_filtering_different_user() {
        let registry = ConsumerRegistry::new();
        let mut receiver = registry
            .register("c1".to_string(), vec!["*".to_string()], UidFilter::Exact(1000))
            .await;

        registry.dispatch(&make_event("file.opened", 2000)).await;

        assert!(receiver.try_recv().is_err(), "different UID should be filtered");
    }

    #[tokio::test]
    async fn test_uid_filtering_system_events() {
        let registry = ConsumerRegistry::new();
        let mut receiver = registry
            .register("c1".to_string(), vec!["*".to_string()], UidFilter::Exact(1000))
            .await;

        // uid=0 is a system event, should reach all consumers.
        registry.dispatch(&make_event("schema.registered", 0)).await;

        assert!(receiver.try_recv().is_ok(), "system event (uid=0) should reach all consumers");
    }

    #[tokio::test]
    async fn test_wildcard_uid_filter() {
        let registry = ConsumerRegistry::new();
        let mut receiver = registry
            .register("c1".to_string(), vec!["*".to_string()], UidFilter::All)
            .await;

        registry.dispatch(&make_event("file.opened", 1000)).await;
        registry.dispatch(&make_event("file.opened", 2000)).await;
        registry.dispatch(&make_event("schema.registered", 0)).await;

        // All three should arrive.
        assert!(receiver.try_recv().is_ok());
        assert!(receiver.try_recv().is_ok());
        assert!(receiver.try_recv().is_ok());
    }

    #[test]
    fn uid_filter_parse() {
        assert_eq!(UidFilter::parse("*").unwrap(), UidFilter::All);
        assert_eq!(UidFilter::parse("1000").unwrap(), UidFilter::Exact(1000));
        assert_eq!(UidFilter::parse("0").unwrap(), UidFilter::Exact(0));
        assert!(UidFilter::parse("abc").is_err());
        assert!(UidFilter::parse("").is_err());
    }

    #[test]
    fn uid_filter_accepts() {
        let all = UidFilter::All;
        assert!(all.accepts(0));
        assert!(all.accepts(1000));
        assert!(all.accepts(2000));

        let exact = UidFilter::Exact(1000);
        assert!(exact.accepts(0));     // system events always pass
        assert!(exact.accepts(1000));  // matching UID
        assert!(!exact.accepts(2000)); // different UID
    }

    /// A state snapshot on the wire, with the timestamp that makes it readable
    /// as last-known rather than current.
    fn snapshot(topic: &str, uid: u32, timestamp: i64) -> Event {
        Event {
            r#type: topic.to_string(),
            uid,
            timestamp,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn a_late_subscriber_is_told_the_last_known_state() {
        // The gap this closes: net and audio publish only when something changes,
        // so anything that connects between two changes is told nothing at all and
        // has no way to ask. Being early was the only way to know.
        let reg = ConsumerRegistry::new();
        reg.dispatch(&snapshot("network.state", 0, 111)).await;

        let mut rx = reg
            .register("late".into(), vec!["network.".into()], UidFilter::All)
            .await;
        let got = rx.try_recv().expect("the retained snapshot is delivered");
        assert_eq!(got.r#type, "network.state");
        // Its OWN timestamp, not the moment of delivery. A consumer that reads it
        // as current would paint stale state as live, and this is what lets it
        // tell the difference.
        assert_eq!(got.timestamp, 111);
    }

    #[tokio::test]
    async fn an_event_topic_keeps_nothing() {
        // `file.opened` is something that happened. Replaying it to someone who
        // was not there says a file is being opened now, which is a lie about the
        // present rather than a fact about the past.
        let reg = ConsumerRegistry::new();
        reg.dispatch(&snapshot("file.opened", 0, 1)).await;
        let mut rx = reg
            .register("late".into(), vec!["file.".into()], UidFilter::All)
            .await;
        assert!(rx.try_recv().is_err(), "an event topic is not retained");
    }

    #[tokio::test]
    async fn a_topic_keeps_one_message_and_replaces_it() {
        // One per topic. A history is the graph's job; a bus that accumulated
        // would hand a late subscriber a replay of everything it missed.
        let reg = ConsumerRegistry::new();
        reg.dispatch(&snapshot("audio.state", 0, 1)).await;
        reg.dispatch(&snapshot("audio.state", 0, 2)).await;
        reg.dispatch(&snapshot("audio.state", 0, 3)).await;

        let mut rx = reg
            .register("late".into(), vec!["audio.".into()], UidFilter::All)
            .await;
        assert_eq!(rx.try_recv().expect("one snapshot").timestamp, 3);
        assert!(rx.try_recv().is_err(), "only the last one, not a history");
    }

    #[tokio::test]
    async fn retained_delivery_is_filtered_exactly_like_a_live_one() {
        // The requirement that matters: a retained message must not become a way
        // to receive what a live delivery would have refused. Both filters are
        // checked here, and the patterns arriving at `register` have already been
        // through the subscribe-scope gate in the socket handler, so this cannot
        // be more permissive than the live path.
        let reg = ConsumerRegistry::new();
        reg.dispatch(&snapshot("power.state", 1000, 5)).await;

        let mut wrong_uid = reg
            .register("other-user".into(), vec!["power.".into()], UidFilter::Exact(1001))
            .await;
        assert!(
            wrong_uid.try_recv().is_err(),
            "another user's snapshot is not handed over on subscribe"
        );

        let mut not_subscribed = reg
            .register("elsewhere".into(), vec!["file.".into()], UidFilter::All)
            .await;
        assert!(
            not_subscribed.try_recv().is_err(),
            "a topic the consumer did not ask for is not delivered"
        );
    }

    #[test]
    fn a_state_topic_is_named_one() {
        assert!(is_state_topic("power.state"));
        assert!(is_state_topic("network.state"));
        assert!(is_state_topic("audio.state"));
        assert!(!is_state_topic("file.opened"));
        // Not a suffix match on the word alone: this is a transition, not a
        // snapshot, and retaining it would replay a change as a condition.
        assert!(!is_state_topic("app.shortcut.state_changed"));
    }
}
