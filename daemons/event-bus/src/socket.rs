use crate::proto::Event;
use crate::registry::{ConsumerRegistry, UidFilter};
use crate::validation;
use anyhow::Result;
use prost::Message;
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tracing::{debug, error, info, warn};

/// Start both the producer socket and the consumer socket concurrently.
/// Both run forever; if either exits the daemon exits.
pub async fn listen(producer_path: &str, consumer_path: &str, registry: Arc<ConsumerRegistry>) -> Result<()> {
    tokio::try_join!(
        listen_producers(producer_path, registry.clone()),
        listen_consumers(consumer_path, registry),
    )?;
    Ok(())
}

/// Accept incoming producer connections and dispatch their events to the registry.
async fn listen_producers(path: &str, registry: Arc<ConsumerRegistry>) -> Result<()> {
    let listener = bind_socket(path)?;
    info_socket("producer", path);

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let registry = registry.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_producer(stream, registry).await {
                        error!("producer connection error: {e}");
                    }
                });
            }
            Err(e) => error!("producer accept error: {e}"),
        }
    }
}

/// Accept incoming consumer connections, register them, and forward matching events.
async fn listen_consumers(path: &str, registry: Arc<ConsumerRegistry>) -> Result<()> {
    let listener = bind_socket(path)?;
    info_socket("consumer", path);

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let registry = registry.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_consumer(stream, registry).await {
                        error!("consumer connection error: {e}");
                    }
                });
            }
            Err(e) => error!("consumer accept error: {e}"),
        }
    }
}

/// Extract the peer UID from a Unix stream via `SO_PEERCRED`. `None` on a
/// credential-read error, so the caller drops the connection rather than fall
/// back to a trusted identity: an unreadable peer cred fails CLOSED, never open
/// to uid 0 (root, which would skip the restamp and bypass every consumer
/// filter). On a connected `AF_UNIX` stream the kernel fixes peercred at connect,
/// so this error path is not normally reachable; it is the fail-safe default.
fn peer_uid(stream: &UnixStream) -> Option<u32> {
    stream.peer_cred().ok().map(|cred| cred.uid())
}

/// Resolve whether the connected peer is an attested system-tier producer.
///
/// The peer's PID comes from `SO_PEERCRED` (kernel-attested, not self-declared);
/// its `/proc/<pid>/exe` install path is classified by [`detect_tier`]. A
/// system-tier producer is a root-owned binary under `/usr/bin/arlen-*` or
/// `/usr/lib/arlen/...` (the eBPF kernel-layer, the compositor, the daemons).
///
/// Only the system tier is exempt from the EBK-2 uid restamp: the kernel-layer
/// observes the whole machine and legitimately forwards events stamped with the
/// *observed* process's uid, so overwriting that with its own peercred uid would
/// collapse every kernel event onto one user and break per-user routing. Any
/// resolution failure (no peercred, unreadable `/proc`) returns `false`
/// (non-system), the fail-safe: a producer that cannot prove a system identity
/// has its uid restamped from peercred.
fn peer_tier(stream: &UnixStream) -> Option<arlen_permissions::AppTier> {
    let cred = stream.peer_cred().ok()?;
    let pid = cred.pid()?;
    let exe = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    Some(arlen_permissions::detect_tier(&exe))
}

/// What the bus could learn about a connected peer.
///
/// The two failure shapes are kept apart on purpose. Shadow mode exists to say
/// what to write BEFORE enforce starts rejecting, and "no scope" alone does not
/// say that: a peer we cannot name needs an identity fix, while a peer we CAN
/// name needs a profile with that exact filename. Collapsing both into one
/// unresolved-looking log made every would-deny point at the wrong repair.
enum PeerScope {
    /// Named, with a profile whose `[event_bus]` scope decides the verdict.
    Profiled(String, Box<arlen_permissions::PermissionProfile>),
    /// Named, but no profile loaded - so no declared scope. The name is the
    /// filename the operator has to create.
    NoProfile(String),
    /// Not attributable to any app id.
    Unresolved,
}

/// Everything the bus decides about a peer, from ONE resolution of its pid.
///
/// It used to take three: `peer_tier` read `/proc/<pid>/exe`, then
/// `peer_app_profile` read the pid again, then the scope check read it a third
/// time. Beyond the waste, the exemption verdict was assembled from two
/// independent reads - so a pid recycled between them would decide "is this
/// System?" about one process and "does it declare a subscribe list?" about
/// another, and the answer would be about no process at all. Narrow, but the
/// kind of narrow that is only ever found afterwards.
///
/// Resolving once also gives the registration log the attested name. The bus knew
/// the peer and printed the self-declared `consumer_id`, which for anything using
/// the SDK default is `os-sdk-unknown-<uuid>` - so a boot journal could not answer
/// "did the anomaly detector subscribe?" without matching pattern lists by hand
/// (measured against the 12 Aug boot, which is exactly how it was answered).
struct Peer {
    scope: PeerScope,
    tier: Option<arlen_permissions::AppTier>,
}

impl Peer {
    fn resolve(stream: &UnixStream) -> Self {
        Self {
            scope: peer_app_profile(stream),
            tier: peer_tier(stream),
        }
    }

    fn is_system(&self) -> bool {
        self.tier == Some(arlen_permissions::AppTier::System)
    }
}

impl PeerScope {
    /// The declared event-bus scope, if any. Both failure shapes are "none
    /// declared", which is what the publish/subscribe checks act on.
    fn event_bus(&self) -> Option<&arlen_permissions::EventBusPermissions> {
        match self {
            Self::Profiled(_, p) => Some(&p.event_bus),
            _ => None,
        }
    }

    /// The app id for logs, or a marker naming which failure it was.
    fn app_id(&self) -> &str {
        match self {
            Self::Profiled(id, _) | Self::NoProfile(id) => id,
            Self::Unresolved => "<unresolved>",
        }
    }

    /// The kernel-attested app id to stamp as the event's authenticated origin,
    /// or empty when the peer could not be attributed. Unlike [`app_id`](Self::app_id)
    /// this never returns a diagnostic sentinel: an unresolved peer stamps empty,
    /// so a consumer's origin classifier treats it as un-attested (external),
    /// fail-closed.
    fn authenticated_origin(&self) -> &str {
        match self {
            Self::Profiled(id, _) | Self::NoProfile(id) => id,
            Self::Unresolved => "",
        }
    }

    /// What an operator has to do about a would-deny from this peer.
    fn remedy(&self) -> &'static str {
        match self {
            Self::Profiled(..) => "declared scope does not cover it",
            Self::NoProfile(_) => "no profile for this app id",
            Self::Unresolved => "peer identity unresolved",
        }
    }
}

/// What the bus does with a producer, split into the two decisions that were
/// previously entangled.
struct PublishDecision<'a> {
    /// The identity stamped as `authenticated_origin`. Never depends on the tier.
    origin: &'a str,
    /// Whether the declared `[event_bus].publish` list is applied. Only this
    /// depends on the tier.
    hold_to_scope: bool,
}

