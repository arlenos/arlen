//! SX-1: the consent gate a module passes before it goes active.
//!
//! A module became active purely by being present and enabled. Whatever its
//! manifest declared - graph writes, network reach, clipboard reads - it simply
//! got, with no decision anywhere and nothing in the audit ledger. The consent
//! broker and its friction ladder were built for apps and never called from
//! here (`shell-extension-model.md`, Call 1: enforcement is manifest
//! `[capabilities]` plus consent plus audit plus revoke).
//!
//! This module is two halves. [`describe`] is the pure classification: what a
//! manifest's capabilities amount to, in the words the dialog shows and at the
//! severity the broker classifies against. [`request_enable_consent`] is the
//! client, mirroring the portal's (4-byte little-endian length prefix then
//! JSON, both directions).
//!
//! FAIL-CLOSED: a broker that is down, a framing or IO error, or an oversized
//! reply resolves to a refusal. A module that cannot obtain consent does not go
//! active. That is the safe direction here - the cost is a module that stays
//! off until the broker is up, against a module that silently gains graph
//! writes because the one thing that would have asked was unreachable.

use std::path::{Path, PathBuf};

use arlen_consent_contract::{
    ActionKind, ConsentClass, ConsentOutcome, IntakeResult, RequestBody,
};
use arlen_modules::ModuleCapabilities;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

/// The largest intake reply frame accepted, matching the broker's `MAX_FRAME`.
const MAX_FRAME: usize = 64 * 1024;

/// What enabling a module would grant, classified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilitySummary {
    /// One human phrase per declared capability, in a fixed order so the same
    /// manifest always reads the same way. Empty when the module declares
    /// nothing.
    pub grants: Vec<String>,
    /// The severity the broker classifies against.
    pub kind: ActionKind,
}

impl CapabilitySummary {
    /// Whether enabling this module is a decision at all. A module that
    /// declares no capabilities gains nothing by being enabled, so prompting
    /// would be friction with no content - the user would be asked to approve
    /// the empty set.
    pub fn needs_consent(&self) -> bool {
        !self.grants.is_empty()
    }

    /// The dialog's one-line summary.
    pub fn summary(&self, module_id: &str) -> String {
        if self.grants.is_empty() {
            return format!("Enable {module_id}");
        }
        format!("Enable {module_id}, which can {}", self.grants.join(", "))
    }
}

/// Classify a manifest's declared capabilities.
///
/// The severity is not cosmetic: `always_requires_confirmation` is what stops
/// the broker resolving a request silently, so anything that reaches out of the
/// machine or writes the user's knowledge graph must not classify `Ordinary`.
///
/// - **Graph WRITE** is `SystemConfigChange`: a module writing the knowledge
///   graph is writing the substrate every other part of the system reasons
///   over, and unlike a file it is not obvious afterwards what changed.
/// - **Network** is `UndeclaredNetwork`: reach off the machine is the
///   exfiltration edge, and the domain allowlist bounds where, not whether.
/// - **Clipboard READ** is `Irreversible`: the clipboard carries passwords and
///   tokens in passing, and a read cannot be taken back.
/// - Graph read, event-bus, storage, notifications and clipboard write are
///   `Ordinary`: each is bounded, observable, or gives the module nothing it
///   could not already infer from the shell it renders in.
///
/// The highest severity present wins, because a module is enabled as a whole.
pub fn describe(caps: &ModuleCapabilities) -> CapabilitySummary {
    let mut grants = Vec::new();
    let mut kind = ActionKind::Ordinary;
    // Raise to `candidate` unless we already hold something at least as severe.
    let raise = |current: &mut ActionKind, candidate: ActionKind| {
        if !current.always_requires_confirmation() {
            *current = candidate;
        }
    };

    if let Some(graph) = &caps.graph {
        if !graph.read.is_empty() {
            grants.push(format!("read {} from your knowledge graph", graph.read.join(", ")));
        }
        if !graph.write.is_empty() {
            grants.push(format!("write {} to your knowledge graph", graph.write.join(", ")));
            raise(&mut kind, ActionKind::SystemConfigChange);
        }
    }
    if let Some(net) = &caps.network {
        if !net.allowed_domains.is_empty() {
            grants.push(format!("connect to {}", net.allowed_domains.join(", ")));
            raise(&mut kind, ActionKind::UndeclaredNetwork);
        }
    }
    if let Some(clip) = &caps.clipboard {
        if clip.read {
            grants.push("read your clipboard".to_string());
            raise(&mut kind, ActionKind::Irreversible);
        }
        if clip.write {
            grants.push("set your clipboard".to_string());
        }
    }
    if let Some(bus) = &caps.event_bus {
        if !bus.subscribe.is_empty() {
            grants.push(format!("observe {} events", bus.subscribe.join(", ")));
        }
        if !bus.publish.is_empty() {
            grants.push(format!("publish {} events", bus.publish.join(", ")));
        }
    }
    if let Some(storage) = &caps.storage {
        grants.push(format!("store up to {} MB", storage.quota_mb));
    }
    if caps.notifications {
        grants.push("send you notifications".to_string());
    }

    CapabilitySummary { grants, kind }
}

