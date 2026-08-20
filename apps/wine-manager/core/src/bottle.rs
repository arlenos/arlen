//! The bottle itself: a Wine prefix plus the grants it was given, turned into a
//! bwrap confinement and the drive letters that agree with it.
//!
//! The two halves have to agree or the design is a claim rather than a boundary.
//! A drive letter is a symlink in `dosdevices`, and a symlink is not authority:
//! if `D:` points at `/home/u/Documents` and bwrap never bound that directory,
//! the program sees a drive that is not there. Point it at a directory bwrap
//! bound read-only while the grant said read-write and the program gets a drive
//! it can list and cannot save into, which surfaces as a mystery rather than as
//! a refusal.
//!
//! So [`unmet_drives`] does not check the struct this module builds. It replays
//! the argv that will actually be handed to bwrap, in order, because ordering is
//! how bwrap resolves overlap: a later `--tmpfs` masks an earlier `--bind`, and a
//! later bind re-exposes a masked path. Reading anything else would be checking
//! my own intention rather than the machine's.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use arlen_confiner::{app_runtime_profile, Confinement, ConfinerError, NetworkPolicy};
use serde::{Deserialize, Serialize};

use crate::{map_drives, Access, Drive, DriveError, PathGrant};

/// What a bottle may reach on the network.
///
/// A separate enum from the confiner's `NetworkPolicy` because this one is
/// persisted with the bottle and has to keep its spelling across versions, while
/// that one is an argv detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Egress {
    /// No network at all. The default a bottle is created with, since most
    /// Windows programs that phone home were never asked to.
    None,
    /// Network up, with the launcher enforcing this host list.
    Hosts(Vec<String>),
    /// Network up with no filter, for a program that was explicitly granted it.
    Unrestricted,
}

impl Egress {
    fn policy(&self) -> NetworkPolicy {
        match self {
            Egress::None => NetworkPolicy::None,
            Egress::Hosts(h) => NetworkPolicy::FilteredHosts(h.clone()),
            Egress::Unrestricted => NetworkPolicy::Unrestricted,
        }
    }
}

/// One capability-scoped Wine prefix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bottle {
    /// Stable id, used for the on-disk directory and as the capability subject.
    pub id: String,
    /// The Wine prefix (`WINEPREFIX`), always writable: Wine rewrites the
    /// registry on every boot, so a read-only prefix is not a bottle, it is a
    /// program that will not start.
    pub prefix_root: PathBuf,
    /// The host directories this bottle was granted, which become drive letters.
    pub grants: Vec<PathGrant>,
    /// What it may reach on the network.
    pub egress: Egress,
}

/// A bottle turned into something runnable.
#[derive(Debug, Clone)]
pub struct BottleRun {
    /// The bwrap spec.
    pub confinement: Confinement,
    /// The drives, in letter order, for writing `dosdevices`.
    pub drives: Vec<Drive>,
}

/// Why a bottle could not be turned into a run.
#[derive(Debug)]
pub enum BottleError {
    /// The grant list could not be mapped to letters.
    Drives(DriveError),
    /// The confiner refused a path.
    Confiner(ConfinerError),
    /// The prefix path was relative.
    PrefixNotAbsolute(PathBuf),
}

impl std::fmt::Display for BottleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BottleError::Drives(e) => write!(f, "{e}"),
            BottleError::Confiner(e) => write!(f, "the bottle could not be confined: {e:?}"),
            BottleError::PrefixNotAbsolute(p) => {
                write!(f, "the prefix {} is not an absolute path", p.display())
            }
        }
    }
}

impl std::error::Error for BottleError {}

