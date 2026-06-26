//! The Signed Profile Package (`.lenv`) format contract (profile-system-plan.md,
//! "Signed Profile Packages (`.lenv`, the BYOD model)").
//!
//! A `.lenv` is an org-signed TOML the user installs themselves: "the
//! organization controls their context, the user controls their device." This
//! crate is the FORMAT layer - the policy schema, parsing + validation, the
//! full-policy summary shown before install (nothing hidden), the declared-expiry
//! revocation check and the publisher key fingerprint for the TOFU prompt. It is
//! deliberately data-only and signature-agnostic: verifying the org signature,
//! pinning the key (TOFU), the enrollment-link import and the `systemd-homed`
//! install lifecycle (PR-R2) build ON this layer; the technically-enforced org
//! limits (the org cannot read the personal profile, cannot wipe the device,
//! cannot prevent uninstall) are properties of the UID boundary, not of this
//! file, so nothing here grants the org any capability - it only declares intent
//! the installer surfaces and the profile enforces.

#![forbid(unsafe_code)]

pub mod tofu;

use serde::Deserialize;

/// A parse or validation failure. Nothing is installed on error; the caller
/// rejects the package.
#[derive(Debug, thiserror::Error)]
pub enum LenvError {
    /// The TOML could not be parsed.
    #[error("invalid .lenv TOML: {0}")]
    Toml(String),
    /// A required field was empty or a value was out of range.
    #[error("invalid .lenv: {0}")]
    Invalid(String),
    /// The verifying key or signature was not the expected length / shape.
    #[error("invalid signing material: {0}")]
    BadKey(String),
    /// The org signature did not verify against the key over the .lenv bytes.
    #[error("signature verification failed")]
    BadSignature,
}

/// Package metadata: who published it, its version, and the user-protecting
/// declarations (expiry, uninstall notification).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Meta {
    /// The publisher (organization) name, shown in the TOFU prompt.
    pub publisher: String,
    /// The package version (org-incremented; the tamper-evident config version).
    pub version: u32,
    /// Optional declared expiry as a Unix timestamp (seconds). The profile's
    /// org context lapses after this; absent means no declared expiry.
    #[serde(default)]
    pub expires_at: Option<i64>,
    /// Whether the org is notified when the user uninstalls. Even when `true`,
    /// uninstall always proceeds - this only declares a notification, never a
    /// veto (a user-controlled-device limit).
    #[serde(default)]
    pub notify_on_uninstall: bool,
}

/// The declared cross-profile transfer stance. The Transfer Daemon (PR-R4)
/// interprets the full directional rule set; the `.lenv` declares the org's
/// default stance for this context.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
pub struct Transfer {
    /// The default transfer policy: `"deny"` (default), `"prompt"` or `"allow"`.
    #[serde(default)]
    pub policy: Option<String>,
}

/// The declared network policy for the profile context.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
pub struct Network {
    /// Require an active VPN for the profile's network use.
    #[serde(default)]
    pub require_vpn: bool,
    /// Hosts/domains explicitly permitted outbound (empty means no allowlist).
    #[serde(default)]
    pub allow_outbound: Vec<String>,
    /// Hosts/domains explicitly blocked outbound.
    #[serde(default)]
    pub block_outbound: Vec<String>,
}

/// The declared app policy.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
pub struct Apps {
    /// Apps the org provisions/requires in the context.
    #[serde(default)]
    pub required: Vec<String>,
    /// Apps the org blocks in the context.
    #[serde(default)]
    pub blocked: Vec<String>,
}

/// The declared knowledge-graph policy for the profile context.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
pub struct Graph {
    /// Whether the AI layer may access the profile's KG in this context.
    #[serde(default)]
    pub ai_access: bool,
    /// Whether KG export is permitted in this context.
    #[serde(default)]
    pub export: bool,
}

/// A bundled `.project` template, written to `~/.projects/<name>/` on first login
/// as a starting config (registered as a `Project` node, never synced back).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ProjectTemplate {
    /// The project directory name under `~/.projects/`.
    pub name: String,
    /// The `.project` TOML content written for it.
    pub content: String,
}

/// The bundled project templates.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
pub struct Projects {
    /// The templates to install on first login.
    #[serde(default)]
    pub template: Vec<ProjectTemplate>,
}

/// A parsed `.lenv` Signed Profile Package.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct LenvPackage {
    /// Package metadata.
    pub meta: Meta,
    /// Cross-profile transfer stance.
    #[serde(default)]
    pub transfer: Transfer,
    /// Network policy.
    #[serde(default)]
    pub network: Network,
    /// App policy.
    #[serde(default)]
    pub apps: Apps,
    /// Knowledge-graph policy.
    #[serde(default)]
    pub graph: Graph,
    /// Bundled project templates.
    #[serde(default)]
    pub projects: Projects,
}