/// The broker's intake socket: `$XDG_RUNTIME_DIR/arlen/consent-intake.sock`,
/// else `/run/arlen/consent-intake.sock`. Mirrors the broker's bind.
pub fn intake_socket_path() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run"));
    base.join("arlen").join("consent-intake.sock")
}

/// Ask the broker to authorise enabling `module_id` with `caps`.
///
/// Returns `true` when the module may go active. A module declaring nothing
/// needs no decision and is admitted without a round trip. Every other outcome
/// - denial, a broker that cannot be reached, a malformed reply - is a refusal.
///
/// The requester is NOT sent: the broker fills it from the attested peer, so
/// modulesd cannot claim to be something else. There is no `on_behalf_of`
/// either, because unlike the portal, modulesd is not relaying an app's request
/// - it is the one that would hold the capability, and the grant should name it.
pub async fn request_enable_consent(
    socket: &Path,
    module_id: &str,
    caps: &ModuleCapabilities,
) -> bool {
    let described = describe(caps);
    if !described.needs_consent() {
        return true;
    }
    let body = RequestBody {
        class: ConsentClass::CapabilityGrant,
        kind: described.kind,
        // Enabling is a deliberate user gesture in the extensions surface, not
        // something a document or a message asked for.
        triggered_by_external_content: false,
        summary: described.summary(module_id),
        scope: Some(described.grants.join("; ")),
        recipient: None,
        preview: None,
        targets: Vec::new(),
        total: None,
        on_behalf_of: None,
    };
    matches!(
        request(socket, &body).await,
        Ok(IntakeResult::SilentGranted)
            | Ok(IntakeResult::Decided {
                outcome: ConsentOutcome::AllowedOnce | ConsentOutcome::AllowedRemembered,
            })
    )
}

