//! Which sockets a confined app may dial, decided by what its profile grants.
//!
//! bwrap binds the whole Arlen runtime directory into the sandbox, so every
//! socket in it is visible to every app, and each daemon's own peer check is
//! the only thing between an app and a service it holds no grant for. Landlock's
//! `RESOLVE_UNIX` right (ABI 9) puts a second line in front of that: connecting
//! to a path socket needs a rule on it, and the launcher writes a rule only for
//! the sockets the profile's sections call for.
//!
//! [`SOCKETS`] names every socket in the tree's inventory
//! (`dev/scripts/check-socket-servers.py`), each with the grant that opens it.
//! `dev/scripts/check-socket-grants.py` fails when the two lists differ, so a
//! new socket cannot arrive without somebody deciding who may dial it.

use std::path::{Path, PathBuf};

use arlen_permissions::PermissionProfile;

/// What a profile must hold for its app to dial a socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grant {
    /// A `[graph]` section that grants something.
    Graph,
    /// An `[event_bus]` section that grants something.
    EventBus,
    /// `[notifications]` switched on.
    Notifications,
    /// A `[clipboard]` section that grants something.
    Clipboard,
    /// A `[search]` section that grants something.
    Search,
    /// An `[intents]` section that grants something.
    Intents,
    /// A `[system.power]` grant.
    Power,
    /// Every confined app, whatever its profile says.
    Everyone,
    /// No app. The socket is between daemons, or between the shell and itself.
    Nobody,
}

/// Every socket in the inventory and who may dial it.
///
/// `Everyone` is the short list: `launch.sock` is how an app asks the shell to
/// open a file or a link when no portal frontend answers, and the shell decides.
/// `Nobody` is everything an app has no section for. That includes the sockets
/// the system monitor probes for its health page (`audit-ingest.sock`,
/// `modulesd.sock`): the monitor is not launched confined, and the day it is,
/// it needs a grant of its own here.
pub const SOCKETS: &[(&str, Grant)] = &[
    ("ai-engine.sock", Grant::Nobody),
    ("ai-engine-drive.sock", Grant::Nobody),
    ("ai-proxy.sock", Grant::Nobody),
    ("audit-ingest.sock", Grant::Nobody),
    ("audit-read.sock", Grant::Nobody),
    ("bottled.sock", Grant::Nobody),
    ("capsule.sock", Grant::Nobody),
    ("capsule-control.sock", Grant::Nobody),
    ("clipboard.sock", Grant::Clipboard),
    ("config-broker.sock", Grant::Nobody),
    ("config-broker-identity.sock", Grant::Nobody),
    ("consent-intake.sock", Grant::Nobody),
    ("consent-control.sock", Grant::Nobody),
    ("event-bus-producer.sock", Grant::EventBus),
    ("event-bus-consumer.sock", Grant::EventBus),
    ("intents.sock", Grant::Intents),
    ("knowledge.sock", Grant::Graph),
    ("launch.sock", Grant::Everyone),
    ("notification.sock", Grant::Notifications),
    ("power.sock", Grant::Power),
    ("search.sock", Grant::Search),
    ("sentinel.sock", Grant::Nobody),
    ("store.sock", Grant::Nobody),
    ("topbar.sock", Grant::Nobody),
    ("undo-signer.sock", Grant::Nobody),
    ("terminal-read.sock", Grant::Nobody),
    ("rcd.sock", Grant::Nobody),
    ("transfer.sock", Grant::Nobody),
    ("portal-picker.sock", Grant::Nobody),
    ("portal-print.sock", Grant::Nobody),
    ("modulesd.sock", Grant::Nobody),
    ("settings-broker.sock", Grant::Nobody),
];

/// Whether `profile` holds `grant`.
fn holds(profile: &PermissionProfile, grant: Grant) -> bool {
    match grant {
        Grant::Graph => profile.graph.reach_summary().is_some(),
        Grant::EventBus => profile.event_bus.reach_summary().is_some(),
        Grant::Notifications => profile.notifications.enabled,
        Grant::Clipboard => profile.clipboard.reach_summary().is_some(),
        Grant::Search => profile.search.reach_summary().is_some(),
        Grant::Intents => profile.intents.reach_summary().is_some(),
        Grant::Power => profile.system.power.suspend || profile.system.power.set_profile,
        Grant::Everyone => true,
        Grant::Nobody => false,
    }
}

