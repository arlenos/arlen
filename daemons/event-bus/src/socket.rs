use crate::proto::Event;
use crate::registry::{ConsumerRegistry, UidFilter};
use crate::validation;
use anyhow::Result;
use prost::Message;
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tracing::{debug, error, warn};

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

fn peer_is_system_producer(stream: &UnixStream) -> bool {
    peer_tier(stream) == Some(arlen_permissions::AppTier::System)
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

    /// What an operator has to do about a would-deny from this peer.
    fn remedy(&self) -> &'static str {
        match self {
            Self::Profiled(..) => "declared scope does not cover it",
            Self::NoProfile(_) => "no profile for this app id",
            Self::Unresolved => "peer identity unresolved",
        }
    }
}

/// Resolve the connected peer from its kernel-attested pid.
fn peer_app_profile(stream: &UnixStream) -> PeerScope {
    let Some(app_id) = stream
        .peer_cred()
        .ok()
        .and_then(|c| c.pid())
        .and_then(|pid| u32::try_from(pid).ok())
        .and_then(|pid| arlen_permissions::identity::app_id_from_pid(pid).ok())
    else {
        return PeerScope::Unresolved;
    };
    match arlen_permissions::load_profile(&app_id) {
        Ok(profile) => PeerScope::Profiled(app_id, Box::new(profile)),
        Err(_) => PeerScope::NoProfile(app_id),
    }
}

/// Whether the bus REJECTS an unauthorised publish/subscribe (enforce) or only
/// LOGS it (shadow). Defaults to shadow so the first-party `[event_bus]` scopes
/// can be verified against real traffic before the reject flip - the same
/// shadow/enforce cutover the stamped-identity strand uses. Set
/// `ARLEN_EVENT_BUS_ENFORCE=1` (or `true`) to reject.
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
    let is_system_producer = peer_is_system_producer(&stream);
    // Non-system producers are held to their declared `[event_bus].publish`
    // scope. Resolved once (peercred is fixed for the connection). System-tier
    // producers - the eBPF kernel-layer, the compositor, first-party daemons -
    // are exempt, mirroring the uid-restamp exemption: they emit machine-wide
    // events by design.
    let publish_scope = if is_system_producer {
        PeerScope::Unresolved
    } else {
        peer_app_profile(&stream)
    };
    let enforce = enforce_pubsub();
    debug!(
        uid = producer_uid,
        system = is_system_producer,
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
        if len == 0 || len > 1024 * 1024 {
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
                if !is_system_producer {
                    event.uid = producer_uid;
                }

                // Hold a non-system producer to its declared publish scope.
                // Shadow mode logs a would-deny and still dispatches; enforce
                // mode drops the event. System producers skipped (exempt above).
                if !is_system_producer
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
                    Err(e) => warn!(error = %e, "dropping invalid event"),
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

    let uid_filter = UidFilter::parse(&uid_line).map_err(|e| anyhow::anyhow!(e))?;

    // Hold a non-system consumer to its declared `[event_bus].subscribe` scope.
    // System-tier consumers (the knowledge daemon, the shell) observe the machine
    // by design and are exempt, mirroring the producer exemption. Shadow mode
    // logs each would-deny and keeps the pattern; enforce mode filters it out.
    let subscribed_types = if peer_tier(&stream) == Some(arlen_permissions::AppTier::System) {
        subscribed_types
    } else {
        let scope = peer_app_profile(&stream);
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

    debug!(
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
        let (sock_a, _sock_b) = tokio::net::UnixStream::pair().unwrap();
        assert!(
            !peer_is_system_producer(&sock_a),
            "a non-system-path peer must not be treated as a system producer"
        );
    }

    fn ebus(publish: &[&str], subscribe: &[&str]) -> arlen_permissions::EventBusPermissions {
        arlen_permissions::EventBusPermissions {
            publish: publish.iter().copied().map(String::from).collect(),
            subscribe: subscribe.iter().copied().map(String::from).collect(),
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
}