/// The components whose publishing is not held to a declared list, BY NAME.
///
/// Exemption is a property of originating system events, not of where a binary
/// is installed. These two observe the machine and are the sole source of what
/// they emit: the compositor is where window events come from, and the kernel
/// layer forwards what it sees from eBPF stamped with the observed process's
/// uid. Neither could be granted less by a profile than it already holds, so a
/// list for them would be decoration.
///
/// IT USED TO BE THE TIER, and that was too coarse in a way no boot could see.
/// `hold_to_scope = !is_system` made every first-party app exempt, because
/// `detect_tier` calls anything under `/usr/lib/arlen/` System - so `apps/files`
/// publishing whatever it liked wore the compositor's exemption. Worse, it made
/// the publish half structurally unmeasurable: no boot could ever produce a
/// publish denial for an app, so a profile's `publish` list was documentation
/// that could quietly be wrong forever.
///
/// Everything else - first-party apps, the daemons, the probes - is held to its
/// list. An addition here should be argued in the same terms: does this
/// component ORIGINATE the events, and would a grant be able to narrow it?
///
/// `compositor` IS VERIFIED and `kernel-layer` IS NOT, which matters more than it
/// looks. The first was read off a boot journal (`/usr/bin/arlen-compositor`,
/// resolver rule (2), the bus logs `app_id="compositor"`). The second is a name I
/// chose: the kernel layer ships no binary, no unit and no install path in this
/// tree, so nothing has ever resolved it and this entry currently exempts
/// nobody.
///
/// THE DAY IT SHIPS, CHECK THE ID BEFORE TRUSTING THIS. If its binary lands
/// somewhere rule (2) does not cover - `/usr/lib/arlen/libexec/`, as most daemons
/// do - it resolves to something else entirely, this entry misses, and it is held
/// to a profile that does not exist. That means every eBPF file, process and
/// network event dropped at the bus, silently, which is the whole Knowledge Graph
/// going quiet with nothing in any log to say why.
///
/// And nothing else will catch it: the kernel layer writes framed protobuf to the
/// producer socket directly rather than through the SDK emitter, so
/// `check-emitters-declared.py` cannot see it either.
/// The largest single event the bus will read, on any of its sockets.
///
/// Named rather than repeated: the producer path checked `1024 * 1024` inline,
/// and the upstream forwarder needs the same ceiling for the same reason - a
/// length prefix is attacker-supplied on both, and allocating from it before
/// bounding it is how a four-byte header becomes a memory limit.
const MAX_EVENT_SIZE: usize = 1024 * 1024;

const PUBLISH_ORIGINATORS: &[&str] = &["compositor", "kernel-layer"];

/// The producers allowed to say an event was about SOMEONE ELSE.
///
/// A whole-machine observer must: the eBPF kernel layer watches every process on
/// the box and forwards each observation stamped with the uid of the task it
/// observed, so restamping those onto the observer's own uid would collapse every
/// kernel event onto one user and make per-user routing meaningless.
///
/// Nobody else may, and this list is how that is decided since 15 Aug. It used to
/// follow the install TIER, which is the same over-broad shape
/// `PUBLISH_ORIGINATORS` above was written to replace: every root-owned binary
/// under `/usr/bin/arlen-*` counts as system tier, so the shell, the dogfood tool
/// and the compositor were all exempt from restamping too - and none of them
/// observes anything but its own session.
///
/// MEASURED on the 15 Aug boot, which is what turned this from a shape into a
/// defect: 17 `file.opened` and one `window.focus_left` arrived carrying uid 0,
/// from `dogfood` and `dev.arlen.desktop-shell`, both logged `system=true`. They
/// do not set the field, so it takes the protobuf default - and 0 is the value the
/// consumer filter short-circuits on, so those events were delivered to every
/// consumer on the machine as though root had done them.
///
/// The compositor is a `PUBLISH_ORIGINATORS` member and deliberately NOT one here:
/// originating an event and attributing it to another user are different powers.
/// Its window and session events are about the session it runs in, so its own uid
/// is the right answer for them.
const UID_OBSERVERS: &[&str] = &["kernel-layer"];

/// Whether this peer may stamp an event with a uid other than its own.
fn observes_other_uids(app_id: &str) -> bool {
    UID_OBSERVERS.contains(&app_id)
}

/// Whether this peer's publishing is exempt from its declared list.
fn originates_system_events(app_id: &str) -> bool {
    PUBLISH_ORIGINATORS.contains(&app_id)
}

/// Decide both at once, so identity can reach only the one it is entitled to.
///
/// A function rather than two lines in the handler because the bug it replaces
/// was invisible in the handler: exempting a system producer by swapping its
/// scope for `PeerScope::Unresolved` reads as scoping, and silently also blanked
/// the stamped origin. Here the exemption can only touch `hold_to_scope`, and
/// `origin` is derived from the peer alone - so the entanglement cannot be
/// reintroduced by editing this, only by deleting it.
fn publish_decision(scope: &PeerScope) -> PublishDecision<'_> {
    PublishDecision {
        origin: scope.authenticated_origin(),
        hold_to_scope: !originates_system_events(scope.app_id()),
    }
}

/// Whether a failed profile load is a RESULT (this app simply has none) rather
/// than an ERROR that happens to look like one.
///
/// The distinction the unit-table measurement forced into the open: a lookup under
/// a WRONG id answers "no grants", and no grants is indistinguishable from
/// correctly-locked-down. So every naming mistake in this system presents as
/// security working. Absent is the one expected cause and stays quiet; a malformed
/// profile, an unreadable one, or an id that fails validation is a fault someone
/// has to see. Both still yield no scopes - this decides what gets said, not what
/// gets allowed.
fn profile_absence_is_a_result(e: &arlen_permissions::PermissionError) -> bool {
    matches!(e, arlen_permissions::PermissionError::NotFound { .. })
}

/// Resolve the connected peer from its kernel-attested pid.
fn peer_app_profile(stream: &UnixStream) -> PeerScope {
    let Ok(cred) = stream.peer_cred() else {
        return PeerScope::Unresolved;
    };
    // The profile belongs to the PEER's user, not to this daemon's. The bus runs
    // as root, so asking for its own would look under /var/lib/arlen/permissions/0
    // while the set that applies to the connecting user is filed under theirs
    // (decided 11 Aug: the uid names who the permissions are for, not who reads
    // them). Same file for every reader, which is the point.
    let peer_uid = cred.uid();
    let Some(app_id) = cred
        .pid()
        .and_then(|pid| u32::try_from(pid).ok())
        .and_then(|pid| arlen_permissions::identity::app_id_from_pid(pid).ok())
    else {
        return PeerScope::Unresolved;
    };
    match arlen_permissions::load_profile_for_user(peer_uid, &app_id) {
        Ok(profile) => PeerScope::Profiled(app_id, Box::new(profile)),
        // An app with no profile yet is a RESULT: it gets no scopes, which is the
        // right answer and needs no line. Every other cause is an ERROR wearing
        // that result's clothes - a malformed TOML, an unreadable file, an id we
        // resolved to something nobody filed a profile under - and all three used
        // to arrive here as the same silent `NoProfile`.
        //
        // That is the failure the unit-table measurement exposed in the general
        // case: a lookup under a WRONG id answers "no grants", which reads as
        // correctly-locked-down rather than misconfigured, so a daemon renamed by
        // accident presents as security working. The peer is still refused either
        // way - this changes nothing about the decision, only whether anyone can
        // tell which of the two happened.
        Err(e) if profile_absence_is_a_result(&e) => PeerScope::NoProfile(app_id),
        Err(e) => {
            warn!(
                app_id = %app_id,
                uid = peer_uid,
                "peer resolved but its profile could not be read, so it gets no \
                 scopes: {e}. This is NOT the same as an app with no profile - \
                 check the id is the one a profile is filed under"
            );
            PeerScope::NoProfile(app_id)
        }
    }
}

