//! Caller identity resolution (FA7 / X-id), peer-authenticated.
//!
//! Portal callers pass `app_id` in their method arguments, but that
//! value is caller-controlled and therefore untrusted: an app that
//! blanks it would otherwise fall open to `Unconfined` and wake the
//! fail-closed guards in the FileChooser and OpenURI handlers. We
//! never read it for identity. Instead we resolve the real caller
//! from the D-Bus message sender, fail-closed:
//!
//! 1. Take the message **sender** (set by the bus daemon, not a
//!    method argument) and resolve its PID via
//!    `GetConnectionUnixProcessID`. No sender or no PID -> `Unknown`.
//! 2. **Flatpak:** if `/proc/<pid>/root/.flatpak-info` exists the
//!    caller is a Flatpak app; the app id is read from that file's
//!    `[Application] name=` (never from the caller argument). This is
//!    the standard xdg-desktop-portal handshake and is robust to the
//!    `xdg-dbus-proxy` topology that makes the proxy's PID, not the
//!    app's, visible. A present-but-unreadable `.flatpak-info` is a
//!    sandbox we cannot name -> `Unknown` -> deny.
//! 3. **Arlen-native:** else resolve the PID through Arlen's own
//!    identity infrastructure (`arlen_permissions::identity`,
//!    openat-hardened `/proc/<pid>/exe` -> anchored install path).
//!    A resolved app is `ArlenNative`.
//! 4. **Else -> `Unconfined`.** A plain host process under the user's
//!    own uid reaches the filesystem directly; serving it raw
//!    `file://` grants nothing it did not already have.
//!
//! Empty or unverifiable callers are `Unknown`, never `Unconfined`.

use arlen_permissions::identity::app_id_from_pid;

/// Outcome of identity resolution for a portal caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallerIdentity {
    /// Flatpak-confined caller. `app_id` comes from `.flatpak-info`
    /// (`org.gnome.Calculator`, ...). Mount-namespaced, so picked
    /// paths must be re-exported through the Document Portal.
    Flatpak {
        /// Flatpak application id from `[Application] name=`.
        app_id: String,
    },
    /// Arlen-native caller resolved to a known install path. Runs on
    /// the host under the user's uid (no separate mount namespace
    /// today), so it reaches picked files directly.
    ArlenNative {
        /// Resolved Arlen app id.
        app_id: String,
    },
    /// Plain host process: a binary we could not anchor to a known
    /// install path, running under the invoking user. It can do
    /// whatever that user can do regardless of any app_id it passes.
    Unconfined,
    /// Identity could not be determined: the message had no sender,
    /// `org.freedesktop.DBus` was unreachable, or the PID lookup
    /// failed. Authorization decisions that touch a security boundary
    /// must fail-closed for this state; silently coalescing it into
    /// `Unconfined` would let a transient D-Bus glitch waive the
    /// sandbox check.
    Unknown,
}

impl CallerIdentity {
    /// App id for Document Portal routing: `Some` only for callers
    /// that live in a separate mount namespace (Flatpak) and so need
    /// their picked paths re-exported. `None` for host-reachable
    /// callers (`ArlenNative`, `Unconfined`) and for `Unknown`.
    pub fn app_id(&self) -> Option<&str> {
        match self {
            CallerIdentity::Flatpak { app_id } => Some(app_id),
            CallerIdentity::ArlenNative { .. }
            | CallerIdentity::Unconfined
            | CallerIdentity::Unknown => None,
        }
    }

    /// True when the caller reaches the host filesystem directly and
    /// so may receive raw `file://` URIs without a Document Portal
    /// export: `ArlenNative` and `Unconfined`. False for the
    /// mount-namespaced `Flatpak` and for `Unknown` (which must be
    /// denied outright before this is consulted).
    pub fn reaches_host_fs(&self) -> bool {
        matches!(
            self,
            CallerIdentity::ArlenNative { .. } | CallerIdentity::Unconfined
        )
    }

    /// True when identity resolution produced a definite answer.
    /// False only for `Unknown`. Callers that must fail-closed on
    /// resolution failure gate on this.
    pub fn is_known(&self) -> bool {
        !matches!(self, CallerIdentity::Unknown)
    }
}