/// The names of the Arlen sockets opened by the grants `held` answers yes to.
fn opened_by(held: impl Fn(Grant) -> bool) -> Vec<&'static str> {
    SOCKETS.iter().filter(|(_, grant)| held(*grant)).map(|(name, _)| *name).collect()
}

/// The names of the Arlen sockets `profile` opens.
pub fn granted(profile: &PermissionProfile) -> Vec<&'static str> {
    opened_by(|grant| holds(profile, grant))
}

/// Every socket path the app may connect to: the session plumbing any app needs
/// (the compositor, PipeWire, the session bus) and the Arlen sockets its profile
/// opens, each at both places a daemon may have bound it.
///
/// A path is listed whether or not it exists. The fence skips one it cannot
/// open, which leaves the app unable to dial a daemon that starts after it:
/// less access, never more.
pub fn dialable(
    profile: &PermissionProfile,
    runtime_dir: Option<&Path>,
    wayland_display: Option<&str>,
) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(rt) = runtime_dir {
        if let Some(wl) = wayland_display {
            paths.push(if Path::new(wl).is_absolute() { PathBuf::from(wl) } else { rt.join(wl) });
        }
        paths.push(rt.join("pipewire-0"));
        paths.push(rt.join("bus"));
    }
    for name in granted(profile) {
        if let Some(rt) = runtime_dir {
            paths.push(rt.join("arlen").join(name));
        }
        paths.push(Path::new("/run/arlen").join(name));
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A profile as the launcher loads it, from a file.
    fn profile(body: &str) -> PermissionProfile {
        let dir = tempfile::tempdir().expect("a temp dir");
        let id = "dev.example.app";
        std::fs::write(
            dir.path().join(format!("{id}.toml")),
            format!("[info]\napp_id = \"{id}\"\ntier = \"third-party\"\n{body}"),
        )
        .expect("the profile is written");
        arlen_permissions::load_profile_from(&dir.path().join(format!("{id}.toml")), id)
            .expect("the profile loads")
    }

    #[test]
    fn a_profile_with_nothing_opens_only_what_everyone_gets() {
        assert_eq!(granted(&profile("")), vec!["launch.sock"]);
    }

    #[test]
    fn a_graph_grant_opens_the_knowledge_socket_and_nothing_else() {
        let got = granted(&profile("[graph]\nread = [\"system.File\"]\n"));
        assert_eq!(got, vec!["knowledge.sock", "launch.sock"]);
    }

    #[test]
    fn an_event_bus_grant_opens_both_bus_sockets() {
        let got = granted(&profile("[event_bus]\npublish = [\"app.thing\"]\n"));
        assert_eq!(got, vec!["event-bus-producer.sock", "event-bus-consumer.sock", "launch.sock"]);
    }

    #[test]
    fn every_grant_opens_something_and_nobody_opens_nothing() {
        for grant in [
            Grant::Graph,
            Grant::EventBus,
            Grant::Notifications,
            Grant::Clipboard,
            Grant::Search,
            Grant::Intents,
            Grant::Power,
            Grant::Everyone,
        ] {
            assert!(!opened_by(|g| g == grant).is_empty(), "{grant:?} opens a socket");
        }
        let everything = profile(
            "[graph]\nread = [\"system.File\"]\n[event_bus]\npublish = [\"app.thing\"]\n\
             [notifications]\nenabled = true\n[clipboard]\nread = true\n\
             [search]\nopen = true\n[intents]\ndispatch = true\n\
             [system.power]\nsuspend = true\n",
        );
        let got = granted(&everything);
        for (name, grant) in SOCKETS {
            assert_eq!(got.contains(name), *grant != Grant::Nobody, "{name}");
        }
    }

    #[test]
    fn a_granted_socket_is_listed_at_both_runtime_directories() {
        let p = profile("[graph]\nread = [\"system.File\"]\n");
        let paths = dialable(&p, Some(Path::new("/run/user/1000")), Some("wayland-1"));
        for want in [
            "/run/user/1000/wayland-1",
            "/run/user/1000/pipewire-0",
            "/run/user/1000/bus",
            "/run/user/1000/arlen/knowledge.sock",
            "/run/arlen/knowledge.sock",
            "/run/arlen/launch.sock",
        ] {
            assert!(paths.contains(&PathBuf::from(want)), "{want} is dialable");
        }
        assert!(!paths.iter().any(|p| p.ends_with("clipboard.sock")));
    }

    #[test]
    fn no_socket_is_named_twice() {
        let mut names: Vec<&str> = SOCKETS.iter().map(|(n, _)| *n).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before);
    }
}