/// Whether the bus REJECTS an unauthorised publish/subscribe (enforce) or only
/// LOGS it (shadow). Defaults to shadow so the first-party `[event_bus]` scopes
/// can be verified against real traffic before the reject flip - the same
/// shadow/enforce cutover the stamped-identity strand uses. Set
/// `ARLEN_EVENT_BUS_ENFORCE=1` (or `true`) to reject.
/// Say which mode the scope gates are in, once, at startup.
///
/// Shadow mode exists to make the flip decidable by measurement rather than by
/// argument, and that only works if a reader can tell which mode produced the log
/// in front of them. Without this line the two states are distinguishable only by
/// a denial happening to occur - so a boot where nothing was refused looks
/// identical whether the switch is on, off, or was never read. Twice now the
/// recorded reason for this switch has been wrong in the same direction, both
/// times from reasoning instead of reading (planner, 12 Aug).
pub fn log_enforcement_mode() {
    if enforce_pubsub() {
        tracing::info!(
            "event-bus: publish and subscribe scopes ENFORCED (ARLEN_EVENT_BUS_ENFORCE)"
        );
    } else {
        tracing::info!(
            "event-bus: publish and subscribe scopes in shadow mode; denials are logged, not applied"
        );
    }
}

fn enforce_pubsub() -> bool {
    matches!(
        std::env::var("ARLEN_EVENT_BUS_ENFORCE").ok().as_deref(),
        Some("1" | "true")
    )
}

/// Whether `event_type` is within the peer's declared `[event_bus].publish`
/// scope. A `None` scope (unresolved caller / no declared scope) is not
/// permitted.
fn publish_allowed(
    scope: Option<&arlen_permissions::EventBusPermissions>,
    event_type: &str,
) -> bool {
    scope.is_some_and(|s| s.can_publish(event_type))
}

/// Apply a consumer's declared `[event_bus].subscribe` scope to its requested
/// patterns. Shadow mode (`enforce == false`) keeps every pattern verbatim so
/// delivery is unchanged while denied patterns are only logged; enforce mode
/// keeps only permitted patterns. A `None` scope is no declared scope: shadow
/// keeps everything, enforce keeps nothing.
fn permitted_subscriptions(
    requested: &[String],
    scope: Option<&arlen_permissions::EventBusPermissions>,
    enforce: bool,
) -> Vec<String> {
    if !enforce {
        return requested.to_vec();
    }
    requested
        .iter()
        .filter(|t| scope.is_some_and(|s| s.can_subscribe(t)))
        .cloned()
        .collect()
}