/// Resolve the caller identity from a D-Bus message header.
///
/// Fails closed to [`CallerIdentity::Unknown`] if the message carries
/// no sender or the PID cannot be resolved, so the security guards in
/// the FileChooser and OpenURI handlers refuse the request rather than
/// treating an unverifiable caller as unconfined.
pub async fn resolve_identity(
    connection: &zbus::Connection,
    header: &zbus::message::Header<'_>,
) -> CallerIdentity {
    let Some(sender) = header.sender() else {
        tracing::warn!("portal request carried no D-Bus sender; identity unknown");
        return CallerIdentity::Unknown;
    };
    match connection_pid(connection, sender).await {
        Ok(pid) => identity_for_pid(pid),
        Err(e) => {
            tracing::warn!(%sender, error = %e, "could not resolve caller PID; identity unknown");
            CallerIdentity::Unknown
        }
    }
}

/// Resolve a sender bus name to its connection PID via
/// `org.freedesktop.DBus.GetConnectionUnixProcessID`.
async fn connection_pid(
    connection: &zbus::Connection,
    sender: &zbus::names::UniqueName<'_>,
) -> Result<u32, zbus::Error> {
    let dbus = zbus::fdo::DBusProxy::new(connection).await?;
    let bus_name = zbus::names::BusName::try_from(sender.as_str())?;
    let pid = dbus.get_connection_unix_process_id(bus_name).await?;
    Ok(pid)
}

/// The resolution chain for a known caller PID: Flatpak via
/// `.flatpak-info`, then Arlen-native via the install-path resolver,
/// else `Unconfined`. The PID-reading is split from the decision so
/// [`classify_identity`] is unit-testable.
fn identity_for_pid(pid: u32) -> CallerIdentity {
    let flatpak_info =
        std::fs::read_to_string(format!("/proc/{pid}/root/.flatpak-info")).ok();
    // Only consult the native resolver when this is not a Flatpak
    // caller: a Flatpak's `/proc/<pid>/exe` points inside the runtime,
    // not at an Arlen install path, so the native lookup is moot.
    let native = if flatpak_info.is_none() {
        app_id_from_pid(pid).ok()
    } else {
        None
    };
    classify_identity(flatpak_info.as_deref(), native)
}

/// Pure classification core. `flatpak_info` is `Some(contents)` when
/// `/proc/<pid>/root/.flatpak-info` exists (file present means the
/// caller is Flatpak-confined), `native` is the Arlen-native app id
/// when the install-path resolver succeeded.
///
/// A present `.flatpak-info` whose `[Application] name=` is missing or
/// unsafe yields `Unknown` (a sandbox we cannot name, denied) rather
/// than falling through to `Unconfined`.
fn classify_identity(flatpak_info: Option<&str>, native: Option<String>) -> CallerIdentity {
    if let Some(contents) = flatpak_info {
        return match parse_flatpak_info(contents) {
            Some(app_id) => CallerIdentity::Flatpak { app_id },
            None => CallerIdentity::Unknown,
        };
    }
    match native {
        Some(app_id) => CallerIdentity::ArlenNative { app_id },
        None => CallerIdentity::Unconfined,
    }
}

/// Pull the Flatpak app id out of a `.flatpak-info` payload: the
/// `name=` key in the `[Application]` section. Returns `None` (caller
/// treats as deny) when the section or key is absent or the value is
/// not a safe app id.
fn parse_flatpak_info(content: &str) -> Option<String> {
    let mut in_application = false;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_application = line == "[Application]";
            continue;
        }
        if in_application {
            if let Some(value) = line.strip_prefix("name=") {
                let id = value.trim();
                return is_safe_flatpak_id(id).then(|| id.to_string());
            }
        }
    }
    None
}