/// One intake round trip. Any transport failure is an error, which the caller
/// reads as a refusal.
async fn request(socket: &Path, body: &RequestBody) -> Result<IntakeResult, String> {
    let mut stream = UnixStream::connect(socket)
        .await
        .map_err(|e| format!("consent broker unreachable: {e}"))?;
    let payload = serde_json::to_vec(body).map_err(|e| format!("encoding request: {e}"))?;
    let len = u32::try_from(payload.len()).map_err(|_| "request too large".to_string())?;
    stream
        .write_all(&len.to_le_bytes())
        .await
        .map_err(|e| format!("writing request: {e}"))?;
    stream
        .write_all(&payload)
        .await
        .map_err(|e| format!("writing request: {e}"))?;

    let mut header = [0u8; 4];
    stream
        .read_exact(&mut header)
        .await
        .map_err(|e| format!("reading reply: {e}"))?;
    // Checked BEFORE allocating, so a hostile or corrupt length cannot make us
    // reserve it.
    let len = u32::from_le_bytes(header) as usize;
    if len > MAX_FRAME {
        return Err(format!("reply frame {len} exceeds {MAX_FRAME}"));
    }
    let mut buf = vec![0u8; len];
    stream
        .read_exact(&mut buf)
        .await
        .map_err(|e| format!("reading reply: {e}"))?;
    serde_json::from_slice(&buf).map_err(|e| format!("decoding reply: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_modules::{ClipboardCapability, GraphCapability, NetworkCapability};

    fn caps() -> ModuleCapabilities {
        ModuleCapabilities::default()
    }

    /// Approving the empty set is not a decision, it is friction.
    #[test]
    fn a_module_that_declares_nothing_needs_no_decision() {
        let d = describe(&caps());
        assert!(!d.needs_consent());
        assert_eq!(d.kind, ActionKind::Ordinary);
    }

    /// `always_requires_confirmation` is what stops the broker resolving
    /// silently, so the reaching-out capabilities must never classify Ordinary.
    #[test]
    fn the_capabilities_that_leave_the_machine_always_confirm() {
        let mut write = caps();
        write.graph = Some(GraphCapability {
            read: Vec::new(),
            write: vec!["md.obsidian.Note".into()],
        });
        assert!(describe(&write).kind.always_requires_confirmation());

        let mut net = caps();
        net.network = Some(NetworkCapability {
            allowed_domains: vec!["api.example.com".into()],
        });
        assert!(describe(&net).kind.always_requires_confirmation());

        let mut clip = caps();
        clip.clipboard = Some(ClipboardCapability {
            read: true,
            write: false,
        });
        assert!(describe(&clip).kind.always_requires_confirmation());
    }

    /// Reading the graph is bounded and observable; it should not carry the
    /// weight of writing it, or every module would prompt identically and the
    /// prompt would stop meaning anything.
    #[test]
    fn reading_the_graph_is_a_decision_but_not_a_severe_one() {
        let mut c = caps();
        c.graph = Some(GraphCapability {
            read: vec!["system.File".into()],
            write: Vec::new(),
        });
        let d = describe(&c);
        assert!(d.needs_consent());
        assert_eq!(d.kind, ActionKind::Ordinary);
    }

    /// A module is enabled as a whole, so a mild capability alongside a severe
    /// one must not dilute it.
    #[test]
    fn the_highest_severity_present_wins_regardless_of_order() {
        let mut c = caps();
        c.notifications = true;
        c.graph = Some(GraphCapability {
            read: vec!["system.File".into()],
            write: vec!["x.Y".into()],
        });
        c.storage = Some(arlen_modules::StorageCapability { quota_mb: 10 });
        assert!(describe(&c).kind.always_requires_confirmation());
    }

    /// The user has to be able to tell what they are approving.
    #[test]
    fn the_summary_names_the_module_and_every_grant() {
        let mut c = caps();
        c.notifications = true;
        c.network = Some(NetworkCapability {
            allowed_domains: vec!["api.example.com".into()],
        });
        let s = describe(&c).summary("com.example.Widget");
        assert!(s.contains("com.example.Widget"), "{s}");
        assert!(s.contains("api.example.com"), "{s}");
        assert!(s.contains("notifications"), "{s}");
    }

    /// A broker that is not there must not admit the module.
    #[tokio::test]
    async fn an_unreachable_broker_refuses_a_capability_bearing_module() {
        let mut c = caps();
        c.graph = Some(GraphCapability {
            read: Vec::new(),
            write: vec!["x.Y".into()],
        });
        let missing = PathBuf::from("/nonexistent/arlen-consent-intake.sock");
        assert!(!request_enable_consent(&missing, "com.example.Widget", &c).await);
    }

    /// ...but a module with nothing to grant is not held hostage by the broker
    /// being down, because there was never a decision to make.
    #[tokio::test]
    async fn an_unreachable_broker_still_admits_a_capability_free_module() {
        let missing = PathBuf::from("/nonexistent/arlen-consent-intake.sock");
        assert!(request_enable_consent(&missing, "com.example.Widget", &caps()).await);
    }
}