/// Handle a single producer connection.
/// Reads length-prefixed protobuf messages, stamps the UID from `SO_PEERCRED`,
/// validates them, and dispatches.
async fn handle_producer(mut stream: UnixStream, registry: Arc<ConsumerRegistry>) -> Result<()> {
    let Some(producer_uid) = peer_uid(&stream) else {
        warn!("could not read producer SO_PEERCRED, dropping connection");
        return Ok(());
    };
    // Resolved once at connect: peercred is fixed for the connection's life, so
    // the tier never changes mid-stream. Drives the EBK-2 uid-restamp exemption.
    let peer = Peer::resolve(&stream);
    let forwards_other_uids = observes_other_uids(peer.scope.app_id());
    // Producers are held to their declared `[event_bus].publish` scope unless
    // they are one of the named `PUBLISH_ORIGINATORS`. That list is the whole
    // exemption now; the tier no longer touches the publish side at all, because
    // install path is a name and originating events is the effect.
    //
    // `is_system_producer` below is still the UID-RESTAMP exemption, which is a
    // different question with a different answer and is deliberately left alone
    // here. (It has the same over-broad shape - every first-party app is exempt
    // from restamping too, while only the kernel layer forwards other uids'
    // events - but narrowing it changes who can attribute an event to whom, so
    // it belongs to the producer-trust decision, not to this one.)
    //
    // The exemption is PUBLISH-only and stays that way: a privileged producer is
    // not automatically a privileged consumer, which is why the subscribe side
    // above hangs its exemption on the declaration too.
    //
    // It is expressed by `decision.hold_to_scope` on the publish check below,
    // and ONLY there. This used to also swap in `PeerScope::Unresolved`
    // for a system producer, which the guard already made redundant - and being
    // redundant was not the same as being harmless. `Unresolved` is how the bus
    // says "I could not identify this peer", so reusing it to mean "identified,
    // and exempt" made one sentinel carry two meanings, and every reader of that
    // value got the wrong one:
    //
    //   - `authenticated_origin` went out EMPTY for every system producer. That
    //     field exists so a consumer can tell an internal first-party event from
    //     external content, and empty is the fail-closed "treat as external"
    //     value - so the compositor's window events and the kernel layer's file
    //     events, the most trusted producers on the machine, were the ones
    //     stamped least trusted.
    //   - the would-deny and drop logs named `<unresolved>` for a peer the bus
    //     had just resolved.
    //
    // `PeerScope`'s own doc warns against exactly this collapse one level down
    // ("no scope" and "no identity" are kept apart because merging them points
    // every repair at the wrong place). Same error, one level up.
    let publish_scope = peer.scope;
    let decision = publish_decision(&publish_scope);
    let enforce = enforce_pubsub();
    debug!(
        app_id = publish_scope.app_id(),
        uid = producer_uid,
        // Renamed with the exemption it reports: "system" described the install
        // tier, which is no longer what decides this, and a boot log saying
        // system=true about the shell is how the over-broad exemption stayed
        // invisible for so long.
        forwards_other_uids,
        "new producer connection"
    );

    loop {
        let mut len_buf = [0u8; 4];
        match stream.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                debug!("producer disconnected");
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        }

        let len = u32::from_be_bytes(len_buf) as usize;
        if len == 0 || len > MAX_EVENT_SIZE {
            warn!(len, "invalid message length, closing connection");
            return Ok(());
        }

        let mut buf = vec![0u8; len];
        stream.read_exact(&mut buf).await?;

        match Event::decode(buf.as_slice()) {
            Ok(mut event) => {
                // EBK-2: a non-system producer's uid is ALWAYS the
                // kernel-attested SO_PEERCRED uid, overwriting any self-declared
                // value, so a user app cannot stamp another user's uid to forge
                // the source of an event. The system tier is exempt: the eBPF
                // kernel-layer observes the whole machine and forwards events
                // stamped with the observed process's uid, which must survive
                // (overwriting it would collapse every kernel event onto the
                // kernel-layer's own uid and break per-user routing). A producer
                // whose identity could not be attested resolves as non-system,
                // so it is restamped — fail-safe.
                if !forwards_other_uids {
                    event.uid = producer_uid;
                }

                // Stamp the bus-resolved kernel-attested producer identity as the
                // authenticated origin, ALWAYS overwriting any producer-supplied
                // value (the bus is the trust anchor; `source` stays spoofable and
                // untrusted). Empty when the peer is unattributable, so a consumer's
                // origin classifier treats it as external, fail-closed. This is what
                // the AI engine reads to distinguish an internal first-party event
                // from external content, instead of confirming on every event.
                event.authenticated_origin = decision.origin.to_string();

                // Hold the producer to its declared publish scope unless it is
                // a named originator. Shadow mode logs a would-deny and still
                // dispatches; enforce mode drops the event.
                if decision.hold_to_scope
                    && !publish_allowed(publish_scope.event_bus(), &event.r#type)
                {
                    let app = publish_scope.app_id();
                    let remedy = publish_scope.remedy();
                    if enforce {
                        warn!(app_id = app, event_type = %event.r#type, remedy, "event-bus: publish denied, dropping event");
                        continue;
                    }
                    // Shadow is advisory - debug so a dev stack whose daemons run
                    // from target/ paths (no wired profile yet) does not flood the
                    // live logs. Turn on debug to audit would-denies before enforce.
                    debug!(app_id = app, event_type = %event.r#type, remedy, "event-bus: publish would be denied (shadow mode)");
                }

                match validation::validate(&event) {
                    Ok(()) => {
                        debug!(id = %event.id, event_type = %event.r#type, uid = event.uid, "received event");
                        registry.dispatch(&event).await;
                    }
                    // Name the producer and the type, as the deny line above does.
                    //
                    // A booted image dropped two events for a missing `origin`
                    // and the log said only that - so the event was lost AND the
                    // producer was unidentifiable, which makes the warning a report
                    // that something is wrong with no way to act on it. The bus
                    // knows the attested app id here; withholding it helps nobody.
                    //
                    // Metadata only: the id, the type and the attested producer. Not
                    // the payload - a malformed event is still the user's data.
                    Err(e) => warn!(
                        error = %e,
                        app_id = publish_scope.app_id(),
                        event_type = %event.r#type,
                        id = %event.id,
                        "dropping invalid event"
                    ),
                }
            }
            Err(e) => warn!(error = %e, "failed to decode event, dropping"),
        }
    }
}

/// Handle a single consumer connection.
///
/// The consumer sends a newline-delimited registration message:
///   Line 1: consumer-id
///   Line 2: event-type1,event-type2,...
///   Line 3: UID filter ("*" for all, or a numeric UID like "1000")
///
/// After registration, the bus writes length-prefixed protobuf Event messages
/// to the socket as they arrive.
/// Hold a consumer's uid filter to the uid the kernel attested for it.
///
/// The third registration line is a CLAIM, and until now it was honoured as
/// written: any consumer could register `*` and receive every user's events, or
/// name another user's uid outright. On a single-user image nothing exercised
/// that, which is exactly why it needed writing down rather than waiting for a
/// second user to find it.
///
/// It is also the rule the per-user bus design is built on - "the filter is
/// enforced with the attested uid of the connecting bus, never a claimed one" -
/// because that design routes a system observer's events to a user by the
/// SUBJECT's uid. A per-user bus that could ask for another user's events by
/// asking wrongly would make the whole arrangement decorative.
///
/// Root keeps what it claims. A uid-0 consumer is the system side: the graph
/// writer on a system deployment, and the forwarding a per-user bus subscribes
/// through. Everyone else is narrowed to their own uid, whatever they asked for -
/// narrowed rather than refused, because the honest reading of "give me
/// everything" from a user process is "give me everything of mine", and refusing
/// the connection would turn a scope question into an outage.
fn clamp_uid_filter(claimed: UidFilter, attested_uid: u32) -> UidFilter {
    if attested_uid == 0 {
        claimed
    } else {
        UidFilter::Exact(attested_uid)
    }
}

async fn handle_consumer(mut stream: UnixStream, registry: Arc<ConsumerRegistry>) -> Result<()> {
    debug!("new consumer connection");

    // Read registration: three newline-terminated strings.
    let consumer_id = read_line(&mut stream).await?;
    let types_line = read_line(&mut stream).await?;
    let uid_line = read_line(&mut stream).await?;

    let subscribed_types: Vec<String> = types_line
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let claimed_filter = UidFilter::parse(&uid_line).map_err(|e| anyhow::anyhow!(e))?;
    // Fail closed: an unreadable peer cred is not a reason to serve everything.
    let Some(consumer_uid) = peer_uid(&stream) else {
        warn!("event-bus: refusing a consumer whose peer credentials cannot be read");
        return Ok(());
    };
    let uid_filter = clamp_uid_filter(claimed_filter.clone(), consumer_uid);
    if uid_filter != claimed_filter {
        debug!(
            consumer_uid,
            claimed = ?claimed_filter,
            effective = ?uid_filter,
            "event-bus: consumer uid filter narrowed to its attested uid"
        );
    }

    // Hold a consumer to its declared `[event_bus].subscribe` scope - and DECLARING
    // is what decides that, not the tier.
    //
    // The system tier used to exempt a peer from both halves at once, and that
    // bundles two effects of which only one is usually earned. The compositor is
    // the case that separates them: it is the origin of every window and session
    // event, so restricting its PUBLISH buys nothing it does not already have, but
    // subscribing would hand it reach it has no claim to - another app's activity,
    // the graph's own traffic. A privileged producer is not automatically a
    // privileged consumer (planner, 12 Aug).
    //
    // So the exemption now hangs on the profile: a component that declares an
    // `[event_bus].subscribe` list is held to it whatever its tier, and one that
    // declares nothing keeps the old machine-wide view. That keeps the knowledge
    // daemon and the shell working unchanged - they observe by design and declare
    // nothing - while making "declared" the way any component, system or not, opts
    // into being bounded. A check can then require a declaration from the
    // components that should have one, which is a stronger guarantee than a tier
    // label because it is per-component and readable.
    let peer = Peer::resolve(&stream);
    let declares_subscribe = peer
        .scope
        .event_bus()
        .is_some_and(|s| s.declares_subscribe());
    let exempt = peer.is_system() && !declares_subscribe;
    let subscribed_types = if exempt {
        subscribed_types
    } else {
        let scope = &peer.scope;
        let ebus = scope.event_bus();
        let app = scope.app_id();
        let remedy = scope.remedy();
        let enforce = enforce_pubsub();
        for t in &subscribed_types {
            if !ebus.is_some_and(|s| s.can_subscribe(t)) {
                if enforce {
                    warn!(app_id = app, pattern = %t, remedy, "event-bus: subscribe denied, filtering pattern");
                } else {
                    debug!(app_id = app, pattern = %t, remedy, "event-bus: subscribe would be denied (shadow mode)");
                }
            }
        }
        permitted_subscriptions(&subscribed_types, ebus, enforce)
    };

    // `app_id` is the attested name and `consumer_id` the self-declared one; they
    // are logged side by side because they answer different questions. The first
    // says who connected, the second says what that peer calls itself - and when
    // the second is `os-sdk-unknown-<uuid>`, only the first is any use.
    debug!(
        app_id = peer.scope.app_id(),
        consumer_id = %consumer_id,
        subscribed = ?subscribed_types,
        uid_filter = ?uid_filter,
        "consumer registered"
    );

    let mut receiver = registry
        .register(consumer_id.clone(), subscribed_types, uid_filter)
        .await;

    // Forward events from the channel to the socket.
    while let Some(event) = receiver.recv().await {
        let encoded = event.encode_to_vec();
        let len = u32::try_from(encoded.len()).expect("event too large to encode").to_be_bytes();

        if stream.write_all(&len).await.is_err()
            || stream.write_all(&encoded).await.is_err()
        {
            break;
        }
    }

    registry.unregister(&consumer_id).await;
    debug!(consumer_id = %consumer_id, "consumer disconnected");
    Ok(())
}

/// Bind a Unix socket, removing any stale socket file first.
///
/// The socket is set mode 0666 so processes of any uid can connect: the event
/// bus is the system-wide funnel every Arlen process must reach (the user-uid
/// compositor/shell/apps as producers, the user-uid AI daemons as consumers),
/// while the daemon itself runs as a system service whose `bind` would otherwise
/// leave the socket 0755 (owner-only write) under systemd's 0022 umask, denying
/// every cross-uid `connect`. Socket ownership is NOT the trust boundary here:
/// the bus stamps each peer's uid from `SO_PEERCRED` at accept time, so a
/// world-connectable socket is safe and is the only mode consistent with a
/// system funnel serving user-uid clients.
fn bind_socket(path: &str) -> Result<UnixListener> {
    use std::os::unix::fs::PermissionsExt;
    if Path::new(path).exists() {
        std::fs::remove_file(path)?;
    }
    if let Some(parent) = Path::new(path).parent() {
        std::fs::create_dir_all(parent)?;
    }
    let listener = UnixListener::bind(path)?;
    // 0666 because the bus is a system daemon and every producer and consumer is a
    // different uid's session process; an owner-only socket would be reachable by
    // nothing it exists for. systemd's 0022 umask leaves `bind` owner-write, so the
    // mode is set rather than inherited. `knowledge` makes the same call with the
    // same reasoning attached; this site had it undocumented.
    //
    // The mode does not carry the security here. Producers are peer-credentialed
    // and a producer whose credential cannot be read is dropped, which is where the
    // trust boundary is.
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666))?;
    Ok(listener)
}

fn info_socket(label: &str, path: &str) {
    tracing::info!(socket = path, "listening for {label} connections");
}

/// Read a newline-terminated string from a Unix stream, up to 4096 bytes.
async fn read_line(stream: &mut UnixStream) -> Result<String> {
    let mut buf = Vec::with_capacity(256);
    loop {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte).await?;
        if byte[0] == b'\n' {
            break;
        }
        buf.push(byte[0]);
        if buf.len() > 4096 {
            anyhow::bail!("registration line too long");
        }
    }
    Ok(String::from_utf8(buf)?)
}

