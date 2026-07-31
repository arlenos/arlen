//! Permission profile types for Arlen OS.
//!
//! Each app has a TOML profile at `~/.config/permissions/{app_id}.toml`
//! defining what it can access: Knowledge Graph, Event Bus, filesystem,
//! network, clipboard, notifications, etc. The user owns this file
//! (foundation §7.3 — sole source of truth).
//!
//! See `docs/architecture/AUTH-CANONICAL.md`.

pub mod connection_auth;
pub mod fd_passing;
pub mod identity;
pub mod learning;
pub mod lint;
pub mod identity_registry;
pub mod identity_store;
pub mod identity_wire;
pub mod peer_pidfd;
pub mod stamped_identity;
pub mod profile_watcher;
pub mod revoke;

pub use connection_auth::{AuthError, ConnectionAuth};
pub use peer_pidfd::{PeerPidfd, PidfdError};
pub use stamped_identity::{app_id_from_connection, IdentitySource, StampedIdentity};
pub use profile_watcher::{ProfileChange, ProfileWatcher};
pub use revoke::{RevokeInitiator, RevokeOutcome, RevokeReach, RevokedReach};

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum PermissionError {
    #[error("profile not found for {app_id}")]
    NotFound { app_id: String },
    #[error("home directory not found")]
    NoHomeDir,
    #[error("invalid app id: {app_id}")]
    InvalidAppId { app_id: String },
    #[error("IO: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse: {0}")]
    Parse(String),
}

// ---------------------------------------------------------------------------
// App tier
// ---------------------------------------------------------------------------

/// Trust tier based on install location and signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppTier {
    System,
    #[serde(alias = "first-party")]
    FirstParty,
    #[serde(alias = "third-party")]
    ThirdParty,
}

/// Detect tier from the executable path.
pub fn detect_tier(exe_path: &Path) -> AppTier {
    let s = exe_path.to_string_lossy();
    if s.starts_with("/usr/lib/arlen/") || s.starts_with("/usr/bin/arlen-") {
        AppTier::System
    } else if s.contains("/arlen/first-party/") || s.starts_with("/usr/lib/arlen-first-party/") {
        AppTier::FirstParty
    } else {
        AppTier::ThirdParty
    }
}

// ---------------------------------------------------------------------------
// Permission profile
// ---------------------------------------------------------------------------

/// Complete permission profile for one app.
///
/// Every dimension here is projected into the Living Capability Graph as a
/// `declared` Grant, which is what makes an app's reach visible on the App-access
/// page and revocable there. A grant that exists in this struct but not in the
/// graph is a second, invisible permission store, and that is the failure mode
/// behind essentially every real-world per-app-permission drift bug: Android's
/// special-access screens, Flatpak's static manifest versus its portal grants,
/// macOS `locationd`. So the projection is enforced structurally rather than by
/// discipline. `emit_all_declared_grants` destructures this struct with no `..`
/// rest pattern, and each dimension's `reach_summary` destructures its own fields
/// the same way, so adding a permission anywhere in this file fails to compile
/// until someone has said whether it is reach. A field that deliberately is not
/// reach is bound to `_` with the reason, which is a decision on the record
/// instead of a silence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionProfile {
    pub info: ProfileInfo,
    #[serde(default)]
    pub graph: GraphPermissions,
    #[serde(default)]
    pub event_bus: EventBusPermissions,
    #[serde(default)]
    pub filesystem: FilesystemPermissions,
    #[serde(default)]
    pub network: NetworkPermissions,
    #[serde(default)]
    pub notifications: NotificationPermissions,
    #[serde(default)]
    pub clipboard: ClipboardPermissions,
    #[serde(default)]
    pub system: SystemPermissions,
    #[serde(default)]
    pub input: InputPermissions,
    #[serde(default)]
    pub search: SearchPermissions,
    #[serde(default)]
    pub intents: IntentsPermissions,
    #[serde(default)]
    pub mcp: McpPermissions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileInfo {
    pub app_id: String,
    #[serde(default = "default_tier")]
    pub tier: AppTier,
}

fn default_tier() -> AppTier {
    AppTier::ThirdParty
}

// ── Graph ──

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GraphPermissions {
    #[serde(default)]
    pub read: Vec<String>,
    #[serde(default)]
    pub write: Vec<String>,
    #[serde(default)]
    pub app_isolated: bool,
    /// Reverse-domain namespaces this app may read annotations FROM
    /// in addition to its own. Foundation §395: "reading another
    /// application's annotations requires an explicit permission
    /// declaration." Wildcards follow the same `pattern_matches`
    /// semantics as `read`/`write`: `"com.example.*"` permits all
    /// namespaces under that prefix; `"*"` would permit reading every
    /// app's annotations and is intentionally not a special-case.
    /// Daemon-side enforcement of this is part of the Phase 3.2-full
    /// token-authenticated write path; for now, the SDK honours the
    /// declaration on the client side.
    #[serde(default)]
    pub annotations_read_cross_namespace: Vec<String>,
    /// Relation-write grants: which `(from, to, type)` edges this app may create
    /// in the Knowledge Graph. Empty for an app that writes no relations.
    #[serde(default)]
    pub relations: Vec<RelationPermission>,
    /// Sensitive entity-type reads the app is granted - read paths that expose
    /// sensitive fields, gated separately from the ordinary `read` set.
    #[serde(default)]
    pub read_sensitive: Vec<String>,
    /// Whether the app reads/writes only its OWN instances (`own`) or ALL
    /// instances of a granted type (`all`). Defaults to `own` (least privilege).
    #[serde(default)]
    pub instance_scope: InstanceScopeConfig,
    /// Namespaces this app may write entity types under besides its own (the
    /// foreign-app-bridge delegation, foreign-app-bridges.md §2). Raw prefix
    /// strings (e.g. `["md.obsidian"]`); the write path validates each through
    /// `NamespaceGrant::new` (reserved `system.*`/`shared.*` ungrantable, fail-
    /// closed). Empty for an ordinary app, which writes only its own namespace.
    #[serde(default)]
    pub delegated_namespaces: Vec<String>,
    /// Entity-type read/write patterns the app declares ESSENTIAL: revoking one
    /// would break the app, so an install-time or App-access tighten refuses to
    /// strip it (app-enrollment §E2, the anti-brick marker). Each entry matches a
    /// `read`/`write` pattern verbatim (e.g. `"system.File.path"`). Empty for the
    /// conservative default profile, where every reach is freely revocable.
    #[serde(default)]
    pub required: Vec<String>,
}

/// A relation-write grant entry from a profile's `[graph]` section: which
/// `(from, to, type)` edge the app may create.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationPermission {
    /// The from-node entity type.
    pub from: String,
    /// The to-node entity type.
    pub to: String,
    /// The relation type (the TOML key is `type`).
    #[serde(rename = "type")]
    pub relation_type: String,
}

/// The declared instance scope in a profile's `[graph]` section: `own` (only the
/// app's own instances) or `all` (all instances of a granted type). The runtime
/// projection to the graph layer's own scope enum is a knowledge-daemon concern.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstanceScopeConfig {
    /// Only the app's own instances (the default, least privilege).
    #[default]
    Own,
    /// All instances of a granted type.
    All,
}

impl GraphPermissions {
    /// Check if a pattern list matches an entity type.
    /// Patterns: `"com.app.Note"` (exact), `"com.app.*"` (namespace wildcard).
    pub fn can_read(&self, entity_type: &str) -> bool {
        pattern_matches(&self.read, entity_type)
    }

    pub fn can_write(&self, entity_type: &str) -> bool {
        pattern_matches(&self.write, entity_type)
    }

