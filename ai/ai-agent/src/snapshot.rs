//! The snapshot inverse seam and its filesystem-capability gating
//! (reversible-receipts-and-the-effect-model.md §9, §14.5).
//!
//! `RestoreSnapshot` is the only crash-exact inverse, but it assumes the target
//! subtree lives on a snapshot-capable filesystem (Btrfs/ZFS). On ext4, xfs, or
//! tmpfs there is no per-action snapshot, so a `Reversible { RestoreSnapshot }`
//! claim whose target lives on a non-capable filesystem is **downgraded to
//! `Irreversible` at predict time** ([`resolve_snapshot_class`]), never lifted to
//! discover at execute time that it has no inverse.
//!
//! This module is the pure core: the [`SnapshotSeam`] trait the executor will
//! call, the filesystem-capability classifier, and the predict-time downgrade.
//! The real agent-callable Snapper/Btrfs `take`/`rollback` implementation is a
//! later increment that needs an on-kernel snapshot-capable filesystem to verify
//! against (gated like the bwrap confiner); a [`DeniedSnapshotSeam`] stands in
//! until then so no path silently believes it snapshotted.

use crate::effect_model::{CanonicalPath, CaptureShape, InverseClass, IrreversibilityReason, SnapshotRef};

/// An error from the snapshot seam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotError {
    /// The target's backing filesystem cannot snapshot; no inverse is available.
    NotSnapshotCapable(String),
    /// The underlying snapshot tool (Snapper/Btrfs) failed.
    Backend(String),
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SnapshotError::NotSnapshotCapable(p) => {
                write!(f, "target {p} is not on a snapshot-capable filesystem")
            }
            SnapshotError::Backend(e) => write!(f, "snapshot backend error: {e}"),
        }
    }
}

impl std::error::Error for SnapshotError {}

/// The agent-callable snapshot primitive (§9): take a pre-action snapshot of a
/// scoped subtree, and roll the subtree back to it. The snapshot handle is the
/// crash-exact commit witness the counter-op inverses lack. The real Snapper/Btrfs
/// implementation is gated on an on-kernel snapshot-capable filesystem; this trait
/// is the seam the executor depends on.
pub trait SnapshotSeam: Send + Sync {
    /// Take a pre-action snapshot of `scope`, returning its handle.
    fn take(&self, scope: &CanonicalPath) -> Result<SnapshotRef, SnapshotError>;

    /// Roll `scope` back to a previously-taken `snapshot`.
    fn rollback(&self, snapshot: &SnapshotRef, scope: &CanonicalPath) -> Result<(), SnapshotError>;
}

/// A seam that takes no snapshot and fails closed, the stand-in until the real
/// Snapper/Btrfs primitive is verified on-kernel. Used where the executor must
/// hold *some* seam but no snapshot path is live, so a `RestoreSnapshot` capture
/// errors visibly rather than silently believing it snapshotted.
#[derive(Debug, Default, Clone, Copy)]
pub struct DeniedSnapshotSeam;

impl SnapshotSeam for DeniedSnapshotSeam {
    fn take(&self, scope: &CanonicalPath) -> Result<SnapshotRef, SnapshotError> {
        Err(SnapshotError::Backend(format!(
            "no snapshot backend wired for {}",
            scope.as_str()
        )))
    }

    fn rollback(&self, _snapshot: &SnapshotRef, scope: &CanonicalPath) -> Result<(), SnapshotError> {
        Err(SnapshotError::Backend(format!(
            "no snapshot backend wired for {}",
            scope.as_str()
        )))
    }
}

/// Whether a filesystem named in `/proc/mounts` (field 3) supports the
/// per-action snapshot the `RestoreSnapshot` inverse needs. Conservative: only
/// the filesystems with a real agent-reachable snapshot facility return true;
/// everything else (ext4, xfs, tmpfs, overlay, vfat, network filesystems, the
/// unknown) returns false, so an unrecognised filesystem fails closed to
/// "irreversible, always-confirm".
pub fn fs_type_is_snapshot_capable(fs_type: &str) -> bool {
    matches!(fs_type, "btrfs" | "zfs")
}

/// A parsed view of the mount table: each `(mount_point, fs_type)` pair, used to
/// find the filesystem backing a given path by longest-prefix match.
#[derive(Debug, Clone, Default)]
pub struct MountTable {
    /// `(mount_point, fs_type)`, mount points absolute and unescaped.
    entries: Vec<(String, String)>,
}