/// Whether a project template name is a safe single path component (it becomes a
/// directory under `~/.projects/`).
fn is_safe_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

impl LenvPackage {
    /// Parse and validate a `.lenv` TOML document. Validates the publisher is
    /// named, the transfer policy (if given) is a known stance, and every project
    /// template name is a safe path component.
    pub fn parse(toml_str: &str) -> Result<Self, LenvError> {
        let pkg: LenvPackage = toml::from_str(toml_str).map_err(|e| LenvError::Toml(e.to_string()))?;
        if pkg.meta.publisher.trim().is_empty() {
            return Err(LenvError::Invalid("meta.publisher must be named".to_string()));
        }
        if let Some(p) = pkg.transfer.policy.as_deref() {
            if !matches!(p, "deny" | "prompt" | "allow") {
                return Err(LenvError::Invalid(format!(
                    "transfer.policy must be deny/prompt/allow, got {p:?}"
                )));
            }
        }
        for t in &pkg.projects.template {
            if !is_safe_name(&t.name) {
                return Err(LenvError::Invalid(format!(
                    "project template name is not a safe path component: {:?}",
                    t.name
                )));
            }
        }
        Ok(pkg)
    }

    /// Whether the package's declared org context has expired as of `now_unix`
    /// (Unix seconds). A package with no declared expiry never expires.
    pub fn is_expired(&self, now_unix: i64) -> bool {
        matches!(self.meta.expires_at, Some(exp) if now_unix >= exp)
    }

    /// The full policy summary shown before install: one human-readable line per
    /// declared limit, so nothing the org enforces is hidden from the user.
    pub fn policy_summary(&self) -> Vec<String> {
        let mut out = Vec::new();
        out.push(format!(
            "Published by {} (version {})",
            self.meta.publisher, self.meta.version
        ));
        if let Some(exp) = self.meta.expires_at {
            out.push(format!("This context expires at {exp} (Unix time)"));
        }
        if self.meta.notify_on_uninstall {
            out.push("The publisher is notified if you uninstall (uninstall still proceeds)".to_string());
        }
        if let Some(p) = self.transfer.policy.as_deref() {
            out.push(format!("Cross-profile transfer: {p}"));
        }
        if self.network.require_vpn {
            out.push("Requires an active VPN".to_string());
        }
        if !self.network.allow_outbound.is_empty() {
            out.push(format!(
                "Network limited to: {}",
                self.network.allow_outbound.join(", ")
            ));
        }
        if !self.network.block_outbound.is_empty() {
            out.push(format!("Network blocks: {}", self.network.block_outbound.join(", ")));
        }
        if !self.apps.required.is_empty() {
            out.push(format!("Provisions apps: {}", self.apps.required.join(", ")));
        }
        if !self.apps.blocked.is_empty() {
            out.push(format!("Blocks apps: {}", self.apps.blocked.join(", ")));
        }
        out.push(format!(
            "AI access to this context's knowledge graph: {}",
            if self.graph.ai_access { "allowed" } else { "denied" }
        ));
        out.push(format!(
            "Knowledge-graph export: {}",
            if self.graph.export { "allowed" } else { "denied" }
        ));
        if !self.projects.template.is_empty() {
            let names: Vec<&str> = self.projects.template.iter().map(|t| t.name.as_str()).collect();
            out.push(format!("Installs starter projects: {}", names.join(", ")));
        }
        out
    }
}

/// Verify the org's detached Ed25519 signature over the raw `.lenv` bytes
/// (the signed content is the whole file, so the policy TOML the user inspects is
/// exactly what was signed - no canonicalization gap). `verifying_key` is the
/// org's 32-byte Ed25519 public key, `signature` its 64-byte signature. The
/// caller obtains the key out of band (the enrollment link) or confirms its
/// [`key_fingerprint`] on first install (TOFU); this only checks the signature.
pub fn verify_signature(
    lenv_bytes: &[u8],
    signature: &[u8],
    verifying_key: &[u8],
) -> Result<(), LenvError> {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    let key_bytes: [u8; 32] = verifying_key
        .try_into()
        .map_err(|_| LenvError::BadKey("verifying key must be 32 bytes".to_string()))?;
    let key = VerifyingKey::from_bytes(&key_bytes)
        .map_err(|e| LenvError::BadKey(format!("invalid Ed25519 key: {e}")))?;
    let sig_bytes: [u8; 64] = signature
        .try_into()
        .map_err(|_| LenvError::BadKey("signature must be 64 bytes".to_string()))?;
    let sig = Signature::from_bytes(&sig_bytes);
    key.verify(lenv_bytes, &sig).map_err(|_| LenvError::BadSignature)
}