    /// Whether the app may read annotations from a foreign namespace.
    /// `own_namespace == requested` is always allowed; reading
    /// another app's namespace requires a matching pattern in
    /// `annotations_read_cross_namespace`.
    pub fn can_read_annotations_from(&self, own_namespace: &str, requested: &str) -> bool {
        if own_namespace == requested {
            return true;
        }
        pattern_matches(&self.annotations_read_cross_namespace, requested)
    }

    /// The declared graph reach as an LCG grant `consent_scope`: the read + write
    /// entity-type patterns. `None` when the app declares neither.
    ///
    /// Unlike its sibling dimensions this one has no caller: the graph grant is
    /// projected from the minted capability TOKEN (`emit_grant_node`) rather than
    /// from the profile, so the reach shown on the App-access page comes from what
    /// the app was actually granted at connect. This method is kept because the
    /// token path covers only `read`/`write`; three of the fields below are reach
    /// the token does not carry, so if graph ever projects from the profile they
    /// need classifying first rather than defaulting to invisible.
    pub fn reach_summary(&self) -> Option<String> {
        // Exhaustive on purpose; see [`PermissionProfile`].
        let Self {
            read,
            write,
            app_isolated: _, // a narrowing, not reach
            // Reach, not yet summarised: reading another app's annotations is an
            // explicit grant (foundation §395), relations are edge-write grants,
            // and a delegated namespace is authority handed to another principal.
            annotations_read_cross_namespace: _,
            relations: _,
            delegated_namespaces: _,
            read_sensitive: _,  // gated separately at the daemon
            instance_scope: _,  // which instances, not which types
            required: _,        // an install-time assertion, not reach
        } = self;
        let mut parts = Vec::new();
        if !read.is_empty() {
            parts.push(format!("read {}", read.join(", ")));
        }
        if !write.is_empty() {
            parts.push(format!("write {}", write.join(", ")));
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join("; "))
        }
    }
}

// ── Event Bus ──

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EventBusPermissions {
    #[serde(default)]
    pub publish: Vec<String>,
    #[serde(default)]
    pub subscribe: Vec<String>,
}

impl EventBusPermissions {
    /// Check if the app can publish to a given event type.
    pub fn can_publish(&self, event_type: &str) -> bool {
        pattern_matches(&self.publish, event_type)
    }

    /// Check if the app can subscribe to a given event type.
    pub fn can_subscribe(&self, event_type: &str) -> bool {
        pattern_matches(&self.subscribe, event_type)
    }

    /// The declared event-bus reach: the subscribed (heard) and published (emitted)
    /// event kinds. An app that hears the bus sees activity, so this is real reach.
    /// `None` when the app declares neither.
    pub fn reach_summary(&self) -> Option<String> {
        // Exhaustive on purpose; see [`PermissionProfile`].
        let Self { publish, subscribe } = self;
        let mut parts = Vec::new();
        if !subscribe.is_empty() {
            parts.push(format!("hears {}", subscribe.join(", ")));
        }
        if !publish.is_empty() {
            parts.push(format!("emits {}", publish.join(", ")));
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join("; "))
        }
    }
}

// ── Filesystem ──

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FilesystemPermissions {
    #[serde(default)]
    pub home: bool,
    #[serde(default)]
    pub documents: bool,
    #[serde(default)]
    pub downloads: bool,
    #[serde(default)]
    pub pictures: bool,
    #[serde(default)]
    pub music: bool,
    #[serde(default)]
    pub videos: bool,
    #[serde(default)]
    pub custom: Vec<PathBuf>,
}

impl FilesystemPermissions {
    /// The declared filesystem reach: the standard directories the app may access
    /// plus any custom paths. `None` when nothing is declared.
    pub fn reach_summary(&self) -> Option<String> {
        // Exhaustive on purpose; see [`PermissionProfile`].
        let Self {
            home,
            documents,
            downloads,
            pictures,
            music,
            videos,
            custom,
        } = self;
        let mut parts: Vec<String> = Vec::new();
        for (on, label) in [
            (home, "home"),
            (documents, "documents"),
            (downloads, "downloads"),
            (pictures, "pictures"),
            (music, "music"),
            (videos, "videos"),
        ] {
            if *on {
                parts.push(label.to_string());
            }
        }
        for p in custom {
            parts.push(p.display().to_string());
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(", "))
        }
    }
}

// ── Network ──

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkPermissions {
    #[serde(default)]
    pub allow_all: bool,
    #[serde(default)]
    pub allowed_domains: Vec<String>,
}

impl NetworkPermissions {
    /// Check if a domain is allowed.
    /// `api.example.com` matches `allowed_domains: ["example.com"]`.
    pub fn is_domain_allowed(&self, domain: &str) -> bool {
        if self.allow_all {
            return true;
        }
        let domain_lower = domain.to_lowercase();
        self.allowed_domains.iter().any(|allowed| {
            let allowed_lower = allowed.to_lowercase();
            domain_lower == allowed_lower
                || domain_lower.ends_with(&format!(".{allowed_lower}"))
        })
    }

    /// The app's declared network reach as the `consent_scope` string an LCG
    /// `NetworkAccess` grant carries, so the App-access page can render "Internet:
    /// all" or "Internet: reaches api.openai.com, github.com" and revoke it
    /// (living-capability-graph.md §11b). `None` means no declared network reach:
    /// no grant to project, the app shows no internet access.
    ///
    /// Projects what the profile declares TODAY: `allow_all` maps to `"all"`,
    /// otherwise the sorted, deduped, lowercased, comma-joined domain allowlist.
    /// Port and direction (LAN vs WAN) are not in the schema yet, so they are not
    /// projected. HONESTY CAVEAT (living-capability-graph.md §12): the allowlist is
    /// domain-DECLARED but IP-ENFORCED at net-guard (hostname to IP at rule-apply
    /// time), so a "reaches example.com" label is only fully truthful once the
    /// net-guard proxy does SNI/Host matching per connection. This is the declared
    /// reach (visible plus revocable is the win), not an enforcement guarantee.
    pub fn reach_summary(&self) -> Option<String> {
        // Exhaustive on purpose; see [`PermissionProfile`].
        let Self { allow_all, allowed_domains } = self;
        if *allow_all {
            return Some("all".to_string());
        }
        let mut domains: Vec<String> = allowed_domains
            .iter()
            .map(|d| d.trim().to_lowercase())
            .filter(|d| !d.is_empty())
            .collect();
        domains.sort();
        domains.dedup();
        if domains.is_empty() {
            None
        } else {
            Some(domains.join(","))
        }
    }
}

// ── Notifications ──

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotificationPermissions {
    #[serde(default)]
    pub enabled: bool,
}

impl NotificationPermissions {
    /// Declared notification reach: `Some("on")` when the app may post
    /// notifications, else `None`.
    pub fn reach_summary(&self) -> Option<String> {
        // Exhaustive on purpose; see [`PermissionProfile`].
        let Self {
            enabled,
        } = self;
        if *enabled {
            Some("on".to_string())
        } else {
            None
        }
    }
}

// ── Clipboard ──

/// Clipboard subsystem permissions. Apps request these in their
/// permission profile under `[permissions.clipboard]`.
///
/// `read`/`write` cover the basic shell.clipboard API surface.
/// `read_sensitive` lets the app see clipboard content that the
/// writer marked `label = "sensitive"`; without it, `read()` and
/// `onChanged()` return `null`-content for sensitive entries.
/// `history` gates `getHistory()` — sensitive entries are filtered
/// out at write time and never appear in history regardless of
/// this permission.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClipboardPermissions {
    #[serde(default)]
    pub read: bool,
    /// Receive content of sensitive-labelled clipboard entries.
    /// Without this, `read()` and `onChanged()` deliver
    /// metadata-only for sensitive content. Defaults to false so
    /// existing permission profiles automatically drop into the
    /// safe state on upgrade.
    #[serde(default)]
    pub read_sensitive: bool,
    #[serde(default)]
    pub write: bool,
    /// Query clipboard history via `getHistory()`. Sensitive
    /// entries are excluded from history at write time, so this
    /// permission is strictly about "may I see the historical
    /// list at all" — not a fine-grained sensitivity gate.
    #[serde(default)]
    pub history: bool,
}