#[cfg(test)]
mod tests {

    /// Only a whole-machine observer may attribute an event to another user.
    ///
    /// The boot that prompted this had `dogfood` and `dev.arlen.desktop-shell`
    /// exempt from restamping because both install as root-owned binaries and so
    /// counted as system tier. Neither observes anything but its own session, and
    /// between them they put 18 uid-0 events on the bus - which the consumer
    /// filter then delivered to everyone.
    ///
    /// `<unresolved>` is in here on purpose: a peer the bus could not name must
    /// fall on the restamping side, so an identity failure cannot buy the power to
    /// speak for another user.
    #[test]
    fn only_a_named_observer_may_stamp_another_users_uid() {
        assert!(observes_other_uids("kernel-layer"));
        for id in [
            "dogfood",
            "dev.arlen.desktop-shell",
            "compositor",
            "knowledge",
            "<unresolved>",
        ] {
            assert!(
                !observes_other_uids(id),
                "{id} observes only its own session and must be restamped"
            );
        }
    }

    /// A user consumer cannot ask for another user's events, however it asks.
    ///
    /// The three shapes are the three ways the claim can be wrong, and they must
    /// all land on the same answer: everything (`*`), someone else's uid, and its
    /// own uid. The middle one is the attack and the first is the accident - a
    /// consumer written before per-user buses existed sends `*` because that was
    /// the only sensible thing to send.
    #[test]
    fn a_user_consumer_is_held_to_its_own_uid() {
        for claimed in [UidFilter::All, UidFilter::Exact(0), UidFilter::Exact(1001)] {
            assert_eq!(
                clamp_uid_filter(claimed.clone(), 1000),
                UidFilter::Exact(1000),
                "claimed {claimed:?} must not survive for a uid-1000 consumer"
            );
        }
    }

    /// Root keeps what it claims, which is what makes the forwarding possible.
    ///
    /// Clamping root too would leave nothing able to observe the machine: the
    /// system side of a per-user arrangement subscribes across uids by design, and
    /// so does the graph writer on a system deployment.
    #[test]
    fn a_root_consumer_keeps_the_filter_it_asked_for() {
        assert_eq!(clamp_uid_filter(UidFilter::All, 0), UidFilter::All);
        assert_eq!(clamp_uid_filter(UidFilter::Exact(1000), 0), UidFilter::Exact(1000));
    }

    /// The invariant the per-user design asks to be asserted rather than assumed:
    /// a consumer receives its own user's events and nobody else's.
    ///
    /// This test named uid 0 as the unclosed half yesterday. It is closed now -
    /// the two upstream causes went first (a normalizer stamping 0 on everything,
    /// and a restamp exemption covering every root-owned binary), and only once a
    /// boot measured zero uid-0 events on the bus was it safe to stop delivering
    /// them to everyone.
    #[test]
    fn a_clamped_consumer_receives_only_its_own_users_events() {
        let f = clamp_uid_filter(UidFilter::All, 1000);
        assert!(f.accepts(1000), "its own events arrive");
        assert!(!f.accepts(1001), "another user's do not");
        assert!(!f.accepts(0), "and neither does root's");
    }

    use arlen_permissions::PermissionError;

    #[test]
    fn an_absent_profile_is_a_result_and_every_other_cause_is_an_error() {
        // The rule the unit-table measurement forced out: a lookup under a wrong
        // id answers "no grants", which reads as correctly-locked-down. Absent is
        // the one cause that is genuinely a result; the rest are faults that must
        // not hide inside it.
        assert!(profile_absence_is_a_result(&PermissionError::NotFound {
            app_id: "com.example.app".into()
        }));
        for fault in [
            PermissionError::Parse("expected a table".into()),
            PermissionError::InvalidAppId { app_id: "../etc".into() },
            PermissionError::NoHomeDir,
            PermissionError::Io(std::io::Error::other("permission denied")),
        ] {
            assert!(
                !profile_absence_is_a_result(&fault),
                "{fault} must be reported, not silently read as an app with no grants"
            );
        }
    }

    use super::*;

    #[tokio::test]
    async fn test_uid_from_peercred() {
        // Create a Unix socket pair to test peer_cred extraction.
        let (sock_a, _sock_b) = tokio::net::UnixStream::pair().unwrap();
        let uid = peer_uid(&sock_a);
        // In tests, the peer UID should be our own UID.
        let expected = unsafe { libc::getuid() };
        assert_eq!(
            uid,
            Some(expected),
            "peer_uid should return the current user's UID"
        );
    }

