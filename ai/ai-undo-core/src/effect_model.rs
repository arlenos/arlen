//! The non-graph effect model's receipt + inverse vocabulary
//! (reversible-receipts-and-the-effect-model.md §5).
//!
//! One inverse vocabulary, a closed enum total over the actions the executor can
//! commit, so irreversibility is the *absence* of a variant, never a stored
//! "irreversible inverse". The executor captures a valid [`InverseReceipt`]
//! write-ahead at commit; `is_reversible` for a non-graph rule is exactly "the
//! executor captured a valid inverse", capture-failure being irreversibility.
//!
//! This module is the pure vocabulary. The receipt wrapper (`ActionReceipt` /
//! `ActionWrite`, the opacity-disciplined twin of the graph `ExecutedWrite`), the
//! durable undo-log that stores these, and the executor arms that capture them
//! are separate increments built on this.

use serde::{Deserialize, Serialize};

/// A canonical, absolute filesystem path: the type-level guarantee that a path
/// is absolute and free of `.`/`..`/empty components. Symlink resolution is the
/// resolver's job at capture time (slice.rs `PathResolver`); this type carries
/// the syntactic canonical invariant the inverse vocabulary relies on, so an
/// inverse can never name a relative or traversal path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct CanonicalPath(String);

// A persisted CanonicalPath must re-pass the same shape validation on load, so a
// tampered or corrupt undo-log record can never carry a relative or traversal
// path into an inverse. Deserialize routes through `new`, the one constructor.
impl<'de> Deserialize<'de> for CanonicalPath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        CanonicalPath::new(&s)
            .ok_or_else(|| serde::de::Error::custom(format!("{s:?} is not a canonical absolute path")))
    }
}

impl CanonicalPath {
    /// Construct from an already-canonical absolute path, fail-closed on a
    /// relative path, an empty path, or any `.`/`..`/empty (`//`) component. The
    /// path is taken verbatim when valid (it must already be canonical; this does
    /// not resolve symlinks or the filesystem, it enforces the shape).
    pub fn new(path: &str) -> Option<CanonicalPath> {
        if !path.starts_with('/') {
            return None;
        }
        // Skip the empty segment before the leading `/`; every real component
        // must be non-empty and not a `.`/`..` traversal.
        let mut components = path.split('/');
        components.next();
        // A bare "/" has no addressable component.
        components.clone().next()?;
        for comp in components {
            if comp.is_empty() || comp == "." || comp == ".." {
                return None;
            }
        }
        Some(CanonicalPath(path.to_string()))
    }

    /// The canonical path string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A scalar setting target: a config key in a named file under the user's Arlen
/// config (`~/.config/arlen`). The executor validates the file's location at
/// capture; this type pairs the file with the dotted key whose prior value the
/// inverse restores.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingTarget {
    file: String,
    key: String,
}

impl SettingTarget {
    /// Construct a setting target from a non-empty config file name and key.
    pub fn new(file: &str, key: &str) -> Option<SettingTarget> {
        if file.is_empty() || key.is_empty() {
            return None;
        }
        Some(SettingTarget {
            file: file.to_string(),
            key: key.to_string(),
        })
    }

    /// The config file name.
    pub fn file(&self) -> &str {
        &self.file
    }

    /// The dotted key.
    pub fn key(&self) -> &str {
        &self.key
    }
}

/// The identity of an entity an action created: its canonical path plus a
/// commit-time fingerprint, so undo (`DeleteCreated`) deletes exactly what the
/// action created and never a later replacement that reused the path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatedIdentity {
    path: CanonicalPath,
    fingerprint: String,
}

impl CreatedIdentity {
    /// Construct from the created entity's canonical path and a non-empty
    /// commit-time fingerprint.
    pub fn new(path: CanonicalPath, fingerprint: &str) -> Option<CreatedIdentity> {
        if fingerprint.is_empty() {
            return None;
        }
        Some(CreatedIdentity {
            path,
            fingerprint: fingerprint.to_string(),
        })
    }

    /// The created entity's canonical path.
    pub fn path(&self) -> &CanonicalPath {
        &self.path
    }

    /// The commit-time fingerprint.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}

/// An opaque handle to a filesystem snapshot (Snapper/Btrfs), the only
/// crash-exact inverse witness (§9); gated on a snapshot-capable filesystem at
/// capture time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotRef(String);

impl SnapshotRef {
    /// Construct from a non-empty snapshot handle.
    pub fn new(handle: &str) -> Option<SnapshotRef> {
        if handle.is_empty() {
            return None;
        }
        Some(SnapshotRef(handle.to_string()))
    }

