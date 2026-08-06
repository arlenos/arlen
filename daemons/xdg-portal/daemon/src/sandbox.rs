//! What a portal caller is, for the handlers that have to decide what a
//! request may reach.
//!
//! Identity is NOT derived here. This backend registers
//! `org.freedesktop.impl.portal.desktop.arlen` and sits behind the standard
//! `xdg-desktop-portal` frontend, which authenticates the real app (pidfd +
//! `.flatpak-info`) and passes the result as the `app_id` METHOD ARGUMENT. The
//! handlers verify the D-Bus sender owns the frontend's well-known name and
//! then trust that argument; this module only names the outcome.
//!
//! It used to derive identity from `/proc/<pid>/cgroup`, and that was the bug
//! behind two reverted attempts: a Flatpak caller reaches the bus through
//! `xdg-dbus-proxy`, so the sender pid is the PROXY, whose cgroup is the
//! user-session scope. Every confined caller read as `Unconfined` - strictly
//! worse than trusting the frontend. The helpers are gone rather than kept
//! dormant: a resolver that misclassifies every real Flatpak caller is not a
//! fallback, and leaving it next to a comment saying not to use it is how the
//! next person reaches for it again.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallerIdentity {
    /// Flatpak-confined caller. `app_id` is the Flatpak application
    /// id (`org.gnome.Calculator`, `com.spotify.Client`, ...).
    Flatpak { app_id: String },
    /// Anything else: native binary, systemd service, container we
    /// have not explicitly detected. The caller can do whatever the
    /// invoking user can do regardless of what app_id they pass.
    Unconfined,
    /// Not constructed today, and kept deliberately rather than deleted with
    /// `Snap`: two live guards read it - `open_uri`'s `file_uri_authorized`
    /// refuses on it and `file_chooser` gates on `is_known()`. Removing the
    /// variant means removing those, and they are the fail-closed branches on an
    /// authorisation path. They do not fire because a caller we cannot attest is
    /// refused earlier by the sender-is-the-frontend check, so this is a second
    /// line rather than dead weight - and a second line whose cost is one
    /// `#[allow]`.
    #[allow(dead_code)]
    /// Identity could not be determined: D-Bus message had no
    /// sender header, `org.freedesktop.DBus` was unreachable, or
    /// PID-to-cgroup lookup failed. Authorization decisions that
    /// touch a security boundary (file:// access through the host)
    /// must fail-closed for this state — Codex review found that
    /// silently coalescing this into `Unconfined` would let a
    /// transient D-Bus glitch waive the sandbox check.
    ///
    /// Also not constructed today, and that is worth knowing rather than
    /// assuming: the fail-closed guards written against it never fire, because
    /// a caller we cannot attest is refused at the door by the
    /// sender-is-the-frontend check before identity is consulted at all.
    /// Refusing earlier is the better place; this variant is the older,
    /// now-unreachable second line.
    Unknown,
}

impl CallerIdentity {
    /// Best-effort app-id string suitable for logs and Document
    /// Portal calls. `None` for unconfined callers and for the
    /// Unknown failure state.
    pub fn app_id(&self) -> Option<&str> {
        match self {
            CallerIdentity::Flatpak { app_id } => Some(app_id),
            CallerIdentity::Unconfined | CallerIdentity::Unknown => None,
        }
    }

    /// True when sandbox detection produced a definite answer
    /// (Flatpak / Unconfined). False only for Unknown.
    /// Callers that need to fail-closed on identity-resolution
    /// failures gate on this.
    pub fn is_known(&self) -> bool {
        !matches!(self, CallerIdentity::Unknown)
    }
}