impl MountTable {
    /// Read and parse the live mount table from `/proc/mounts`.
    pub fn from_proc_mounts() -> std::io::Result<MountTable> {
        let text = std::fs::read_to_string("/proc/mounts")?;
        Ok(MountTable::parse(&text))
    }

    /// Parse `/proc/mounts` content: device, mount point, fs type, then options.
    /// Mount fields use octal escapes for space/tab/newline/backslash, decoded
    /// here so a mount point with a space matches correctly.
    pub fn parse(text: &str) -> MountTable {
        let mut entries = Vec::new();
        for line in text.lines() {
            let mut fields = line.split_whitespace();
            let _device = fields.next();
            let Some(mount_point) = fields.next() else {
                continue;
            };
            let Some(fs_type) = fields.next() else {
                continue;
            };
            let mount_point = unescape_mount_field(mount_point);
            if mount_point.starts_with('/') {
                entries.push((mount_point, fs_type.to_string()));
            }
        }
        MountTable { entries }
    }

    /// The filesystem type backing `path`: the fs type of the mount point that is
    /// the longest path-component prefix of `path`. `None` if no mount point
    /// covers it (which should not happen for an absolute path on a live system
    /// with `/` mounted, but is treated as not-capable by the caller).
    pub fn backing_fs_type(&self, path: &str) -> Option<&str> {
        let mut best: Option<(&str, &str)> = None;
        for (mount_point, fs_type) in &self.entries {
            if path_has_prefix(path, mount_point) {
                let better = match best {
                    Some((bp, _)) => mount_point.len() > bp.len(),
                    None => true,
                };
                if better {
                    best = Some((mount_point.as_str(), fs_type.as_str()));
                }
            }
        }
        best.map(|(_, fs)| fs)
    }

    /// Whether the filesystem backing `path` is snapshot-capable. A path with no
    /// covering mount, or one backed by a non-capable filesystem, is not capable
    /// (fail closed to irreversible).
    pub fn is_path_snapshot_capable(&self, path: &CanonicalPath) -> bool {
        self.backing_fs_type(path.as_str())
            .map(fs_type_is_snapshot_capable)
            .unwrap_or(false)
    }
}

/// Whether `mount_point` is a path-component prefix of `path` (so `/home` covers
/// `/home/tim` but not `/homely`). The root `/` covers every absolute path.
fn path_has_prefix(path: &str, mount_point: &str) -> bool {
    if mount_point == "/" {
        return path.starts_with('/');
    }
    if let Some(rest) = path.strip_prefix(mount_point) {
        rest.is_empty() || rest.starts_with('/')
    } else {
        false
    }
}

/// Decode the octal escapes `/proc/mounts` uses for space, tab, newline, and
/// backslash. Backslash is decoded last so its replacement cannot reintroduce an
/// escape sequence.
fn unescape_mount_field(s: &str) -> String {
    s.replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
        .replace("\\134", "\\")
}