    /// The snapshot handle.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The captured inverse of a committed non-graph action (§5): a closed enum
/// total over the actions the executor can commit. The executor captures one of
/// these write-ahead; replaying it is the undo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InverseReceipt {
    /// Relocation: undo moves `now` back to `prior`. Captured: the prior path.
    RestorePath {
        /// Where the action moved the entity to.
        now: CanonicalPath,
        /// Where it was before, where undo restores it.
        prior: CanonicalPath,
    },
    /// Scalar setting: undo sets `target` back to `prior` (`None` unsets a key
    /// that was absent before the action).
    RestoreValue {
        /// The setting the action changed.
        target: SettingTarget,
        /// Its value before the action, or `None` if it was unset.
        prior: Option<String>,
    },
    /// Creation: undo deletes exactly the entity this action created,
    /// identity-bound so undo never deletes a replacement.
    DeleteCreated {
        /// The created entity's identity (canonical path + fingerprint).
        created: CreatedIdentity,
    },
    /// Bulk or irregular: undo restores a pre-action snapshot (§9). The only
    /// crash-exact inverse; gated on a snapshot-capable filesystem.
    RestoreSnapshot {
        /// The pre-action snapshot.
        snapshot: SnapshotRef,
        /// The path scope the snapshot covers.
        scope: CanonicalPath,
    },
}

/// The domain of a non-graph effect (reversible-receipts-and-the-effect-model.md
/// §3.1). Closed: it selects the writer seam and the inverse-capture seam, and a
/// new domain extends this enum rather than adding an `Effect` variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectDomain {
    /// A filesystem operation (move, trash, create).
    Filesystem,
    /// A scalar setting write under the user's Arlen config.
    Setting,
    /// An opaque external action (a write-MCP tool, a send).
    External,
}

/// The shape of inverse an effect's capture produces, mirroring the
/// [`InverseReceipt`] variant the executor will capture at commit. A small closed
/// enum so the gate and the content-free activity view label it without leaking
/// operands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaptureShape {
    /// A prior path is captured ([`InverseReceipt::RestorePath`]).
    RestorePath,
    /// A prior setting value is captured ([`InverseReceipt::RestoreValue`]).
    RestoreValue,
    /// The created entity's identity is captured ([`InverseReceipt::DeleteCreated`]).
    DeleteCreated,
    /// A pre-action snapshot is captured ([`InverseReceipt::RestoreSnapshot`]).
    RestoreSnapshot,
}

/// What the undo of a `ReversibleWithCost` effect spends that the user owns
/// (§3.2). The named examples from the design; closed and extensible. Arlen does
/// not auto-execute this class: the cost is the user's to accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResidualCost {
    /// Undo costs a refund or charge.
    Fee,
    /// Undo requires re-fetching data (a re-download).
    Redownload,
}

/// Why an effect has no capturable inverse (§3.2). The named examples; closed and
/// extensible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IrreversibilityReason {
    /// A permanent delete with no recoverable prior state.
    PermanentDelete,
    /// A send to an external party that cannot be recalled.
    ExternalSend,
    /// An opaque command whose effect cannot be inverted.
    OpaqueCommand,
    /// A snapshot-inverse action whose target is on a filesystem that cannot
    /// snapshot (ext4/xfs/tmpfs). The predict-time downgrade (§9, §14.5): a
    /// `Reversible { RestoreSnapshot }` claim resolves to irreversible here
    /// rather than lifting and discovering at execute time that it has no
    /// inverse.
    NoSnapshotCapableFilesystem,
}

/// The static, predict-time reversibility class an effect declares (§3.2): the
/// ONE source of truth for "may the gate lift this, and how must it be reported".
/// Two consumers read this same field (the lift bit and the audit-honest kind),
/// which is not a synchronisation hazard because it is one field read twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InverseClass {
    /// A captured inverse fully undoes it at no residual cost. The only class
    /// eligible for the autonomous lift.
    Reversible {
        /// The inverse shape the executor captures.
        capture: CaptureShape,
    },
    /// Undoable, but the undo spends something the user owns. Always confirms,
    /// with the cost surfaced.
    ReversibleWithCost {
        /// The inverse shape the executor captures.
        capture: CaptureShape,
        /// What the undo spends.
        cost: ResidualCost,
    },
    /// No inverse can be captured. Always confirms, never autonomous.
    Irreversible {
        /// Why it cannot be inverted.
        reason: IrreversibilityReason,
    },
}

