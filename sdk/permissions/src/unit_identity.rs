//! A daemon's identity is the systemd unit that started it, read from its cgroup.
//!
//! The other resolvers answer "what binary is this" ([`crate::identity`]) or "what
//! did the launcher stamp" ([`crate::stamped_identity`]). Neither reaches a
//! systemd-started daemon: nothing stamps a unit, because nothing but systemd
//! starts one, and `/proc/{pid}/exe` is refused to a reader hardened with
//! `ProtectSystem=strict`. Measured on this tree, three of the five shipped system
//! units do not resolve by binary AT ALL - `/usr/bin/event-bus` and
//! `/usr/bin/llama-server` both come back `UnknownBinary`, so the core event funnel
//! has no identity today by any route.
//!
//! The cgroup does name it, and the kernel maintains that name: a process cannot
//! move itself into `/system.slice`, because entering the system tree needs root,
//! which is the privilege the threat model assumes the attacker lacks.
//!
//! # Why this refuses the user slice
//!
//! A unit name is only attested where the user cannot choose it, and in the user
//! session they can. Measured 13 Aug, three ways:
//!
//!   - `systemd-run --user --unit=arlen-knowledge-spoof sleep 30` lands at
//!     `.../user@1000.service/app.slice/arlen-knowledge-spoof.service`, so an
//!     arbitrary name is one command away;
//!   - that route REFUSES a name that already has a fragment file ("Unit ... was
//!     already loaded or has a fragment file"), even for an inactive unit, which
//!     makes the transient path look safe - and it is not, because
//!   - a hand-written `~/.config/systemd/user/<anything>.service` runs any binary
//!     under any unit name, and `systemd-analyze --user unit-paths` puts that
//!     directory ABOVE `/usr/lib/systemd/user`. The user owns the directory that
//!     wins.
//!
//! So a user-slice peer is REFUSED here rather than resolved, and falls through to
//! the resolvers that do attest it (the launcher's registration). Resolving a
//! self-chosen name would be worse than the `/proc/{pid}/exe` read this replaces:
//! it would hand an attacker a way to pick their identity rather than merely
//! inherit one.

/// The cgroup path prefix that only root can put a process under.
const SYSTEM_SLICE: &str = "/system.slice/";

/// The user session's cgroup prefix, named so the refusal can say which case it
/// hit rather than only that it refused.
const USER_SLICE: &str = "/user.slice/";

/// Why a cgroup line yielded no unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitError {
    /// The peer is in the user session, where it chooses its own unit name. Not a
    /// failure to read: a deliberate refusal, and the caller should fall through to
    /// a resolver that attests the user session.
    UserSlice(String),
    /// Neither slice: a bare `/` (pid 1), a container's own root, a v1 hierarchy,
    /// or a machine slice. Nothing to attest.
    NotAUnit(String),
    /// The cgroup file had no unified (`0::`) line at all.
    Unreadable,
}

impl std::fmt::Display for UnitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UserSlice(p) => write!(
                f,
                "the peer is in the user session ({p}), where a unit names itself; \
                 identity there comes from the launcher that asked for it"
            ),
            Self::NotAUnit(p) => write!(f, "the peer's cgroup names no service unit ({p})"),
            Self::Unreadable => write!(f, "the peer's cgroup has no unified hierarchy line"),
        }
    }
}

/// The unified (`0::`) path out of a `/proc/{pid}/cgroup` file.
///
/// cgroup v2 puts everything on one line whose prefix is exactly `0::`. A v1
/// hierarchy has numbered controller lines instead and no unified one, so it is
/// [`UnitError::Unreadable`] rather than something to guess at.
fn unified_path(cgroup_text: &str) -> Option<&str> {
    cgroup_text
        .lines()
        .find_map(|l| l.strip_prefix("0::"))
        .map(str::trim_end)
}