impl ClipboardPermissions {
    /// Declared clipboard reach: the enabled capabilities (read, write, read
    /// sensitive, history). `None` when the app declares no clipboard access.
    pub fn reach_summary(&self) -> Option<String> {
        // Exhaustive on purpose; see [`PermissionProfile`].
        let Self {
            read,
            read_sensitive,
            write,
            history,
        } = self;
        let mut parts: Vec<&str> = Vec::new();
        if *read {
            parts.push("read");
        }
        if *write {
            parts.push("write");
        }
        if *read_sensitive {
            parts.push("read sensitive");
        }
        if *history {
            parts.push("history");
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(", "))
        }
    }
}

// ── System ──

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SystemPermissions {
    #[serde(default)]
    pub autostart: bool,
    #[serde(default)]
    pub background: bool,
    /// Power-action grants (suspend/power-off, profile changes) the power
    /// daemon (`org.arlen.Power1`) mediates. Default-empty: no app may suspend
    /// or change the power profile without an explicit grant (PWR-R7).
    #[serde(default)]
    pub power: PowerPermissions,
}

impl SystemPermissions {
    /// Declared system reach: autostart, background running, and power actions.
    /// `None` when none is declared. Power actions are `org.arlen.Power1`-mediated.
    pub fn reach_summary(&self) -> Option<String> {
        // Exhaustive on purpose; see [`PermissionProfile`].
        let Self {
            autostart,
            background,
            power,
        } = self;
        let mut parts: Vec<&str> = Vec::new();
        if *autostart {
            parts.push("autostart");
        }
        if *background {
            parts.push("background");
        }
        if power.suspend {
            parts.push("suspend/power-off");
        }
        if power.set_profile {
            parts.push("set power profile");
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(", "))
        }
    }
}

/// The power-action capability scope (system-services-plan.md PWR-R7).
///
/// The power daemon holds the logind / power-profiles-daemon trust; a caller
/// reaches a sleep/power-off or profile change only with the matching grant
/// here, resolved from the caller's profile by its attested app id. Nothing is
/// granted by default, so an unprofiled or unprivileged app cannot suspend the
/// machine or change its profile.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PowerPermissions {
    /// May request a sleep/power-off action (suspend, hibernate, power-off,
    /// reboot, and the sleep variants) via `org.arlen.Power1`.
    #[serde(default)]
    pub suspend: bool,
    /// May change the active power profile (performance/balanced/power-saver).
    #[serde(default)]
    pub set_profile: bool,
}

// ── Search ──

/// Waypointer search subsystem permissions.
///
/// `open` lets an app programmatically open the Waypointer launcher
/// with a prefilled query via `os-sdk::search::UnixSearchClient::open`.
/// Low-blast-radius scope; spoof per F3 lets an attacker pop the
/// user's launcher with a chosen query, no further reach.
///
/// `register_handler` and `intercept_all` are reserved for the Phase-7
/// modulesd-based handler-registration pipeline and are NOT honored
/// by any current broker. They parse cleanly so profiles authored
/// today survive the Phase-7 schema rollout without rewrite.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchPermissions {
    /// Programmatic Waypointer-open with prefilled query.
    /// `os-sdk::search::open(query, mode)` requires this scope.
    #[serde(default)]
    pub open: bool,
    /// Phase-7 modulesd-hosted search-result-provider registration.
    /// Reserved; no broker honors this today. Document-only.
    #[serde(default)]
    pub register_handler: bool,
    /// Phase-7 gate for `.*`-style universal-match patterns in
    /// register_handler. Reserved; no broker honors this today.
    #[serde(default)]
    pub intercept_all: bool,
}

impl SearchPermissions {
    /// Declared search reach: whether the app may open the Waypointer (the only
    /// live capability today; the reserved handler flags are surfaced too).
    /// `None` when the app declares no search access.
    pub fn reach_summary(&self) -> Option<String> {
        // Exhaustive on purpose; see [`PermissionProfile`].
        let Self {
            open,
            register_handler,
            intercept_all,
        } = self;
        let mut parts: Vec<&str> = Vec::new();
        if *open {
            parts.push("open launcher");
        }
        if *register_handler {
            parts.push("register handler");
        }
        if *intercept_all {
            parts.push("intercept all");
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(", "))
        }
    }
}

// ── Intents ──

/// `shell.intents` cross-process action dispatch.
///
/// `dispatch` lets an app fire typed intents (`url`, `file`,
/// `text`, `email`, `project`) via the `os-sdk::intents` SDK.
/// Phase-6-live, broker single-shot, Foundation §6.4 / Listing 11.
///
/// `register` and `preferences` are reserved for the Phase-7
/// modulesd `intent.handler` extension point and the multi-
/// handler-resolution preference cache (foundation §6.4 "user
/// chooses once and the preference is remembered"). They parse
/// cleanly today so profiles authored against Phase-7 schema
/// survive without rewrite.
///
/// See `docs/architecture/intent-system.md` for the broker
/// contract and `identity-spoof-mitigation.md` for the F3 same-
/// uid spoof acceptance for `dispatch` (blast ≤ xdg-open).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IntentsPermissions {
    /// Programmatic intent dispatch via `shell.intents.dispatch`.
    /// Built-in Phase-6 handlers cover url/file/text/email/project.
    #[serde(default)]
    pub dispatch: bool,
    /// Phase-7 modulesd-hosted handler-registration. Reserved;
    /// no broker honors this today.
    #[serde(default)]
    pub register: bool,
    /// Phase-7 multi-handler-resolution preference cache write
    /// permission. Reserved; "user chooses once and is remembered"
    /// requires consent-prompt + AppArmor (F3 bundle).
    #[serde(default)]
    pub preferences: bool,
}

impl IntentsPermissions {
    /// Declared intents reach: `dispatch` (live) plus the reserved register /
    /// preferences flags. `None` when the app declares no intents access.
    pub fn reach_summary(&self) -> Option<String> {
        // Exhaustive on purpose; see [`PermissionProfile`].
        let Self {
            dispatch,
            register,
            preferences,
        } = self;
        let mut parts: Vec<&str> = Vec::new();
        if *dispatch {
            parts.push("dispatch");
        }
        if *register {
            parts.push("register");
        }
        if *preferences {
            parts.push("preferences");
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(", "))
        }
    }
}

// ── Input ──

/// Input subsystem permissions. Module manifests request these via
/// `[permissions].input = [...]`; the install daemon copies the
/// matching flags into the runtime profile at
/// `~/.config/permissions/{module_id}.toml` (the path the runtime reads).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct InputPermissions {
    /// Register keybindings that fire only while the module's own
    /// window has keyboard focus.
    #[serde(default)]
    pub register_focused_bindings: bool,
    /// Register keybindings that fire regardless of focus. Reserved
    /// for system and first-party modules; third-party modules must
    /// be granted this explicitly.
    #[serde(default)]
    pub register_global_bindings: bool,
}

impl InputPermissions {
    /// Default input permissions for a given trust tier. Third-party
    /// modules get only focused bindings; global bindings need an
    /// explicit grant.
    pub fn defaults_for_tier(tier: AppTier) -> Self {
        match tier {
            AppTier::System | AppTier::FirstParty => Self {
                register_focused_bindings: true,
                register_global_bindings: true,
            },
            AppTier::ThirdParty => Self {
                register_focused_bindings: true,
                register_global_bindings: false,
            },
        }
    }