/// The predict-time downgrade (§9, §14.5): if `class` captures a
/// `RestoreSnapshot` inverse and the target is NOT on a snapshot-capable
/// filesystem, resolve it to `Irreversible` (so it never lifts and always
/// confirms) rather than lifting and finding no inverse at execute time. Any
/// class that does not depend on a snapshot, and any snapshot class on a capable
/// target, is returned unchanged.
pub fn resolve_snapshot_class(class: InverseClass, target_snapshot_capable: bool) -> InverseClass {
    let needs_snapshot = class.capture_shape() == Some(CaptureShape::RestoreSnapshot);
    if needs_snapshot && !target_snapshot_capable {
        InverseClass::Irreversible {
            reason: IrreversibilityReason::NoSnapshotCapableFilesystem,
        }
    } else {
        class
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect_model::ResidualCost;

    #[test]
    fn only_btrfs_and_zfs_are_snapshot_capable() {
        assert!(fs_type_is_snapshot_capable("btrfs"));
        assert!(fs_type_is_snapshot_capable("zfs"));
        for fs in ["ext4", "xfs", "tmpfs", "overlay", "vfat", "nfs", "unknown-fs"] {
            assert!(!fs_type_is_snapshot_capable(fs), "{fs} must not be capable");
        }
    }

    fn sample_mounts() -> MountTable {
        MountTable::parse(
            "rootfs / btrfs rw,relatime 0 0\n\
             tmpfs /tmp tmpfs rw,nosuid 0 0\n\
             /dev/sda2 /home ext4 rw,relatime 0 0\n\
             /dev/sda3 /home/tim/snap btrfs rw 0 0\n\
             proc /proc proc rw 0 0\n",
        )
    }

    #[test]
    fn backing_fs_is_the_longest_prefix_mount() {
        let m = sample_mounts();
        assert_eq!(m.backing_fs_type("/home/tim/notes.md"), Some("ext4"));
        assert_eq!(m.backing_fs_type("/home/tim/snap/data"), Some("btrfs"), "longer mount wins");
        assert_eq!(m.backing_fs_type("/tmp/scratch"), Some("tmpfs"));
        assert_eq!(m.backing_fs_type("/var/x"), Some("btrfs"), "falls back to root mount");
    }

    #[test]
    fn prefix_match_respects_path_components() {
        let m = MountTable::parse("/dev/x /home ext4 rw 0 0\nrootfs / btrfs rw 0 0\n");
        // "/homely" must NOT match the "/home" mount; it falls back to "/".
        assert_eq!(m.backing_fs_type("/homely/x"), Some("btrfs"));
        assert_eq!(m.backing_fs_type("/home"), Some("ext4"), "the mount point itself");
        assert_eq!(m.backing_fs_type("/home/tim"), Some("ext4"));
    }

    #[test]
    fn path_capability_reads_the_backing_fs() {
        let m = sample_mounts();
        assert!(m.is_path_snapshot_capable(&CanonicalPath::new("/home/tim/snap/x").unwrap()));
        assert!(!m.is_path_snapshot_capable(&CanonicalPath::new("/home/tim/notes.md").unwrap()));
        assert!(!m.is_path_snapshot_capable(&CanonicalPath::new("/tmp/x").unwrap()));
    }

    #[test]
    fn an_uncovered_path_is_not_capable() {
        // A mount table with no root and no covering mount: fail closed.
        let m = MountTable::parse("/dev/x /mnt/data btrfs rw 0 0\n");
        assert_eq!(m.backing_fs_type("/home/tim/x"), None);
        assert!(!m.is_path_snapshot_capable(&CanonicalPath::new("/home/tim/x").unwrap()));
    }

    #[test]
    fn snapshot_class_downgrades_on_a_non_capable_target() {
        let class = InverseClass::Reversible { capture: CaptureShape::RestoreSnapshot };
        // Capable target: unchanged, still liftable.
        assert!(resolve_snapshot_class(class, true).is_reversible());
        // Non-capable target: downgraded to irreversible, never lifts.
        let downgraded = resolve_snapshot_class(class, false);
        assert!(!downgraded.is_reversible());
        assert_eq!(
            downgraded,
            InverseClass::Irreversible {
                reason: IrreversibilityReason::NoSnapshotCapableFilesystem,
            }
        );
    }

    #[test]
    fn non_snapshot_classes_are_unaffected_by_capability() {
        // A RestorePath inverse does not depend on the snapshot facility.
        let path_class = InverseClass::Reversible { capture: CaptureShape::RestorePath };
        assert_eq!(resolve_snapshot_class(path_class, false), path_class);
        // ReversibleWithCost on a non-snapshot capture stays as declared.
        let cost_class = InverseClass::ReversibleWithCost {
            capture: CaptureShape::RestoreValue,
            cost: ResidualCost::Fee,
        };
        assert_eq!(resolve_snapshot_class(cost_class, false), cost_class);
        // Already-irreversible stays irreversible.
        let irr = InverseClass::Irreversible { reason: IrreversibilityReason::PermanentDelete };
        assert_eq!(resolve_snapshot_class(irr, false), irr);
    }

    #[test]
    fn a_snapshot_class_with_cost_also_downgrades() {
        // A ReversibleWithCost { RestoreSnapshot } on a non-capable target has no
        // inverse either, so it too becomes irreversible.
        let class = InverseClass::ReversibleWithCost {
            capture: CaptureShape::RestoreSnapshot,
            cost: ResidualCost::Redownload,
        };
        assert_eq!(
            resolve_snapshot_class(class, false),
            InverseClass::Irreversible {
                reason: IrreversibilityReason::NoSnapshotCapableFilesystem,
            }
        );
    }

    #[test]
    fn the_denied_seam_fails_closed() {
        let seam = DeniedSnapshotSeam;
        let scope = CanonicalPath::new("/home/tim/project").unwrap();
        assert!(seam.take(&scope).is_err());
        let snap = SnapshotRef::new("never").unwrap();
        assert!(seam.rollback(&snap, &scope).is_err());
    }
}