/// The systemd unit that started the process this cgroup file describes.
///
/// Only a `/system.slice/...` path yields a unit; see the module doc for why the
/// user slice is refused rather than resolved. Nested slices are followed
/// (`/system.slice/a.slice/b.service`), because systemd nests freely and the root
/// is what carries the attestation.
pub fn unit_from_cgroup(cgroup_text: &str) -> Result<String, UnitError> {
    let path = unified_path(cgroup_text).ok_or(UnitError::Unreadable)?;
    if path.starts_with(USER_SLICE) || path.contains("/user@") {
        return Err(UnitError::UserSlice(path.to_string()));
    }
    let Some(rest) = path.strip_prefix(SYSTEM_SLICE) else {
        return Err(UnitError::NotAUnit(path.to_string()));
    };
    // The LAST component, so a nested slice resolves to the service and not to the
    // slice it sits in.
    let last = rest.rsplit('/').next().unwrap_or("");
    if !last.ends_with(".service") || last.len() <= ".service".len() {
        return Err(UnitError::NotAUnit(path.to_string()));
    }
    Ok(last.to_string())
}

/// The shipped system units, each with the app_id its peers authenticate as.
///
/// Explicit rather than derived from the unit name, for the same reason the name
/// is trusted at all: what makes this sound is that WE choose the mapping and the
/// kernel guarantees the key. Deriving `arlen-graph.service` -> `arlen-graph`
/// would also quietly disagree with the binary route, which resolves that daemon
/// as `knowledge` - and two resolvers naming one daemon differently is how a
/// profile lookup silently misses.
///
/// Kept in step with the shipped units by `dev/scripts/check-unit-identity.py`.
const UNIT_APP_IDS: &[(&str, &str)] = &[
    ("arlen-config-broker.service", "config-broker"),
];

/// The per-user units and the app_id each one's peers authenticate as.
///
/// Separate from [`UNIT_APP_IDS`] because the ATTESTATION is separate: a system
/// unit is named by the kernel, a user unit by whoever asked systemd to start it
/// (see the module doc). This table says what the launcher registers, and is not
/// on its own evidence of anything - a user unit nobody asked for is absent from
/// the registry and resolves to nothing regardless of what stands here.
///
/// Every id is the one the BINARY route already produces for that unit's
/// `ExecStart`, so the two resolvers cannot disagree about one daemon. Checked by
/// `dev/scripts/check-unit-identity.py`, which is also where the one deviation is
/// forced to be visible: `arlen-ai-engine-daemon.service` is `ai-agent`, not
/// `ai-engine-daemon`, and a name-derived guess would have produced an id no
/// profile is filed under - a lookup that answers "no grants" and reads as
/// correctly-locked-down.
const USER_UNIT_APP_IDS: &[(&str, &str)] = &[
    ("arlen-ai-engine-daemon.service", "ai-agent"),
    ("arlen-ai-proxy.service", "ai-proxy"),
    ("arlen-ai-undo-signer.service", "ai-undo-signer"),
    ("arlen-anomalyd.service", "anomalyd"),
    ("arlen-auditd.service", "auditd"),
    ("arlen-capsuled.service", "capsuled"),
    // Moved from the SYSTEM table on 15 Aug with the units themselves: the graph
    // daemon and its timeline view now run under the user manager, so the
    // supervisor is what stamps them and a lookup has to find them here. Leaving
    // them in the system table would not have failed loudly - `app_id_for_unit`
    // would keep answering for a unit no system manager runs, while
    // `app_id_for_user_unit` returned None for the one that does.
    ("arlen-clockd.service", "clockd"),
    ("arlen-code-indexer.service", "code-indexer"),
    ("arlen-consent-broker.service", "consent-broker"),
    ("arlen-dogfood.service", "dogfood"),
    ("arlen-event-bus.service", "event-bus"),
    ("arlen-graph.service", "knowledge"),
    ("arlen-journald-parser.service", "journald-parser"),
    ("arlen-modulesd.service", "modulesd"),
    ("arlen-notifyd.service", "notifyd"),
    ("arlen-powerd.service", "powerd"),
    ("arlen-terminal-run-mcp.service", "terminal-run-mcp"),
    ("arlen-timeline.service", "timeline"),
    ("arlen-undod.service", "undod"),
    ("arlen-wallpaperd.service", "wallpaperd"),
];

/// The app_id a per-user unit's peers authenticate as, or `None` for a unit that
/// has no name yet. See [`USER_UNIT_APP_IDS`]; `None` is a refusal, not a guess.
pub fn app_id_for_user_unit(unit: &str) -> Option<&'static str> {
    USER_UNIT_APP_IDS.iter().find(|(u, _)| *u == unit).map(|(_, a)| *a)
}