/// Build the confinement and the drive map together, from one grant list.
///
/// Together rather than in two calls on purpose: the pair is the invariant, and a
/// caller that could build one without the other would eventually do so.
pub fn bottle_run(
    bottle: &Bottle,
    usr: &Path,
    env: BTreeMap<String, String>,
) -> Result<BottleRun, BottleError> {
    if !bottle.prefix_root.is_absolute() {
        return Err(BottleError::PrefixNotAbsolute(bottle.prefix_root.clone()));
    }
    let drives = map_drives(&bottle.grants).map_err(BottleError::Drives)?;

    // Writable dirs go through the confiner's own app-dir handling; the prefix is
    // always one of them.
    let mut writable: Vec<PathBuf> = vec![bottle.prefix_root.clone()];
    writable.extend(
        drives
            .iter()
            .filter(|d| d.access == Access::ReadWrite)
            .map(|d| d.host.clone()),
    );
    let writable_refs: Vec<&Path> = writable.iter().map(PathBuf::as_path).collect();

    let skeleton = app_runtime_profile(usr, &writable_refs, &[], env, bottle.egress.policy())
        .map_err(BottleError::Confiner)?;

    // Read-only grants are not app dirs (the confiner binds those read-write), so
    // they are added as their own binds when the skeleton is completed.
    let read_only: Vec<arlen_confiner::Bind> = drives
        .iter()
        .filter(|d| d.access == Access::ReadOnly)
        .map(|d| {
            let p = d.host.to_string_lossy().to_string();
            arlen_confiner::Bind::ReadOnly(p.clone(), p)
        })
        .collect();

    Ok(BottleRun {
        confinement: skeleton.complete(read_only, Vec::new()),
        drives,
    })
}

/// What a path is at the end of the argv, once every bind and mask has been
/// applied in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reachable {
    /// Nothing covers it, or the last thing that covered it was a mask.
    No,
    /// Covered by a read-only bind.
    ReadOnly,
    /// Covered by a read-write bind.
    ReadWrite,
}

/// Replay `args` and answer what `path` actually is inside the sandbox.
///
/// Last covering operation wins, which is bwrap's own rule: argv is applied in
/// order against the new root, so a `--tmpfs` over a directory hides a bind made
/// earlier and a bind made later re-exposes what a mask hid.
pub fn reachable(args: &[String], path: &Path) -> Reachable {
    let mut verdict = Reachable::No;
    let mut i = 0;
    while i < args.len() {
        let (dest, effect, width) = match args[i].as_str() {
            "--ro-bind" if i + 2 < args.len() => (&args[i + 2], Reachable::ReadOnly, 3),
            "--bind" if i + 2 < args.len() => (&args[i + 2], Reachable::ReadWrite, 3),
            "--tmpfs" | "--proc" | "--dev" if i + 1 < args.len() => {
                (&args[i + 1], Reachable::No, 2)
            }
            _ => {
                i += 1;
                continue;
            }
        };
        if path.starts_with(Path::new(dest)) {
            verdict = effect;
        }
        i += width;
    }
    verdict
}

/// A drive the confinement does not actually back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnmetDrive {
    /// The drive letter.
    pub letter: char,
    /// Where it points.
    pub host: PathBuf,
    /// What the grant promised.
    pub promised: Access,
    /// What bwrap will actually give.
    pub actual: Reachable,
}