/// Validate that a `.flatpak-info` app id is a safe reverse-DNS
/// identifier before it is used as a Document Portal app id or
/// interpolated into a path. Rejects empty, leading/trailing dot,
/// `..`, and anything outside `[A-Za-z0-9._-]` (Flatpak ids are
/// case-sensitive reverse-DNS, so uppercase is allowed unlike the
/// lowercase-only Arlen app-id rule).
fn is_safe_flatpak_id(id: &str) -> bool {
    if id.is_empty() || id.starts_with('.') || id.ends_with('.') || id.contains("..") {
        return false;
    }
    id.bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real `.flatpak-info` payload: the app id comes from
    /// `[Application] name=`, not from any caller argument.
    #[test]
    fn parses_application_name() {
        let info = "[Application]\nname=org.gnome.Calculator\nruntime=runtime/org.gnome.Platform/x86_64/45\n\n[Instance]\ninstance-id=12345\n";
        assert_eq!(
            parse_flatpak_info(info).as_deref(),
            Some("org.gnome.Calculator")
        );
    }

    /// `name=` only counts inside `[Application]`. A `name=` in a
    /// later section must not be picked up.
    #[test]
    fn ignores_name_outside_application_section() {
        let info = "[Instance]\nname=evil\n[Context]\nname=also-evil\n";
        assert_eq!(parse_flatpak_info(info), None);
    }

    /// Missing `[Application]` section -> None -> denied.
    #[test]
    fn missing_application_section_is_none() {
        assert_eq!(parse_flatpak_info("[Instance]\ninstance-id=1\n"), None);
        assert_eq!(parse_flatpak_info(""), None);
    }

    /// An unsafe app id (path traversal) is rejected at parse time so
    /// it can never reach a Document Portal call or a path join.
    #[test]
    fn unsafe_app_id_rejected() {
        assert_eq!(parse_flatpak_info("[Application]\nname=../etc\n"), None);
        assert_eq!(parse_flatpak_info("[Application]\nname=\n"), None);
        assert_eq!(parse_flatpak_info("[Application]\nname=a/b\n"), None);
        assert_eq!(parse_flatpak_info("[Application]\nname=.hidden\n"), None);
    }

    #[test]
    fn safe_id_charset() {
        assert!(is_safe_flatpak_id("org.gnome.Calculator"));
        assert!(is_safe_flatpak_id("com.valve.Steam"));
        assert!(is_safe_flatpak_id("io.github.app-name_2"));
        assert!(!is_safe_flatpak_id(""));
        assert!(!is_safe_flatpak_id(".x"));
        assert!(!is_safe_flatpak_id("x."));
        assert!(!is_safe_flatpak_id("a..b"));
        assert!(!is_safe_flatpak_id("a/b"));
        assert!(!is_safe_flatpak_id("a b"));
    }

    /// Flatpak present + parseable -> Flatpak (mount-namespaced,
    /// routes through Document Portal).
    #[test]
    fn classify_flatpak() {
        let id = classify_identity(Some("[Application]\nname=org.x.Y\n"), None);
        assert_eq!(
            id,
            CallerIdentity::Flatpak {
                app_id: "org.x.Y".into()
            }
        );
        assert_eq!(id.app_id(), Some("org.x.Y"));
        assert!(!id.reaches_host_fs());
        assert!(id.is_known());
    }

    /// Flatpak present but unparseable -> Unknown (deny), never
    /// Unconfined. Even if a native id was somehow resolved, the
    /// sandbox signal wins.
    #[test]
    fn classify_unparseable_flatpak_is_unknown() {
        let id = classify_identity(Some("[Instance]\ninstance-id=1\n"), Some("settings".into()));
        assert_eq!(id, CallerIdentity::Unknown);
        assert!(!id.is_known());
    }

    /// No Flatpak, install-path resolver succeeded -> ArlenNative.
    /// It reaches the host fs directly but is positively identified.
    #[test]
    fn classify_arlen_native() {
        let id = classify_identity(None, Some("settings".into()));
        assert_eq!(
            id,
            CallerIdentity::ArlenNative {
                app_id: "settings".into()
            }
        );
        assert_eq!(id.app_id(), None);
        assert!(id.reaches_host_fs());
        assert!(id.is_known());
    }

    /// No Flatpak, no native resolution -> Unconfined (a plain host
    /// binary under the user's uid).
    #[test]
    fn classify_unconfined() {
        let id = classify_identity(None, None);
        assert_eq!(id, CallerIdentity::Unconfined);
        assert_eq!(id.app_id(), None);
        assert!(id.reaches_host_fs());
        assert!(id.is_known());
    }

    /// Accessors for the failure state: no app id, not known, does
    /// not reach host fs (must be denied before that is consulted
    /// anyway).
    #[test]
    fn unknown_accessors() {
        let id = CallerIdentity::Unknown;
        assert_eq!(id.app_id(), None);
        assert!(!id.is_known());
        assert!(!id.reaches_host_fs());
    }
}