    /// Apply a manifest-declared list of input permission strings on
    /// top of `self`. Unknown strings are ignored (forward-compat).
    pub fn apply_manifest_requests(&mut self, requests: &[String]) {
        for r in requests {
            match r.as_str() {
                "register_focused_bindings" => self.register_focused_bindings = true,
                "register_global_bindings" => self.register_global_bindings = true,
                _ => {}
            }
        }
    }

    /// Declared input reach: focused / global keybinding registration. `None` when
    /// the module registers no bindings.
    pub fn reach_summary(&self) -> Option<String> {
        // Exhaustive on purpose; see [`PermissionProfile`].
        let Self { register_focused_bindings, register_global_bindings } = self;
        let mut parts: Vec<&str> = Vec::new();
        if *register_focused_bindings {
            parts.push("focused bindings");
        }
        if *register_global_bindings {
            parts.push("global bindings");
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(", "))
        }
    }
}

// ── MCP ──

/// MCP-related permissions on a [`PermissionProfile`].
///
/// An app that ships an MCP server declares which of its tools are
/// read-only (default-permitted within the user's AI permission
/// level) and which require per-session authorization before the AI
/// may call them. `always_confirm_overrides` lets an app mark extra
/// tools as always-confirm; it can only *add* to the hardcoded
/// always-confirm set, never remove from it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpPermissions {
    /// Tool names exposed as read-only. Calling these does not
    /// change state, so no per-session authorization is required.
    #[serde(default)]
    pub tools_default_permit: Vec<String>,
    /// Tool names that mutate state. The AI must hold a live
    /// per-session authorization grant before calling these.
    #[serde(default)]
    pub tools_action_authorize: Vec<String>,
    /// Tool names this app additionally wants always-confirmed, on
    /// top of the hardcoded always-confirm set. Additive only.
    #[serde(default)]
    pub always_confirm_overrides: Vec<String>,
}

impl McpPermissions {
    /// Whether a tool is declared as read-only by this app.
    pub fn is_read_only(&self, tool: &str) -> bool {
        self.tools_default_permit.iter().any(|t| t == tool)
    }

    /// Whether a tool is declared as requiring per-session
    /// authorization by this app.
    pub fn requires_authorization(&self, tool: &str) -> bool {
        self.tools_action_authorize.iter().any(|t| t == tool)
    }

    /// Declared MCP reach: the exposed tool set (read-only tools plus the
    /// action tools the AI can call after authorization). An app exposing tools
    /// the AI then uses is real reach. `None` when the app exposes no MCP tools.
    pub fn reach_summary(&self) -> Option<String> {
        // Exhaustive on purpose; see [`PermissionProfile`].
        let Self {
            tools_default_permit,
            tools_action_authorize,
            always_confirm_overrides: _, // friction the app adds, not reach it gains
        } = self;
        let mut parts: Vec<String> = Vec::new();
        for t in tools_default_permit {
            parts.push(t.clone());
        }
        for t in tools_action_authorize {
            parts.push(format!("{t} (action)"));
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(", "))
        }
    }
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

/// Get the profile file path for an app.
///
/// Foundation §7.3 canonical path: `~/.config/permissions/{app_id}.toml`.
/// The user owns this file. The optional `ARLEN_PERMISSIONS_DIR` env
/// override is for tests and dev sandboxes only — never set in
/// production.
pub fn profile_path(app_id: &str) -> Result<PathBuf, PermissionError> {
    // The id is interpolated into a filesystem path, so it MUST be a single safe
    // path component - the same guard `system_profile_path` applies. Validating
    // here makes the function fail-safe for EVERY caller: an id like `/etc/x`
    // (absolute, discards the base) or `../../etc/x` (lexical escape) can never
    // produce a path outside the permissions directory, even if a caller forgot to
    // pre-check it. Callers that resolve the id from a kernel-attested peer, or
    // charset-check it first, are unaffected (every real app id passes).
    if !is_valid_app_id(app_id) {
        return Err(PermissionError::InvalidAppId {
            app_id: app_id.to_string(),
        });
    }
    Ok(permissions_dir()
        .ok_or(PermissionError::NoHomeDir)?
        .join(format!("{app_id}.toml")))
}

/// The directory user profiles live in: `$ARLEN_PERMISSIONS_DIR` when set (tests
/// and dev sandboxes only), else `~/.config/permissions`.
///
/// The single statement of that location. It used to be written out in three
/// places - here, the profile watcher, and installd's module installer - and the
/// three had to agree for the system to work at all: if the watcher ever watched a
/// different directory than the loaders read, no profile change would reach the
/// Living Capability Graph and nothing would report the silence. Two of the three
/// now call this; installd is a separate crate and says so at its own copy.
pub fn permissions_dir() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("ARLEN_PERMISSIONS_DIR") {
        return Some(PathBuf::from(p));
    }
    Some(dirs::home_dir()?.join(".config").join("permissions"))
}

/// Whether `app_id` is a safe single path component for joining into a
/// root-owned profile path: a non-empty reverse-DNS-style id over `[A-Za-z0-9._-]`
/// with no traversal (`..`, leading/trailing dot, or any path separator — the
/// charset already excludes `/`). A root-owned path must never be built from an
/// unvalidated id, so [`system_profile_path`] returns `None` for an invalid one
/// rather than touching `/var/lib`. `pub` so the identity broker can apply the same
/// charset guard on the register side, symmetric with the resolver.
///
/// ASCII case is accepted because a real app id carries it: `org.gnome.Calculator`,
/// `app.drey.Biblioteca` and `md.obsidian.Obsidian` are the ids those apps actually
/// ship, and the reverse-DNS convention capitalises the final component almost
/// everywhere. This guard exists to keep an id a single traversal-free path
/// component, and an uppercase letter is no less safe a path component than a
/// lowercase one; rejecting it was over-tight for that purpose and made every such
/// app unaddressable. Ids are compared and stored verbatim, never case-folded, so
/// two ids differing only in case are two apps - which is how a case-sensitive
/// filesystem and Flatpak itself already treat them.
pub fn is_valid_app_id(app_id: &str) -> bool {
    !app_id.is_empty()
        && app_id != ".."
        && !app_id.starts_with('.')
        && !app_id.ends_with('.')
        && !app_id.contains("..")
        && app_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// The system-tier (root-owned) profile path for `app_id`, or `None` if the id is
/// not a safe path component (F3 Rung A). System-installed apps get a profile
/// under `/var/lib/arlen/permissions/{uid}/{app_id}.toml`, written only through the
/// root `permission-helper`, so a same-uid process cannot forge it
/// (AUTH-CANONICAL.md §2). The `ARLEN_SYSTEM_PERMISSIONS_DIR` override (tests/dev
/// only, never production) resolves directly to `<dir>/{app_id}.toml` with no uid
/// subdir, mirroring `ARLEN_PERMISSIONS_DIR`.
fn system_profile_path(app_id: &str) -> Option<PathBuf> {
    if !is_valid_app_id(app_id) {
        return None;
    }
    Some(system_permissions_dir().join(format!("{app_id}.toml")))
}

/// The directory system-tier profiles live in for the current uid:
/// `/var/lib/arlen/permissions/{uid}`, or `$ARLEN_SYSTEM_PERMISSIONS_DIR` when set
/// (tests and dev sandboxes only, and then with no uid subdir - the override IS
/// the directory).
///
/// `pub` because a surface that lists what is installed has to read this tier, and
/// having no way to ask led the desktop shell to write the path out for itself -
/// with the USER-tier override name attached to it, so under a test harness it
/// reads a directory nothing writes. Two overrides that mean different tiers are
/// exactly the pair a second copy gets wrong.
///
/// Infallible, unlike [`permissions_dir`]: the system tier is an absolute path
/// that needs no home directory.
pub fn system_permissions_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("ARLEN_SYSTEM_PERMISSIONS_DIR") {
        return PathBuf::from(dir);
    }
    // SAFETY: getuid never fails.
    let uid = unsafe { libc::getuid() };
    PathBuf::from("/var/lib/arlen/permissions").join(uid.to_string())
}