    #[tokio::test]
    async fn test_peer_is_system_producer_false_for_non_system_binary() {
        // The peer of a socket pair is this test binary, which runs from a
        // cargo `target/` path — not `/usr/bin/arlen-*` or `/usr/lib/arlen/`.
        // It must classify as non-system, so its events get the EBK-2 uid
        // restamp. This also exercises the /proc/<pid>/exe resolution path and
        // the fail-safe (an unresolvable peer is non-system).
        //
        // Goes through `Peer::resolve` because that is what the handlers call.
        // It used to call a one-line `peer_is_system_producer` wrapper, and when
        // the handlers stopped using that wrapper the test kept passing against
        // code nothing ran - a green test for a dead path.
        let (sock_a, _sock_b) = tokio::net::UnixStream::pair().unwrap();
        assert!(
            !Peer::resolve(&sock_a).is_system(),
            "a non-system-path peer must not be treated as a system producer"
        );
    }

    fn ebus(publish: &[&str], subscribe: &[&str]) -> arlen_permissions::EventBusPermissions {
        arlen_permissions::EventBusPermissions {
            publish: publish.iter().copied().map(String::from).collect(),
            subscribe: Some(subscribe.iter().copied().map(String::from).collect()),
        }
    }

    #[test]
    fn a_would_deny_names_the_repair_not_just_the_refusal() {
        use arlen_permissions::PermissionProfile;
        // Shadow mode's whole job is to say what to write before enforce starts
        // rejecting. Measured against the live stack, the knowledge daemon's two
        // consumers logged as "<unresolved>" though their identity resolved
        // fine - what they lacked was a profile file named for that id. Pointing
        // at an identity bug instead of a missing file sends the operator to the
        // wrong place, so the three states stay distinguishable.
        let named_no_profile = PeerScope::NoProfile("dev.arlen-graph-daemon".to_string());
        assert_eq!(named_no_profile.app_id(), "dev.arlen-graph-daemon");
        assert_eq!(named_no_profile.remedy(), "no profile for this app id");

        assert_eq!(PeerScope::Unresolved.app_id(), "<unresolved>");
        assert_eq!(PeerScope::Unresolved.remedy(), "peer identity unresolved");

        let profile: PermissionProfile = toml::from_str(
            "[info]\napp_id = \"app\"\ntier = \"first-party\"\n\n[event_bus]\npublish = [\"file.opened\"]\n",
        )
        .expect("fixture profile parses");
        let profiled = PeerScope::Profiled("app".to_string(), Box::new(profile));
        assert_eq!(profiled.remedy(), "declared scope does not cover it");

        // Only a loaded profile carries a scope; both failure shapes decide the
        // same way (nothing declared) even though they read differently.
        assert!(profiled.event_bus().is_some());
        assert!(named_no_profile.event_bus().is_none());
        assert!(PeerScope::Unresolved.event_bus().is_none());
    }

    #[test]
    fn authenticated_origin_is_the_resolved_id_or_empty_never_a_sentinel() {
        use arlen_permissions::PermissionProfile;
        // The stamp carries the real kernel-attested id for an attributable peer,
        // and EMPTY (not the "<unresolved>" diagnostic sentinel) for one that is
        // not, so an unattributable producer can never be mistaken for a named one.
        //
        // This used to claim the consumers run an origin classifier that reads
        // empty as un-attested and treats it as external. They do not, and saying
        // so was describing a component that does not exist. What the two real
        // consumers do, checked:
        //
        //   clock          `sleep_origin_recognised` is allowlist membership
        //                  (`main.rs:372`). Empty is not in SLEEP_PRODUCERS, so it
        //                  is rejected - fail-closed, by not being on a list rather
        //                  than by being classified as anything.
        //   ai-engine      deliberately does not consume the stamp at all, and says
        //                  why (`orchestrator.rs:86`): its trigger flag is already
        //                  hardcoded external, so routing this in would be inert.
        //
        // The property holds either way. It is worth stating precisely, because a
        // future consumer reading the old sentence would think a shared classifier
        // was covering it.
        let profile: PermissionProfile = toml::from_str(
            "[info]\napp_id = \"app\"\ntier = \"first-party\"\n",
        )
        .expect("fixture profile parses");
        assert_eq!(
            PeerScope::Profiled("com.example.app".to_string(), Box::new(profile))
                .authenticated_origin(),
            "com.example.app"
        );
        assert_eq!(
            PeerScope::NoProfile("dev.arlen-graph-daemon".to_string()).authenticated_origin(),
            "dev.arlen-graph-daemon"
        );
        assert_eq!(PeerScope::Unresolved.authenticated_origin(), "");
    }

    #[test]
    fn the_publish_exemption_does_not_blank_a_system_producers_identity() {
        // A system producer is exempt from the declared-publish check and is still
        // fully identified. Those are separate facts, and the handler used to
        // express the first by throwing away the second: it substituted
        // `PeerScope::Unresolved` for a system peer, so `authenticated_origin`
        // went out empty - the fail-closed "treat as external" value - for the
        // compositor and the kernel layer, the two most trusted producers there
        // are.
        //
        // The substitution was already redundant: the check reads
        // `decision.hold_to_scope && !publish_allowed(...)`, so the flag alone
        // exempts. This pins the part that was NOT redundant, because a rewrite
        // that reintroduces the sentinel would otherwise pass every existing test.
        use arlen_permissions::PermissionProfile;

        let profile: PermissionProfile =
            toml::from_str("[info]\napp_id = \"app\"\ntier = \"system\"\n")
                .expect("fixture profile parses");
        // `compositor`, not `arlen-compositor`: rule (2) of the resolver maps
        // /usr/bin/arlen-compositor to `compositor`, and this list is keyed on
        // the id the bus actually resolves.
        let exempt = PeerScope::Profiled("compositor".to_string(), Box::new(profile.clone()));
        let held = PeerScope::Profiled("dev.arlen.files".to_string(), Box::new(profile));

        // An originator and an ordinary app. The stamped origin must be a
        // property of the peer in both cases and the scope check must differ -
        // that pair of assertions IS the bug: the old code produced "" for the
        // exempt case.
        let as_originator = publish_decision(&exempt);
        let as_ordinary = publish_decision(&held);

        assert_eq!(
            as_originator.origin, "compositor",
            "a resolved producer stamps its attested id, not an empty origin"
        );
        assert_eq!(
            as_ordinary.origin, "dev.arlen.files",
            "the stamped origin is a property of the peer; the exemption must not reach it"
        );
        assert!(!as_originator.hold_to_scope, "a named originator is exempt");
        assert!(
            as_ordinary.hold_to_scope,
            "and a first-party app is not, however it is installed - the whole point \
             of naming originators instead of reading the install path"
        );

        // The exemption has to come from that flag rather than from an absent
        // scope: on its own, no declared publish list is a deny.
        assert!(
            !publish_allowed(exempt.event_bus(), "window.focused"),
            "no declared publish list is a deny on its own; only the originator flag exempts"
        );
    }