/// Every drive whose letter promises more than the confinement delivers.
///
/// Empty is the invariant. A non-empty result means the bottle would start and
/// then behave in a way nobody could explain from either half on its own.
pub fn unmet_drives(confinement: &Confinement, drives: &[Drive]) -> Vec<UnmetDrive> {
    let args = confinement.bwrap_args();
    drives
        .iter()
        .filter_map(|d| {
            let actual = reachable(&args, &d.host);
            let met = matches!(
                (d.access, actual),
                (Access::ReadOnly, Reachable::ReadOnly)
                    | (Access::ReadOnly, Reachable::ReadWrite)
                    | (Access::ReadWrite, Reachable::ReadWrite)
            );
            (!met).then(|| UnmetDrive {
                letter: d.letter,
                host: d.host.clone(),
                promised: d.access,
                actual,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bottle() -> Bottle {
        Bottle {
            id: "notepadpp".into(),
            prefix_root: PathBuf::from("/home/u/.local/share/arlen/bottles/notepadpp/pfx"),
            grants: vec![
                PathGrant { host: PathBuf::from("/home/u/Projects"), access: Access::ReadWrite },
                // Deliberately outside /usr. An earlier version of this fixture
                // granted /usr/share/fonts read-only, and the invariant test below
                // still passed with the read-only binds removed entirely, because
                // the confiner's own read-only /usr already covered it. The test
                // was measuring the confiner, not this module.
                PathGrant { host: PathBuf::from("/srv/reference"), access: Access::ReadOnly },
            ],
            egress: Egress::None,
        }
    }

    fn run() -> BottleRun {
        bottle_run(&bottle(), Path::new("/usr"), BTreeMap::new()).unwrap()
    }

    #[test]
    fn every_drive_is_backed_by_a_bind() {
        let r = run();
        assert_eq!(r.drives.len(), 2);
        assert_eq!(unmet_drives(&r.confinement, &r.drives), vec![]);
    }

    #[test]
    fn a_drive_pointing_somewhere_unbound_is_reported() {
        // The control. If this passed, the check above would be decoration: it
        // has to fail for a letter that was never granted.
        let r = run();
        let smuggled = Drive {
            letter: 'F',
            host: PathBuf::from("/home/u/.ssh"),
            access: Access::ReadOnly,
        };
        let found = unmet_drives(&r.confinement, &[smuggled]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].actual, Reachable::No);
    }

    #[test]
    fn a_read_only_bind_does_not_satisfy_a_read_write_letter() {
        let r = run();
        // /usr is bound read-only by the confiner, so this asks for a letter that
        // promises writes over a directory that genuinely is read-only.
        let lying = Drive {
            letter: 'F',
            host: PathBuf::from("/usr/share/fonts"),
            access: Access::ReadWrite,
        };
        let found = unmet_drives(&r.confinement, &[lying]);
        assert_eq!(found[0].actual, Reachable::ReadOnly);
        assert_eq!(found[0].promised, Access::ReadWrite);
    }

    #[test]
    fn the_home_and_the_filesystem_are_never_bound() {
        let args = run().confinement.bwrap_args();
        for hole in ["/", "/home/u", "/home"] {
            assert_eq!(
                reachable(&args, Path::new(hole)),
                Reachable::No,
                "{hole} is inside the bottle, which is the thing this whole design exists to stop"
            );
        }
    }

    #[test]
    fn a_bottle_with_no_grants_still_has_its_prefix() {
        let mut b = bottle();
        b.grants.clear();
        let r = bottle_run(&b, Path::new("/usr"), BTreeMap::new()).unwrap();
        assert!(r.drives.is_empty());
        assert_eq!(
            reachable(&r.confinement.bwrap_args(), &b.prefix_root),
            Reachable::ReadWrite,
            "Wine rewrites the registry on boot, so the prefix is writable or nothing starts"
        );
    }

    #[test]
    fn a_later_mask_beats_an_earlier_bind() {
        // bwrap's own resolution rule, and the reason this reads argv in order
        // rather than collecting the binds into a set.
        let args = vec![
            "--bind".into(),
            "/home/u/Projects".into(),
            "/home/u/Projects".into(),
            "--tmpfs".into(),
            "/home/u/Projects/secret".into(),
        ];
        assert_eq!(reachable(&args, Path::new("/home/u/Projects/open")), Reachable::ReadWrite);
        assert_eq!(reachable(&args, Path::new("/home/u/Projects/secret")), Reachable::No);
    }

    #[test]
    fn a_bottle_with_no_egress_unshares_the_network() {
        assert!(run().confinement.bwrap_args().contains(&"--unshare-net".to_string()));
    }

    #[test]
    fn a_relative_prefix_is_refused() {
        let mut b = bottle();
        b.prefix_root = PathBuf::from("pfx");
        assert!(matches!(
            bottle_run(&b, Path::new("/usr"), BTreeMap::new()),
            Err(BottleError::PrefixNotAbsolute(_))
        ));
    }
}