impl InverseClass {
    /// The lift bit (§3.2): only a `Reversible` effect is eligible for the
    /// autonomous lift. `ReversibleWithCost` and `Irreversible` always confirm.
    /// This is the single reversibility source `is_reversible` derives from.
    pub fn is_reversible(&self) -> bool {
        matches!(self, InverseClass::Reversible { .. })
    }

    /// The inverse shape this class captures, or `None` for `Irreversible` (which
    /// captures nothing). Lets a consumer (the snapshot downgrade §9) inspect the
    /// declared capture without re-matching every variant.
    pub fn capture_shape(&self) -> Option<CaptureShape> {
        match self {
            InverseClass::Reversible { capture }
            | InverseClass::ReversibleWithCost { capture, .. } => Some(*capture),
            InverseClass::Irreversible { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_path_accepts_a_canonical_absolute_path() {
        assert_eq!(
            CanonicalPath::new("/home/tim/notes.md").unwrap().as_str(),
            "/home/tim/notes.md"
        );
    }

    #[test]
    fn canonical_path_rejects_relative_traversal_and_degenerate_forms() {
        assert!(CanonicalPath::new("home/tim").is_none(), "relative");
        assert!(CanonicalPath::new("").is_none(), "empty");
        assert!(CanonicalPath::new("/").is_none(), "bare root has no component");
        assert!(CanonicalPath::new("/home/../etc/passwd").is_none(), "parent traversal");
        assert!(CanonicalPath::new("/home/./tim").is_none(), "current-dir component");
        assert!(CanonicalPath::new("/home//tim").is_none(), "empty component");
        assert!(CanonicalPath::new("/home/tim/").is_none(), "trailing slash");
    }

    #[test]
    fn setting_target_requires_a_file_and_key() {
        assert!(SettingTarget::new("shell.toml", "layout.mode").is_some());
        assert!(SettingTarget::new("", "k").is_none());
        assert!(SettingTarget::new("f", "").is_none());
    }

    #[test]
    fn created_identity_requires_a_fingerprint() {
        let p = CanonicalPath::new("/home/tim/new.txt").unwrap();
        assert!(CreatedIdentity::new(p.clone(), "sha256:abc").is_some());
        assert!(CreatedIdentity::new(p, "").is_none());
    }

    #[test]
    fn snapshot_ref_requires_a_handle() {
        assert!(SnapshotRef::new("snapper:42").is_some());
        assert!(SnapshotRef::new("").is_none());
    }

    #[test]
    fn only_reversible_is_lift_eligible() {
        assert!(InverseClass::Reversible { capture: CaptureShape::RestorePath }.is_reversible());
        assert!(!InverseClass::ReversibleWithCost {
            capture: CaptureShape::RestoreValue,
            cost: ResidualCost::Fee,
        }
        .is_reversible());
        assert!(!InverseClass::Irreversible {
            reason: IrreversibilityReason::PermanentDelete,
        }
        .is_reversible());
    }

    #[test]
    fn canonical_path_validates_on_deserialize() {
        // A valid path round-trips as a bare string (transparent).
        let p = CanonicalPath::new("/home/tim/x").unwrap();
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(json, "\"/home/tim/x\"");
        assert_eq!(serde_json::from_str::<CanonicalPath>(&json).unwrap(), p);
        // A tampered or corrupt record carrying a traversal / relative path is
        // rejected on deserialize, so the shape invariant survives persistence,
        // not only construction.
        assert!(serde_json::from_str::<CanonicalPath>("\"/home/../etc\"").is_err());
        assert!(serde_json::from_str::<CanonicalPath>("\"relative/x\"").is_err());
        assert!(serde_json::from_str::<CanonicalPath>("\"/\"").is_err());
    }

    #[test]
    fn inverse_receipt_round_trips_through_json() {
        let inv = InverseReceipt::RestoreValue {
            target: SettingTarget::new("shell.toml", "layout.mode").unwrap(),
            prior: Some("tiling".to_string()),
        };
        let json = serde_json::to_string(&inv).unwrap();
        assert_eq!(serde_json::from_str::<InverseReceipt>(&json).unwrap(), inv);
    }

    #[test]
    fn inverse_receipt_carries_the_captured_prior_state() {
        let now = CanonicalPath::new("/b/x").unwrap();
        let prior = CanonicalPath::new("/a/x").unwrap();
        let inv = InverseReceipt::RestorePath { now, prior: prior.clone() };
        match inv {
            InverseReceipt::RestorePath { prior: p, .. } => assert_eq!(p, prior),
            _ => panic!("wrong variant"),
        }
    }
}