    #[test]
    fn the_originator_list_is_keyed_on_resolved_ids() {
        // The trap this pins, found by writing this list and since fixed at the
        // other end: the compositor USED to ship `arlen-compositor.toml`
        // declaring `app_id = "arlen-compositor"`, while rule (2) of the
        // resolver maps /usr/bin/arlen-compositor to `compositor` and the bus
        // keys everything on what it resolved. So that profile was never loaded
        // for anything, and an entry here spelled the same way would match no
        // peer and exempt nobody - failing exactly as quietly.
        //
        // The profile is `compositor.toml` now. This stays because the pull
        // toward the binary's name is what caused it, and that pull does not go
        // away: `daemons/session` legitimately calls the PROGRAM
        // `arlen-compositor`, a keystroke away from an id that means nothing
        // here.
        assert!(originates_system_events("compositor"));
        assert!(
            !originates_system_events("arlen-compositor"),
            "the list is keyed on the id the resolver produces, not on a profile filename"
        );
        assert!(
            !originates_system_events("dev.arlen.files"),
            "a first-party app is held to its list however it is installed"
        );
    }

    #[test]
    fn publish_allowed_matches_the_declared_scope() {
        let scope = ebus(&["file.*"], &[]);
        assert!(publish_allowed(Some(&scope), "file.opened"));
        assert!(!publish_allowed(Some(&scope), "window.focused"));
        // No resolved scope is never permitted (fail-closed for enforce, the
        // shadow logger's would-deny signal).
        assert!(!publish_allowed(None, "file.opened"));
    }

    #[test]
    fn shadow_mode_keeps_every_subscription_verbatim() {
        let scope = ebus(&[], &["file.*"]);
        let requested = vec!["file.opened".to_string(), "window.focused".to_string()];
        // enforce == false: delivery is unchanged even for out-of-scope patterns.
        let kept = permitted_subscriptions(&requested, Some(&scope), false);
        assert_eq!(kept, requested);
        // And with no resolved scope at all.
        let kept = permitted_subscriptions(&requested, None, false);
        assert_eq!(kept, requested);
    }

    #[test]
    fn enforce_mode_filters_out_of_scope_subscriptions() {
        let scope = ebus(&[], &["file.*"]);
        let requested = vec!["file.opened".to_string(), "window.focused".to_string()];
        let kept = permitted_subscriptions(&requested, Some(&scope), true);
        assert_eq!(kept, vec!["file.opened".to_string()]);
        // No resolved scope under enforce keeps nothing (fail-closed).
        assert!(permitted_subscriptions(&requested, None, true).is_empty());
    }

    #[test]
    fn enforcement_defaults_to_shadow() {
        // The default (env unset) must be shadow so the reject flip is an
        // explicit opt-in and cannot silently break the live stack.
        std::env::remove_var("ARLEN_EVENT_BUS_ENFORCE");
        assert!(!enforce_pubsub(), "the bus must default to shadow (log-only)");
    }

    /// An event crosses the bus: producer socket in, consumer socket out.
    ///
    /// Nothing in this crate bound a socket before, so `bind_socket`, both
    /// accept loops, the registration handshake and the delivery write had
    /// never run in its own suite - only in the FUSE-gated integration suite
    /// that normal CI skips. This is the funnel every other component depends
    /// on, and the one bug that hurt most was exactly here: the knowledge
    /// writer sent a two-line registration to a reader expecting three, so it
    /// blocked forever and received nothing, silently, for months.
    ///
    /// So this pins the wire contract from the server's side - three
    /// newline-terminated lines, then length-prefixed protobuf out - and proves
    /// an event actually arrives.
    #[tokio::test]
    async fn the_forwarder_carries_an_upstream_event_into_this_bus() {
        // The per-user half of the design: a whole-machine observer publishes to
        // the system bus, and each user's bus pulls from it. Two registries here
        // stand in for the two buses, which is what they are - the forwarder is a
        // consumer of one and a producer into the other.
        let dir = tempfile::tempdir().unwrap();
        let upstream_consumers = dir.path().join("up-consumer.sock").to_str().unwrap().to_string();

        let upstream = ConsumerRegistry::new();
        let up = Arc::clone(&upstream);
        let path = upstream_consumers.clone();
        tokio::spawn(async move { listen_consumers(&path, up).await });

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !Path::new(&upstream_consumers).exists() && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert!(Path::new(&upstream_consumers).exists(), "the upstream must be bound");

        // The downstream bus, and a consumer on it to observe what arrives.
        let downstream = ConsumerRegistry::new();
        let mut seen = downstream
            .register("watcher".to_string(), vec!["*".to_string()], UidFilter::All)
            .await;

        let fwd_registry = Arc::clone(&downstream);
        let fwd_path = upstream_consumers.clone();
        tokio::spawn(async move { forward_from_upstream(&fwd_path, fwd_registry).await });

        // Dispatch upstream until the forwarder has registered and it lands.
        let mine = unsafe { libc::getuid() };
        let event = Event {
            id: "01890000-0000-7000-8000-0000000000ff".to_string(),
            r#type: "file.opened".to_string(),
            timestamp: 1_700_000_000_000_000,
            source: "ebpf".to_string(),
            uid: mine,
            ..Default::default()
        };

        let mut arrived = None;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            upstream.dispatch(&event).await;
            if let Ok(Ok(got)) = tokio::time::timeout(
                std::time::Duration::from_millis(100),
                seen.recv(),
            )
            .await
            .map(|o| o.ok_or("closed"))
            {
                arrived = Some(got);
                break;
            }
        }