/// Whether `verifying_key`'s [`key_fingerprint`] equals `pinned` (the fingerprint
/// confirmed at first install / imported via the enrollment link). The TOFU
/// check on every later update: a key whose fingerprint does not match the pin is
/// a different publisher, never silently accepted.
pub fn fingerprint_matches(verifying_key: &[u8], pinned: &str) -> bool {
    key_fingerprint(verifying_key) == pinned
}

/// The publisher key fingerprint for the TOFU prompt: SHA-256 of the raw public
/// key bytes, lowercase hex in colon-separated byte groups (the form shown to the
/// user to confirm on first install).
pub fn key_fingerprint(public_key: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(public_key);
    digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
        [meta]
        publisher = "Acme Corp"
        version = 3
        expires_at = 2000000000
        notify_on_uninstall = true

        [transfer]
        policy = "prompt"

        [network]
        require_vpn = true
        block_outbound = ["facebook.com"]

        [apps]
        required = ["com.acme.vpn"]
        blocked = ["com.example.game"]

        [graph]
        ai_access = false
        export = false

        [[projects.template]]
        name = "onboarding"
        content = "[project]\nname = \"Onboarding\""
    "#;

    #[test]
    fn parses_a_full_package() {
        let p = LenvPackage::parse(SAMPLE).unwrap();
        assert_eq!(p.meta.publisher, "Acme Corp");
        assert_eq!(p.meta.version, 3);
        assert!(p.network.require_vpn);
        assert_eq!(p.apps.required, vec!["com.acme.vpn"]);
        assert_eq!(p.projects.template.len(), 1);
        assert_eq!(p.projects.template[0].name, "onboarding");
    }

    #[test]
    fn a_minimal_package_uses_defaults() {
        let p = LenvPackage::parse("[meta]\npublisher = \"Org\"\nversion = 1").unwrap();
        assert!(!p.network.require_vpn);
        assert!(p.apps.required.is_empty());
        assert!(!p.graph.ai_access);
        assert!(p.transfer.policy.is_none());
    }

    #[test]
    fn an_empty_publisher_is_rejected() {
        let err = LenvPackage::parse("[meta]\npublisher = \"  \"\nversion = 1").unwrap_err();
        assert!(matches!(err, LenvError::Invalid(_)));
    }

    #[test]
    fn an_unknown_transfer_policy_is_rejected() {
        let toml = "[meta]\npublisher=\"O\"\nversion=1\n[transfer]\npolicy=\"whatever\"";
        assert!(matches!(LenvPackage::parse(toml), Err(LenvError::Invalid(_))));
    }

    #[test]
    fn a_traversing_template_name_is_rejected() {
        let toml = "[meta]\npublisher=\"O\"\nversion=1\n[[projects.template]]\nname=\"../escape\"\ncontent=\"x\"";
        assert!(matches!(LenvPackage::parse(toml), Err(LenvError::Invalid(_))));
    }

    #[test]
    fn expiry_is_checked_against_now() {
        let p = LenvPackage::parse(SAMPLE).unwrap();
        assert!(!p.is_expired(1_000_000_000));
        assert!(p.is_expired(2_000_000_001));
        let no_exp = LenvPackage::parse("[meta]\npublisher=\"O\"\nversion=1").unwrap();
        assert!(!no_exp.is_expired(i64::MAX));
    }

    #[test]
    fn the_summary_hides_nothing_material() {
        let p = LenvPackage::parse(SAMPLE).unwrap();
        let s = p.policy_summary().join("\n");
        assert!(s.contains("Acme Corp"));
        assert!(s.contains("VPN"));
        assert!(s.contains("facebook.com"));
        assert!(s.contains("com.acme.vpn"));
        assert!(s.contains("knowledge graph: denied"));
        assert!(s.contains("onboarding"));
        assert!(s.contains("notified if you uninstall"));
    }

    /// A deterministic test keypair from a fixed seed: (verifying_key_bytes,
    /// sign-fn).
    fn keypair(seed: u8) -> ed25519_dalek::SigningKey {
        ed25519_dalek::SigningKey::from_bytes(&[seed; 32])
    }

    #[test]
    fn a_valid_signature_verifies_and_a_tamper_does_not() {
        use ed25519_dalek::Signer;
        let sk = keypair(1);
        let vk = sk.verifying_key().to_bytes();
        let lenv = SAMPLE.as_bytes();
        let sig = sk.sign(lenv).to_bytes();

        assert!(verify_signature(lenv, &sig, &vk).is_ok());
        // A single flipped byte in the content fails verification.
        let mut tampered = SAMPLE.as_bytes().to_vec();
        tampered[0] ^= 0xff;
        assert!(matches!(
            verify_signature(&tampered, &sig, &vk),
            Err(LenvError::BadSignature)
        ));
    }

    #[test]
    fn a_signature_from_another_key_does_not_verify() {
        use ed25519_dalek::Signer;
        let signer = keypair(1);
        let other_vk = keypair(2).verifying_key().to_bytes();
        let lenv = SAMPLE.as_bytes();
        let sig = signer.sign(lenv).to_bytes();
        assert!(matches!(
            verify_signature(lenv, &sig, &other_vk),
            Err(LenvError::BadSignature)
        ));
    }

    #[test]
    fn malformed_key_or_signature_is_rejected() {
        assert!(matches!(
            verify_signature(b"x", &[0u8; 64], &[0u8; 16]),
            Err(LenvError::BadKey(_))
        ));
        assert!(matches!(
            verify_signature(b"x", &[0u8; 10], &[0u8; 32]),
            Err(LenvError::BadKey(_))
        ));
    }

    #[test]
    fn tofu_fingerprint_match() {
        let vk = keypair(1).verifying_key().to_bytes();
        let pinned = key_fingerprint(&vk);
        assert!(fingerprint_matches(&vk, &pinned));
        let other = keypair(2).verifying_key().to_bytes();
        assert!(!fingerprint_matches(&other, &pinned));
    }

    #[test]
    fn fingerprint_is_stable_colon_grouped_hex() {
        let fp = key_fingerprint(&[0u8; 32]);
        assert_eq!(fp.split(':').count(), 32);
        assert_eq!(fp, key_fingerprint(&[0u8; 32]));
        assert_ne!(fp, key_fingerprint(&[1u8; 32]));
    }

    #[test]
    fn fingerprint_matches_only_the_pinned_key() {
        // The TOFU update check: a key whose fingerprint equals the pin is the
        // same publisher; a different key (or a corrupted pin string) is not, so
        // a publisher swap is never silently accepted.
        let key = [7u8; 32];
        let pin = key_fingerprint(&key);
        assert!(fingerprint_matches(&key, &pin), "the pinned key matches its own pin");
        assert!(!fingerprint_matches(&[8u8; 32], &pin), "a different key does not match");
        assert!(!fingerprint_matches(&key, &pin.to_uppercase()), "the lowercase-hex form is exact");
        assert!(!fingerprint_matches(&key, ""), "an empty pin never matches");
    }

    #[test]
    fn is_safe_name_accepts_components_and_rejects_traversal() {
        assert!(is_safe_name("onboarding"));
        assert!(is_safe_name("proj.v2_final-1"));
        assert!(!is_safe_name(""));
        assert!(!is_safe_name("."));
        assert!(!is_safe_name(".."));
        assert!(!is_safe_name("a/b"));
        assert!(!is_safe_name("has space"));
        assert!(!is_safe_name("tab\t"));
    }

    #[test]
    fn expiry_boundary_is_inclusive() {
        // expires_at = 2_000_000_000; the check is `now >= exp`.
        let p = LenvPackage::parse(SAMPLE).unwrap();
        assert!(!p.is_expired(1_999_999_999));
        assert!(p.is_expired(2_000_000_000));
    }

    #[test]
    fn invalid_toml_is_a_toml_error() {
        assert!(matches!(
            LenvPackage::parse("this = = not valid ["),
            Err(LenvError::Toml(_))
        ));
    }

    #[test]
    fn all_known_transfer_policies_parse() {
        for policy in ["deny", "prompt", "allow"] {
            let toml = format!("[meta]\npublisher=\"O\"\nversion=1\n[transfer]\npolicy=\"{policy}\"");
            let p = LenvPackage::parse(&toml).unwrap();
            assert_eq!(p.transfer.policy.as_deref(), Some(policy));
        }
    }

    #[test]
    fn a_minimal_summary_omits_unset_lines_and_defaults_to_denied() {
        let p = LenvPackage::parse("[meta]\npublisher=\"Org\"\nversion=1").unwrap();
        let joined = p.policy_summary().join("\n");
        assert!(joined.contains("Published by Org (version 1)"));
        assert!(!joined.contains("expires"));
        assert!(!joined.contains("VPN"));
        assert!(!joined.contains("notified if you uninstall"));
        // A policy with no network allowlist and no blocked-apps list must not
        // invent those restriction lines: the summary must never claim a limit
        // that is not in the policy (it is a trust-transparency surface).
        assert!(!joined.contains("Network limited to"));
        assert!(!joined.contains("Blocks apps"));
        assert!(joined.contains("knowledge graph: denied"));
        assert!(joined.contains("export: denied"));
    }
}