/// Whether `app_id` is one this resolver's own tables name for a shipped daemon.
///
/// The set a supervisor may legitimately stamp, and therefore the set a broker may
/// legitimately return for one - which is what lets a reader accept a RESERVED id
/// (`ai-agent`, `settings`) from the broker without accepting every reserved id.
/// A compromised broker is still capped: it can name one of these, never `system`
/// or an `org.arlen.*` principal that appears in no table.
pub fn is_enrolled_daemon_id(app_id: &str) -> bool {
    USER_UNIT_APP_IDS.iter().any(|(_, a)| *a == app_id)
        || UNIT_APP_IDS.iter().any(|(_, a)| *a == app_id)
}

/// Every per-user unit this resolver knows.
pub fn enrolled_user_units() -> impl Iterator<Item = &'static str> {
    USER_UNIT_APP_IDS.iter().map(|(u, _)| *u)
}

/// The app_id for an attested system unit, or `None` for a unit we do not ship.
///
/// `None` is a refusal, not a fallback: a system unit nobody enrolled has no
/// profile to load, and inventing an app_id from its name would let anything root
/// installs pick up an identity the catalogue never granted.
pub fn app_id_for_unit(unit: &str) -> Option<&'static str> {
    UNIT_APP_IDS.iter().find(|(u, _)| *u == unit).map(|(_, a)| *a)
}

/// Every unit this resolver knows, for the drift check and for callers that want
/// to report what is enrollable.
pub fn enrolled_units() -> impl Iterator<Item = &'static str> {
    UNIT_APP_IDS.iter().map(|(u, _)| *u)
}