        let got = arrived.expect("the forwarder never carried an upstream event across");
        assert_eq!(got.id, event.id);
        assert_eq!(
            got.uid, mine,
            "the observed subject's uid must survive the hop - overwriting it here \
             is the one thing that would make per-user routing meaningless"
        );
    }

    #[tokio::test]
    async fn an_event_crosses_from_a_producer_socket_to_a_consumer_socket() {
        let dir = tempfile::tempdir().unwrap();
        let producer_path = dir.path().join("producer.sock").to_str().unwrap().to_string();
        let consumer_path = dir.path().join("consumer.sock").to_str().unwrap().to_string();

        let registry = ConsumerRegistry::new();
        let (p, c) = (producer_path.clone(), consumer_path.clone());
        let reg = Arc::clone(&registry);
        let producers = tokio::spawn(async move { listen_producers(&p, reg).await });
        let consumers = tokio::spawn(async move { listen_consumers(&c, registry).await });

        // Both loops bind before accepting, so the files appearing is readiness.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while (!Path::new(&producer_path).exists() || !Path::new(&consumer_path).exists())
            && std::time::Instant::now() < deadline
        {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert!(
            Path::new(&producer_path).exists() && Path::new(&consumer_path).exists(),
            "both sockets must be bound"
        );

        // Registration: consumer id, comma-separated patterns, uid filter.
        // Three lines, because that is what the server reads.
        let mut consumer = UnixStream::connect(&consumer_path).await.unwrap();
        consumer.write_all(b"test-consumer\nfile.\n*\n").await.unwrap();

        let event = Event {
            id: "01890000-0000-7000-8000-000000000001".to_string(),
            r#type: "file.opened".to_string(),
            timestamp: 1_700_000_000_000_000,
            source: "app:test".to_string(),
            origin: "test-session".to_string(),
            ..Default::default()
        };
        let encoded = event.encode_to_vec();
        let mut producer = UnixStream::connect(&producer_path).await.unwrap();

        // Emit until it lands. Registration completes concurrently with the
        // first writes, and the bus drops an event that has no consumer at
        // dispatch time, so a single send would race.
        let mut len_buf = [0u8; 4];
        let mut delivered = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            producer
                .write_all(&u32::try_from(encoded.len()).unwrap().to_be_bytes())
                .await
                .unwrap();
            producer.write_all(&encoded).await.unwrap();

            let read = tokio::time::timeout(
                std::time::Duration::from_millis(200),
                consumer.read_exact(&mut len_buf),
            )
            .await;
            if matches!(read, Ok(Ok(_))) {
                delivered = true;
                break;
            }
        }
        assert!(delivered, "no event ever reached the consumer socket");

        let mut body = vec![0u8; u32::from_be_bytes(len_buf) as usize];
        consumer.read_exact(&mut body).await.unwrap();
        let got = Event::decode(&body[..]).expect("the consumer receives a decodable event");

        assert_eq!(got.id, event.id);
        assert_eq!(got.r#type, "file.opened");
        // The test binary is not system tier, so the bus restamps the event
        // with the producer's peercred uid rather than trusting the wire.
        assert_eq!(
            got.uid,
            unsafe { libc::getuid() },
            "a non-system producer's event must carry the peercred uid"
        );

        producers.abort();
        consumers.abort();
    }

    #[tokio::test]
    async fn a_state_topic_reaches_a_consumer_that_was_not_there_when_it_was_sent() {
        // The sibling test above has to emit in a loop until it lands, and its
        // comment says why: the bus drops an event with no consumer at dispatch
        // time. That is correct for an event and wrong for a state topic - it is
        // the whole reason net and audio were readable only by whoever happened to
        // connect first. Here the snapshot is sent with NOBODY listening, and a
        // consumer that arrives afterwards still learns the state.
        let dir = tempfile::tempdir().unwrap();
        let producer_path = dir.path().join("p.sock").to_str().unwrap().to_string();
        let consumer_path = dir.path().join("c.sock").to_str().unwrap().to_string();

        let registry = ConsumerRegistry::new();
        let (p, c) = (producer_path.clone(), consumer_path.clone());
        let reg = Arc::clone(&registry);
        let _producers = tokio::spawn(async move { listen_producers(&p, reg).await });
        let _consumers = tokio::spawn(async move { listen_consumers(&c, registry).await });

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while (!Path::new(&producer_path).exists() || !Path::new(&consumer_path).exists())
            && std::time::Instant::now() < deadline
        {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }

        let event = Event {
            id: "01890000-0000-7000-8000-00000000000a".to_string(),
            r#type: "power.state".to_string(),
            timestamp: 1_700_000_000_000_000,
            source: "app:test".to_string(),
            origin: "test-session".to_string(),
            ..Default::default()
        };
        let encoded = event.encode_to_vec();
        let mut producer = UnixStream::connect(&producer_path).await.unwrap();
        producer
            .write_all(&u32::try_from(encoded.len()).unwrap().to_be_bytes())
            .await
            .unwrap();
        producer.write_all(&encoded).await.unwrap();

        // Each attempt is a whole fresh late subscriber, because the retained
        // copy is taken when the server processes the write and connecting
        // instantly could beat it. Retrying the connect - rather than sleeping a
        // guessed interval - keeps the assertion on the behaviour.
        let mut len_buf = [0u8; 4];
        let mut delivered = None;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            let mut consumer = UnixStream::connect(&consumer_path).await.unwrap();
            consumer.write_all(b"late\npower.\n*\n").await.unwrap();
            if tokio::time::timeout(
                std::time::Duration::from_millis(200),
                consumer.read_exact(&mut len_buf),
            )
            .await
            .is_ok()
            {
                let len = u32::from_be_bytes(len_buf) as usize;
                let mut body = vec![0u8; len];
                consumer.read_exact(&mut body).await.unwrap();
                delivered = Some(Event::decode(&body[..]).unwrap());
                break;
            }
        }

        let got = delivered.expect("a late subscriber is handed the retained snapshot");
        assert_eq!(got.r#type, "power.state");
        // Its own timestamp, not the moment it was handed over. This is what lets
        // a consumer show it as last-known instead of painting it as live.
        assert_eq!(got.timestamp, 1_700_000_000_000_000);
    }
}

/// Subscribe to an upstream bus and republish what it sends into this one.
///
/// THE ONE THING A PER-USER BUS CANNOT DO FOR ITSELF. A whole-machine observer -
/// the eBPF kernel layer - watches every process on the box and has no per-user
/// form, so it keeps publishing to the system bus. Each user's bus then pulls
/// what belongs to it. The coupling points that way on purpose: the system side
/// holds no connections into anyone's runtime directory and does not need to know
/// which users exist.
///
/// It asks for `*` and does not get it. The upstream bus clamps a consumer's
/// filter to the uid it attested through `SO_PEERCRED` (see `clamp_uid_filter`),
/// so this registration comes back narrowed to this bus's own uid whatever it
/// requested - which is why the design can say a per-user bus cannot reach
/// another user's events "even by asking wrongly". Asking for everything is the
/// honest request to make; being held to your own is the upstream's job.
///
/// Reconnects with a fixed delay rather than giving up: the upstream is a
/// separate unit with its own restarts, and a forwarder that exits on the first
/// hiccup turns a momentary gap into a permanently empty graph.
pub async fn forward_from_upstream(
    upstream_consumer_path: &str,
    registry: Arc<ConsumerRegistry>,
) -> Result<()> {
    const RECONNECT_DELAY: std::time::Duration = std::time::Duration::from_secs(2);
    loop {
        match forward_once(upstream_consumer_path, &registry).await {
            Ok(()) => debug!(upstream = upstream_consumer_path, "upstream closed the stream"),
            Err(e) => debug!(upstream = upstream_consumer_path, "upstream forward ended: {e}"),
        }
        tokio::time::sleep(RECONNECT_DELAY).await;
    }
}

/// One connection's worth of forwarding, so the retry loop above stays readable.
async fn forward_once(upstream_consumer_path: &str, registry: &Arc<ConsumerRegistry>) -> Result<()> {
    let mut stream = UnixStream::connect(upstream_consumer_path).await?;

    // The consumer registration is three newline-terminated lines: id, patterns,
    // uid filter. `*` for both because this bus forwards whatever its own
    // consumers may later want, and the upstream decides what that may include.
    let uid = unsafe { libc::getuid() };
    let registration = format!("event-bus-forwarder-{uid}\n*\n*\n");
    stream.write_all(registration.as_bytes()).await?;
    stream.flush().await?;
    info!(upstream = upstream_consumer_path, uid, "forwarding from the upstream bus");

    loop {
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;
        if len > MAX_EVENT_SIZE {
            anyhow::bail!("upstream event of {len} bytes exceeds the {MAX_EVENT_SIZE} cap");
        }
        let mut body = vec![0u8; len];
        stream.read_exact(&mut body).await?;

        // Forwarded verbatim. The upstream already restamped the producer's uid
        // and origin, and re-stamping here would overwrite the observed subject
        // with this bus's own identity - which is the single thing the whole
        // arrangement exists to preserve.
        let event = Event::decode(&body[..])?;
        registry.dispatch(&event).await;
    }
}