/// Every path a profile for `app_id` could live at, system tier first, in the
/// precedence order [`load_tiered`] resolves them.
///
/// Exists because "which files decide this app's authority" was being answered
/// independently at each site that needed it, and each one answered "the user
/// file" - so a system-tier profile could be installed, narrowed or removed
/// while a token cache kept serving the old grants, a grant projection deleted
/// an installed app's grants as though it had been uninstalled, and a file
/// watcher reported nothing at all. Anything that stats, watches or invalidates
/// on a profile should ask here rather than rebuild the list.
///
/// The user path is absent when there is no home directory; the system path is
/// always present because it needs none.
pub fn profile_paths(app_id: &str) -> Vec<PathBuf> {
    [
        Some(system_permissions_dir().join(format!("{app_id}.toml"))),
        profile_path(app_id).ok(),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Whether a profile for `app_id` exists in **either** tier. A system-tier-only
/// app - the shape an apt-installed one has - is installed just as much as a
/// user-tier one, so a caller asking "is this app still around" must not answer
/// from one tier alone.
pub fn profile_exists(app_id: &str) -> bool {
    profile_paths(app_id).iter().any(|p| p.exists())
}

/// Resolve a profile across the two tiers (F3 Rung A semantics): if a root-owned
/// system-tier profile exists it is **authoritative and wins outright** — the user
/// `~/.config` overlay is ignored for that app_id. This is the conservative,
/// correct-by-construction behaviour (a naive union would let a same-uid user
/// *widen* a system app's grants, the exact F3 hole); a tighten-only overlay that
/// may only narrow the system ceiling is a noted follow-up, not done here. When no
/// system base exists, the user-config profile is loaded as before.
fn load_tiered(
    system: Option<&Path>,
    user: &Path,
    app_id: &str,
) -> Result<PermissionProfile, PermissionError> {
    if let Some(sys) = system {
        if sys.exists() {
            return load_profile_from(sys, app_id);
        }
    }
    load_profile_from(user, app_id)
}

/// Load a permission profile, preferring the root-owned system tier over the user
/// `~/.config` tier (F3 Rung A — see [`load_tiered`] for the system-base-wins
/// semantics).
pub fn load_profile(app_id: &str) -> Result<PermissionProfile, PermissionError> {
    let system = system_profile_path(app_id);
    let user = profile_path(app_id)?;
    load_tiered(system.as_deref(), &user, app_id)
}

/// Load from an explicit path (for testing).
pub fn load_profile_from(
    path: &Path,
    app_id: &str,
) -> Result<PermissionProfile, PermissionError> {
    if !path.exists() {
        return Err(PermissionError::NotFound {
            app_id: app_id.into(),
        });
    }
    let content = std::fs::read_to_string(path)?;
    toml::from_str(&content).map_err(|e| PermissionError::Parse(e.to_string()))
}

// ---------------------------------------------------------------------------
// Pattern matching
// ---------------------------------------------------------------------------

/// Check if any pattern in `patterns` matches `value`.
/// `"com.app.*"` matches `"com.app.Note"` and `"com.app.Deck"`.
/// `"com.app.Note"` matches only itself.
fn pattern_matches(patterns: &[String], value: &str) -> bool {
    patterns.iter().any(|p| {
        if let Some(prefix) = p.strip_suffix(".*") {
            value.starts_with(prefix) && value[prefix.len()..].starts_with('.')
        } else {
            p == value
        }
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn every_curated_starting_profile_is_valid_and_conservative() {
        // The curated starting profiles (the apt-enroll hook's source set) must
        // each parse against the live schema and stay fail-closed conservative:
        // third-party tier, no Knowledge Graph grant, filename == declared id.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("profiles");
        let mut count = 0usize;
        for entry in std::fs::read_dir(&dir).expect("the curated profiles dir exists") {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let content = std::fs::read_to_string(&path).unwrap();
            let profile: PermissionProfile = toml::from_str(&content)
                .unwrap_or_else(|e| panic!("{} does not parse: {e}", path.display()));
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap();
            assert_eq!(
                stem,
                profile.info.app_id,
                "{}: filename must equal info.app_id",
                path.display()
            );
            // A curated profile is only worth anything if the loader can address
            // the id it is filed under. This was silently false for 478 of them
            // while the guard demanded lowercase: correctly named, correctly
            // parsed and permanently unloadable. Filename-equals-id did not catch
            // it, because both sides were equally unaddressable.
            assert!(
                profile_path(stem).is_ok(),
                "{}: no profile path can be built for this id",
                path.display()
            );
            assert_eq!(
                profile.info.tier,
                AppTier::ThirdParty,
                "{}: a curated app profile must be third-party tier",
                path.display()
            );
            assert!(
                profile.graph.read.is_empty() && profile.graph.write.is_empty(),
                "{}: a starting profile must not grant Knowledge Graph access",
                path.display()
            );
            // The curated corpus is the review baseline, so every profile must
            // itself clear the submission hard-deny lint (§E8).
            let deny = crate::lint::hard_deny_reasons(&profile);
            assert!(
                deny.is_empty(),
                "{}: a curated profile must clear the hard-deny lint, got {deny:?}",
                path.display()
            );
            count += 1;
        }
        assert!(count >= 8, "expected the curated starting profiles, found {count}");
    }

    #[test]
    fn a_curated_profile_id_is_the_id_enrollment_resolves_to() {
        // The corpus is keyed by app_id, and an app_id is only ever produced by
        // `path_to_app_id`. These two facts are asserted in separate places and
        // nothing tied them together, which invites the wrong conclusion about
        // what a profile is for: `firefox.toml` does NOT apply to a distro
        // `/usr/bin/firefox`, because that path matches no resolution rule and
        // errors (see `test_app_id_from_path_unknown`). It applies to a firefox
        // ENROLLED under `/usr/lib/arlen/apps/firefox/`, where the directory
        // name is the id.
        //
        // So this pins the pairing: for a real profile from the corpus, the
        // canonical enrolled path built from its filename must resolve back to
        // exactly that filename. If the enrollment layout or rule (3) ever
        // changes shape, every profile in the corpus becomes unreachable at
        // once, and that should fail here rather than at a user's first launch.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("profiles");
        let mut checked = 0usize;
        for entry in std::fs::read_dir(&dir).expect("the curated profiles dir exists") {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let id = path.file_stem().and_then(|s| s.to_str()).unwrap().to_string();
            let installed = std::path::PathBuf::from(format!("/usr/lib/arlen/apps/{id}/bin/{id}"));
            assert_eq!(
                crate::identity::path_to_app_id(&installed).ok(),
                Some(id.clone()),
                "the enrolled path for {id} does not resolve back to its profile id",
            );
            checked += 1;
            // The property is uniform over the corpus; a sample keeps the test
            // fast while still catching a change to the layout or the rule.
            if checked >= 50 {
                break;
            }
        }
        assert!(checked >= 8, "expected profiles to sample, found {checked}");
    }

    const SAMPLE_PROFILE: &str = r#"
[info]
app_id = "com.example.notes"
tier = "third-party"

[graph]
read = ["com.example.notes.*", "shared.Person"]
write = ["com.example.notes.*"]
app_isolated = true

[event_bus]
publish = ["com.example.notes.*"]
subscribe = ["com.example.notes.*", "config.changed"]

[filesystem]
documents = true
downloads = true
custom = ["/tmp/notes"]

[network]
allowed_domains = ["api.example.com", "cdn.example.com"]

[notifications]
enabled = true

[clipboard]
read = true
write = true

[system]
autostart = false
background = true
"#;

    fn write_profile(dir: &Path, content: &str) -> PathBuf {
        let path = dir.join("com.example.notes.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    // ── Round-trip ──

    #[test]
    fn test_roundtrip() {
        let profile: PermissionProfile = toml::from_str(SAMPLE_PROFILE).unwrap();
        assert_eq!(profile.info.app_id, "com.example.notes");
        assert_eq!(profile.info.tier, AppTier::ThirdParty);

        let serialized = toml::to_string_pretty(&profile).unwrap();
        let reparsed: PermissionProfile = toml::from_str(&serialized).unwrap();
        assert_eq!(reparsed.info.app_id, "com.example.notes");
        assert_eq!(reparsed.graph.read.len(), profile.graph.read.len());
    }

    // ── MCP ──

    #[test]
    fn test_mcp_permissions_roundtrip() {
        let toml_str = r#"
[info]
app_id = "com.example.files"
tier = "first-party"

[mcp]
tools_default_permit = ["list_directory", "file_metadata"]
tools_action_authorize = ["move_file", "delete_file"]
always_confirm_overrides = ["empty_trash"]
"#;
        let profile: PermissionProfile = toml::from_str(toml_str).unwrap();
        assert!(profile.mcp.is_read_only("list_directory"));
        assert!(!profile.mcp.is_read_only("move_file"));
        assert!(profile.mcp.requires_authorization("delete_file"));
        assert!(!profile.mcp.requires_authorization("list_directory"));
        assert_eq!(profile.mcp.always_confirm_overrides, vec!["empty_trash"]);

        let reparsed: PermissionProfile =
            toml::from_str(&toml::to_string_pretty(&profile).unwrap()).unwrap();
        assert_eq!(reparsed.mcp, profile.mcp);
    }

    #[test]
    fn test_mcp_permissions_default_empty() {
        // A profile with no [mcp] section parses with empty lists.
        let profile: PermissionProfile = toml::from_str(SAMPLE_PROFILE).unwrap();
        assert!(profile.mcp.tools_default_permit.is_empty());
        assert!(profile.mcp.tools_action_authorize.is_empty());
    }

    // ── Loading ──

    #[test]
    fn test_load_from_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = write_profile(dir.path(), SAMPLE_PROFILE);
        let profile = load_profile_from(&path, "com.example.notes").unwrap();
        assert_eq!(profile.info.app_id, "com.example.notes");
        assert!(profile.graph.app_isolated);
        assert!(profile.filesystem.documents);
        assert!(!profile.filesystem.home);
    }

    #[test]
    fn test_load_not_found() {
        let result = load_profile_from(
            Path::new("/tmp/nonexistent-xyz.toml"),
            "com.missing",
        );
        assert!(matches!(result, Err(PermissionError::NotFound { .. })));
    }

    // ── F3 Rung A: the system tier ──

    #[test]
    fn system_profile_path_validates_the_app_id() {
        let p = system_profile_path("com.example.notes").expect("a valid id resolves");
        assert!(p.ends_with("com.example.notes.toml"));
        // Without the test override, the default is the root-owned /var/lib path.
        if std::env::var("ARLEN_SYSTEM_PERMISSIONS_DIR").is_err() {
            assert!(p.to_string_lossy().contains("/var/lib/arlen/permissions"));
        }
        // A root-owned path is never built from an unsafe id.
        for bad in ["..", "a/b", "../etc/x", "", ".hidden", "trailing.", "com.exämple.x"] {
            assert!(system_profile_path(bad).is_none(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn the_two_tiers_have_separate_directories_and_separate_overrides() {
        // The pair a second copy gets wrong: both tiers hold profiles named after
        // the same app ids, so reading the wrong one shows the wrong answer with
        // no error - and each tier has its OWN override, so honouring the user
        // variable while reading the system path points a test at a directory
        // nothing writes.
        let system = system_permissions_dir();
        let user = profile_path("com.example.notes")
            .expect("a valid id resolves")
            .parent()
            .expect("a profile path has a directory")
            .to_path_buf();
        assert_ne!(system, user, "the tiers must not resolve to one directory");
        // And the system path a profile lands on is inside the directory this
        // reports, so a lister and a loader agree about where to look.
        assert!(system_profile_path("com.example.notes")
            .expect("a valid id resolves")
            .starts_with(&system));
    }

    #[test]
    fn profile_paths_covers_both_tiers_in_resolution_order() {
        // Every site that stats, watches or invalidates on a profile has to know
        // BOTH files decide the answer. Answering from the user tier alone is the
        // mistake this exists to stop: a system-tier-only app then looks absent,
        // which a cache reads as "nothing changed" and a projection reads as
        // "uninstalled, delete its grants".
        let paths = profile_paths("com.example.notes");
        assert_eq!(paths.len(), 2, "both tiers, given a home directory");
        assert!(
            paths[0].starts_with(system_permissions_dir()),
            "system tier first, matching the order load_tiered resolves them"
        );
        let user_dir = profile_path("com.example.notes")
            .expect("a valid id resolves")
            .parent()
            .expect("a profile path has a directory")
            .to_path_buf();
        assert!(paths[1].starts_with(&user_dir));
        for p in &paths {
            assert!(p.ends_with("com.example.notes.toml"));
        }
        // An id that cannot form a safe system path still yields the user path
        // rather than silently reporting no profile locations at all.
        assert!(profile_paths("com.example.notes")
            .iter()
            .all(|p| p.is_absolute()));
    }

    #[test]
    fn the_watcher_watches_the_directory_the_loaders_read() {
        // The invariant the whole projection rests on. If these two ever differ,
        // profiles are read from one place and watched in another, so an edit
        // lands and nothing notices - no error anywhere, the Living Capability
        // Graph just quietly stops matching the profiles on disk.
        let read_from = profile_path("com.example.notes")
            .expect("a valid id resolves")
            .parent()
            .expect("a profile path has a directory")
            .to_path_buf();
        assert_eq!(read_from, crate::ProfileWatcher::permissions_dir());
    }

    #[test]
    fn a_real_world_reverse_dns_id_is_addressable() {
        // Regression: this guard used to demand lowercase, so every app whose id
        // capitalises its last component - the ordinary convention, and what 478
        // of the curated profiles are named after - resolved through
        // `path_to_app_id` and was then refused by `profile_path`. The profile was
        // written and could never be loaded, so the app's connection was rejected
        // outright. Fails closed, but it broke every such app.
        for id in [
            "org.gnome.Calculator",
            "app.drey.Biblioteca",
            "md.obsidian.Obsidian",
            "com.example.notes",
        ] {
            assert!(is_valid_app_id(id), "{id} is a real app id and must resolve");
            assert!(profile_path(id).is_ok(), "{id} must build a profile path");
            assert!(system_profile_path(id).is_some(), "{id} must build a system path");
        }
    }

    #[test]
    fn profile_path_rejects_a_traversal_id() {
        // The user-config path is fail-safe too: a traversal id can never build a
        // path outside the permissions dir, even without a caller-side pre-check.
        for bad in ["..", "a/b", "../etc/x", "/etc/x", "", ".hidden", "trailing.", "com.exämple.x"] {
            assert!(
                matches!(profile_path(bad), Err(PermissionError::InvalidAppId { .. })),
                "{bad:?} must be rejected as an invalid app id"
            );
        }
        // A real app id is never rejected as invalid (it resolves, or fails only
        // for want of a home dir - never InvalidAppId).
        assert!(!matches!(
            profile_path("com.example.notes"),
            Err(PermissionError::InvalidAppId { .. })
        ));
    }

    #[test]
    fn load_tiered_prefers_the_system_base() {
        const USER_PROFILE: &str = "[info]\napp_id = \"com.example.notes\"\ntier = \"first-party\"\n";
        let sys_dir = tempfile::TempDir::new().unwrap();
        let user_dir = tempfile::TempDir::new().unwrap();
        // System base is third-party (SAMPLE); the user overlay claims first-party.
        let sys_path = write_profile(sys_dir.path(), SAMPLE_PROFILE);
        let user_path = user_dir.path().join("com.example.notes.toml");
        std::fs::write(&user_path, USER_PROFILE).unwrap();

        let loaded = load_tiered(Some(&sys_path), &user_path, "com.example.notes").unwrap();
        assert_eq!(
            loaded.info.tier,
            AppTier::ThirdParty,
            "the root-owned system base wins outright; the user overlay cannot widen it"
        );
    }

    #[test]
    fn load_tiered_falls_back_to_user_without_a_system_base() {
        let user_dir = tempfile::TempDir::new().unwrap();
        let user_path = write_profile(user_dir.path(), SAMPLE_PROFILE);
        // A system path that does not exist falls through to the user tier.
        let missing = user_dir.path().join("absent").join("system.toml");
        let loaded = load_tiered(Some(&missing), &user_path, "com.example.notes").unwrap();
        assert_eq!(loaded.info.app_id, "com.example.notes");
        // No system tier at all also falls through.
        let loaded2 = load_tiered(None, &user_path, "com.example.notes").unwrap();
        assert_eq!(loaded2.info.app_id, "com.example.notes");
    }

    // ── Tier detection ──

    #[test]
    fn test_detect_tier_system() {
        assert_eq!(
            detect_tier(Path::new("/usr/lib/arlen/apps/system-monitor/bin/sm")),
            AppTier::System
        );
        assert_eq!(
            detect_tier(Path::new("/usr/bin/arlen-graph-daemon")),
            AppTier::System
        );
    }

    #[test]
    fn test_detect_tier_third_party() {
        assert_eq!(
            detect_tier(Path::new("/home/user/.local/share/flatpak/app/com.app/bin/app")),
            AppTier::ThirdParty
        );
    }

    // ── Graph permissions ──

    #[test]
    fn test_graph_read_exact() {
        let g = GraphPermissions {
            read: vec!["shared.Person".into()],
            ..Default::default()
        };
        assert!(g.can_read("shared.Person"));
        assert!(!g.can_read("shared.Organization"));
    }

    #[test]
    fn test_graph_read_wildcard() {
        let g = GraphPermissions {
            read: vec!["com.app.*".into()],
            ..Default::default()
        };
        assert!(g.can_read("com.app.Note"));
        assert!(g.can_read("com.app.Deck"));
        assert!(!g.can_read("com.other.Note"));
    }

    #[test]
    fn test_graph_write() {
        let g = GraphPermissions {
            write: vec!["com.app.*".into()],
            ..Default::default()
        };
        assert!(g.can_write("com.app.Note"));
        assert!(!g.can_write("shared.Person"));
    }

    #[test]
    fn test_annotations_own_namespace_always_allowed() {
        // Empty cross-namespace allowlist — own namespace is still
        // allowed because the API has no concept of forbidding it.
        let g = GraphPermissions::default();
        assert!(g.can_read_annotations_from("com.example.editor", "com.example.editor"));
    }

    #[test]
    fn test_annotations_cross_namespace_denied_by_default() {
        let g = GraphPermissions::default();
        assert!(!g.can_read_annotations_from("com.example.editor", "com.example.git"));
    }

    #[test]
    fn test_annotations_cross_namespace_explicit_allow() {
        let g = GraphPermissions {
            annotations_read_cross_namespace: vec!["com.example.git".into()],
            ..Default::default()
        };
        assert!(g.can_read_annotations_from("com.example.editor", "com.example.git"));
        // Allowlist is exact / pattern based — unrelated namespace
        // still denied.
        assert!(!g.can_read_annotations_from(
            "com.example.editor",
            "com.malicious.read-everything"
        ));
    }

    #[test]
    fn test_annotations_cross_namespace_wildcard() {
        let g = GraphPermissions {
            annotations_read_cross_namespace: vec!["com.example.*".into()],
            ..Default::default()
        };
        assert!(g.can_read_annotations_from("com.example.editor", "com.example.git"));
        assert!(g.can_read_annotations_from("com.example.editor", "com.example.notes"));
        assert!(!g.can_read_annotations_from("com.example.editor", "com.other.app"));
    }

    // ── Event Bus permissions ──

    #[test]
    fn test_event_bus_publish() {
        let e = EventBusPermissions {
            publish: vec!["com.app.*".into()],
            ..Default::default()
        };
        assert!(e.can_publish("com.app.note_created"));
        assert!(!e.can_publish("system.shutdown"));
    }

    #[test]
    fn test_event_bus_subscribe() {
        let e = EventBusPermissions {
            subscribe: vec!["com.app.*".into(), "config.changed".into()],
            ..Default::default()
        };
        assert!(e.can_subscribe("com.app.note_created"));
        assert!(e.can_subscribe("config.changed"));
        assert!(!e.can_subscribe("window.focused"));
    }

    // ── Network subdomain matching ──

    #[test]
    fn test_domain_exact() {
        let n = NetworkPermissions {
            allowed_domains: vec!["example.com".into()],
            ..Default::default()
        };
        assert!(n.is_domain_allowed("example.com"));
        assert!(!n.is_domain_allowed("other.com"));
    }

    #[test]
    fn test_domain_subdomain() {
        let n = NetworkPermissions {
            allowed_domains: vec!["example.com".into()],
            ..Default::default()
        };
        assert!(n.is_domain_allowed("api.example.com"));
        assert!(n.is_domain_allowed("cdn.api.example.com"));
        assert!(!n.is_domain_allowed("exampleX.com"));
        assert!(!n.is_domain_allowed("notexample.com"));
    }

    #[test]
    fn test_domain_case_insensitive() {
        let n = NetworkPermissions {
            allowed_domains: vec!["Example.COM".into()],
            ..Default::default()
        };
        assert!(n.is_domain_allowed("example.com"));
        assert!(n.is_domain_allowed("API.EXAMPLE.COM"));
    }

    #[test]
    fn test_domain_allow_all() {
        let n = NetworkPermissions {
            allow_all: true,
            ..Default::default()
        };
        assert!(n.is_domain_allowed("anything.com"));
    }

    #[test]
    fn reach_summary_projects_the_declared_network_reach() {
        // allow_all -> "all".
        assert_eq!(
            NetworkPermissions { allow_all: true, ..Default::default() }.reach_summary(),
            Some("all".to_string()),
        );
        // No declared reach -> None (the app shows no internet access).
        assert_eq!(NetworkPermissions::default().reach_summary(), None);
        // A domain list -> sorted, deduped, lowercased, comma-joined.
        let n = NetworkPermissions {
            allow_all: false,
            allowed_domains: vec!["github.com".into(), "api.openai.com".into()],
        };
        assert_eq!(n.reach_summary(), Some("api.openai.com,github.com".to_string()));
        // Case/whitespace/duplicate normalisation; blanks dropped.
        let messy = NetworkPermissions {
            allow_all: false,
            allowed_domains: vec!["GitHub.com".into(), " github.com ".into(), "  ".into()],
        };
        assert_eq!(messy.reach_summary(), Some("github.com".to_string()));
    }

    #[test]
    fn reach_summary_projects_every_dimension() {
        // Each dimension: default (no declaration) projects None; a declared reach
        // projects a non-empty consent-scope string.
        assert_eq!(GraphPermissions::default().reach_summary(), None);
        assert_eq!(
            GraphPermissions {
                read: vec!["system.File".into()],
                write: vec!["com.app.Note".into()],
                ..Default::default()
            }
            .reach_summary(),
            Some("read system.File; write com.app.Note".to_string()),
        );

        assert_eq!(EventBusPermissions::default().reach_summary(), None);
        assert_eq!(
            EventBusPermissions {
                subscribe: vec!["file.opened".into()],
                publish: vec![],
            }
            .reach_summary(),
            Some("hears file.opened".to_string()),
        );

        assert_eq!(FilesystemPermissions::default().reach_summary(), None);
        assert_eq!(
            FilesystemPermissions { documents: true, ..Default::default() }.reach_summary(),
            Some("documents".to_string()),
        );

        assert_eq!(NotificationPermissions::default().reach_summary(), None);
        assert_eq!(
            NotificationPermissions { enabled: true }.reach_summary(),
            Some("on".to_string()),
        );

        assert_eq!(ClipboardPermissions::default().reach_summary(), None);
        assert_eq!(
            ClipboardPermissions { read: true, write: true, ..Default::default() }.reach_summary(),
            Some("read, write".to_string()),
        );

        assert_eq!(SystemPermissions::default().reach_summary(), None);
        assert_eq!(
            SystemPermissions { autostart: true, ..Default::default() }.reach_summary(),
            Some("autostart".to_string()),
        );

        assert_eq!(SearchPermissions::default().reach_summary(), None);
        assert_eq!(
            SearchPermissions { open: true, ..Default::default() }.reach_summary(),
            Some("open launcher".to_string()),
        );

        assert_eq!(IntentsPermissions::default().reach_summary(), None);
        assert_eq!(
            IntentsPermissions { dispatch: true, ..Default::default() }.reach_summary(),
            Some("dispatch".to_string()),
        );

        assert_eq!(InputPermissions::default().reach_summary(), None);
        assert_eq!(
            InputPermissions {
                register_focused_bindings: true,
                register_global_bindings: false,
            }
            .reach_summary(),
            Some("focused bindings".to_string()),
        );

        assert_eq!(McpPermissions::default().reach_summary(), None);
        assert_eq!(
            McpPermissions {
                tools_default_permit: vec!["search".into()],
                tools_action_authorize: vec!["send".into()],
                ..Default::default()
            }
            .reach_summary(),
            Some("search, send (action)".to_string()),
        );
    }

    // ── Defaults ──

    #[test]
    fn test_minimal_profile() {
        let minimal = r#"
[info]
app_id = "com.test"
"#;
        let profile: PermissionProfile = toml::from_str(minimal).unwrap();
        assert_eq!(profile.info.tier, AppTier::ThirdParty); // default
        assert!(!profile.graph.app_isolated);
        assert!(profile.graph.read.is_empty());
        assert!(!profile.network.allow_all);
        assert!(!profile.notifications.enabled);
        assert!(!profile.search.open);
        assert!(!profile.search.register_handler);
        assert!(!profile.search.intercept_all);
    }

    // ── Search permissions ──

    #[test]
    fn search_default_deny() {
        let toml = r#"
[info]
app_id = "com.test"
"#;
        let p: PermissionProfile = toml::from_str(toml).unwrap();
        assert!(!p.search.open, "search.open must default to false");
    }

    #[test]
    fn search_explicit_grant() {
        let toml = r#"
[info]
app_id = "com.test"
[search]
open = true
"#;
        let p: PermissionProfile = toml::from_str(toml).unwrap();
        assert!(p.search.open);
        // Reserved Phase-7 fields default-deny even with explicit [search] section
        assert!(!p.search.register_handler);
        assert!(!p.search.intercept_all);
    }

    #[test]
    fn search_phase7_fields_parse_without_being_honored() {
        // Forward-compat: profiles authored against the Phase-7 schema
        // must parse cleanly today even though no current broker reads
        // these flags.
        let toml = r#"
[info]
app_id = "com.test"
[search]
open = true
register_handler = true
intercept_all = true
"#;
        let p: PermissionProfile = toml::from_str(toml).unwrap();
        assert!(p.search.open);
        assert!(p.search.register_handler);
        assert!(p.search.intercept_all);
    }

    // ── Intents permissions ──

    #[test]
    fn intents_default_deny() {
        let toml = r#"
[info]
app_id = "com.test"
"#;
        let p: PermissionProfile = toml::from_str(toml).unwrap();
        assert!(!p.intents.dispatch, "intents.dispatch must default to false");
        assert!(!p.intents.register);
        assert!(!p.intents.preferences);
    }

    #[test]
    fn intents_explicit_grant() {
        let toml = r#"
[info]
app_id = "com.test"
[intents]
dispatch = true
"#;
        let p: PermissionProfile = toml::from_str(toml).unwrap();
        assert!(p.intents.dispatch);
        // Reserved Phase-7 fields default-deny even with explicit section
        assert!(!p.intents.register);
        assert!(!p.intents.preferences);
    }

    #[test]
    fn intents_phase7_fields_parse_without_being_honored() {
        // Forward-compat for Phase-7 schema.
        let toml = r#"
[info]
app_id = "com.test"
[intents]
dispatch = true
register = true
preferences = true
"#;
        let p: PermissionProfile = toml::from_str(toml).unwrap();
        assert!(p.intents.dispatch);
        assert!(p.intents.register);
        assert!(p.intents.preferences);
    }

    // ── Pattern matching ──

    #[test]
    fn test_pattern_matches() {
        assert!(pattern_matches(&["com.app.*".into()], "com.app.Note"));
        assert!(pattern_matches(&["com.app.Note".into()], "com.app.Note"));
        assert!(!pattern_matches(&["com.app.*".into()], "com.app"));
        assert!(!pattern_matches(&["com.app.*".into()], "com.other.Note"));
        assert!(!pattern_matches(&[], "anything"));
    }

    // ── Input permissions ──

    #[test]
    fn input_permissions_parse() {
        let toml = r#"
[info]
app_id = "com.example"
[input]
register_global_bindings = true
register_focused_bindings = true
"#;
        let profile: PermissionProfile = toml::from_str(toml).unwrap();
        assert!(profile.input.register_global_bindings);
        assert!(profile.input.register_focused_bindings);
    }

    #[test]
    fn input_defaults_by_tier() {
        let third = InputPermissions::defaults_for_tier(AppTier::ThirdParty);
        assert!(third.register_focused_bindings);
        assert!(!third.register_global_bindings);

        let first = InputPermissions::defaults_for_tier(AppTier::FirstParty);
        assert!(first.register_focused_bindings);
        assert!(first.register_global_bindings);

        let system = InputPermissions::defaults_for_tier(AppTier::System);
        assert!(system.register_global_bindings);
    }

    #[test]
    fn input_apply_manifest_requests() {
        let mut p = InputPermissions::default();
        p.apply_manifest_requests(&[
            "register_focused_bindings".into(),
            "register_global_bindings".into(),
            "unknown_future_flag".into(),
        ]);
        assert!(p.register_focused_bindings);
        assert!(p.register_global_bindings);
    }

    #[test]
    fn input_section_optional() {
        // Profiles that predate the input section must still parse.
        let toml = r#"
[info]
app_id = "com.legacy"
"#;
        let profile: PermissionProfile = toml::from_str(toml).unwrap();
        assert!(!profile.input.register_focused_bindings);
        assert!(!profile.input.register_global_bindings);
    }
}