/// Resolve a pinned peer's app_id from its cgroup.
///
/// The pid must already be PINNED by the caller (a `SO_PEERPIDFD` pidfd held
/// across this call, see [`crate::peer_pidfd::PeerPidfd`]) - otherwise the pid can
/// be recycled between the peer-credential read and this one, and the answer would
/// name a different process. This reads by pid number under that pin, the same way
/// the other `/proc` resolvers do.
pub fn app_id_from_pinned_pid(pid: u32) -> Result<&'static str, UnitError> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/cgroup"))
        .map_err(|_| UnitError::Unreadable)?;
    let unit = unit_from_cgroup(&text)?;
    app_id_for_unit(&unit).ok_or(UnitError::NotAUnit(unit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_system_unit_is_named_by_its_cgroup() {
        // `arlen-event-bus` rather than `arlen-graph`: the graph daemon moved to
        // the user manager on 15 Aug, and a system-table assertion about it kept
        // passing for a while afterwards because the entry was still there. The
        // unit named here has to be one that is actually a system unit, or the
        // test outlives the fact it asserts.
        let line = "0::/system.slice/arlen-event-bus.service\n";
        assert_eq!(unit_from_cgroup(line).unwrap(), "arlen-event-bus.service");
        assert_eq!(app_id_for_unit("arlen-event-bus.service"), Some("event-bus"));
    }

    #[test]
    fn the_graph_daemon_is_a_user_unit_now() {
        // The move is only half done if the tables disagree with the unit files:
        // the supervisor stamps by the user table, so an entry left in the system
        // one would answer for a manager that never runs it.
        assert_eq!(app_id_for_user_unit("arlen-graph.service"), Some("knowledge"));
        assert_eq!(app_id_for_user_unit("arlen-timeline.service"), Some("timeline"));
        assert_eq!(app_id_for_unit("arlen-graph.service"), None);
        assert_eq!(app_id_for_unit("arlen-timeline.service"), None);
    }

    #[test]
    fn a_nested_system_slice_resolves_to_the_service() {
        let line = "0::/system.slice/system-arlen.slice/arlen-event-bus.service\n";
        assert_eq!(unit_from_cgroup(line).unwrap(), "arlen-event-bus.service");
    }

    #[test]
    fn a_user_slice_peer_is_refused_not_resolved() {
        // The measured spoof: any name, one command, no privilege. Resolving it
        // would let a peer pick its identity instead of inherit one.
        let line = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/arlen-graph.service\n";
        let err = unit_from_cgroup(line).unwrap_err();
        assert!(matches!(err, UnitError::UserSlice(_)), "{err:?}");
        // And it says which case it hit, so a caller can fall through knowingly.
        assert!(err.to_string().contains("launcher"), "{err}");
    }

    #[test]
    fn a_user_manager_outside_user_slice_is_still_refused() {
        // Belt for a layout where the session manager is not under /user.slice:
        // the `user@` marker refuses it on its own.
        let line = "0::/some.slice/user@1000.service/app.slice/x.service\n";
        assert!(matches!(unit_from_cgroup(line).unwrap_err(), UnitError::UserSlice(_)));
    }

    #[test]
    fn a_scope_or_a_bare_root_names_no_unit() {
        for path in [
            "0::/\n",
            "0::/system.slice/arlen-graph.scope\n",
            "0::/machine.slice/machine-x.scope\n",
            "0::/system.slice/.service\n",
        ] {
            assert!(
                matches!(unit_from_cgroup(path), Err(UnitError::NotAUnit(_))),
                "{path:?} should name no unit"
            );
        }
    }

    #[test]
    fn a_v1_hierarchy_has_no_unified_line() {
        let v1 = "12:pids:/system.slice/arlen-graph.service\n1:name=systemd:/system.slice/x.service\n";
        assert_eq!(unit_from_cgroup(v1), Err(UnitError::Unreadable));
    }

    #[test]
    fn a_unit_we_do_not_ship_resolves_to_nothing() {
        assert_eq!(app_id_for_unit("sshd.service"), None);
        assert_eq!(app_id_for_unit("arlen-graph.service.evil"), None);
    }

    #[test]
    fn the_event_bus_gets_an_identity_it_has_no_other_way_to_get() {
        // `/usr/bin/event-bus` resolves to UnknownBinary by path (it carries no
        // `arlen-` prefix and lives in no app directory), so before this resolver
        // the core event funnel had no attested identity by any route.
        assert!(crate::identity::path_to_app_id(std::path::Path::new("/usr/bin/event-bus")).is_err());
        assert_eq!(app_id_for_unit("arlen-event-bus.service"), Some("event-bus"));
    }

    #[test]
    fn every_enrolled_unit_maps_to_a_valid_app_id() {
        for unit in enrolled_units().chain(enrolled_user_units()) {
            let id = app_id_for_unit(unit)
                .or_else(|| app_id_for_user_unit(unit))
                .expect("enrolled unit maps");
            assert!(crate::is_valid_app_id(id), "{unit} -> {id}");
        }
    }

    #[test]
    fn the_two_tables_never_name_the_same_unit() {
        // One unit is attested one way or the other, never both: a unit in the
        // system slice is named by the kernel, one in the user session by the
        // party that started it. A name in both tables would mean the answer
        // depends on which resolver ran, which is the thing neither may allow.
        for unit in enrolled_user_units() {
            assert!(
                app_id_for_unit(unit).is_none(),
                "{unit} is in both the system and the user table"
            );
        }
    }

    #[test]
    fn the_engine_daemon_is_ai_agent_and_not_its_unit_name() {
        // The measurement that killed the derive-from-the-name shortcut. Keep it
        // as a test so the next person who has that idea is stopped by a red
        // suite rather than by a boot where a daemon quietly has no grants.
        assert_eq!(app_id_for_user_unit("arlen-ai-engine-daemon.service"), Some("ai-agent"));
    }

    #[test]
    fn the_enrolled_set_names_shipped_daemons_and_nothing_else() {
        // What a reader may accept as a RESERVED stamp. `ai-agent` is the case that
        // matters: it is reserved, it is a real shipped daemon, and refusing it made
        // the supervised path inert on 13 Aug.
        assert!(is_enrolled_daemon_id("ai-agent"));
        assert!(is_enrolled_daemon_id("knowledge"));
        // And the principals nobody ships a unit for stay out, which is the cap: a
        // compromised broker can name a daemon, never one of these.
        assert!(!is_enrolled_daemon_id("system"));
        assert!(!is_enrolled_daemon_id("org.arlen.anything"));
        assert!(!is_enrolled_daemon_id("com.example.app"));
    }
}
