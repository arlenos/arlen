//! Artifact collection for forage packaging.
//!
//! After the build, only the artifacts a recipe explicitly declares in
//! `[artifacts]` are gathered into the `.lunpkg` staging tree; everything else
//! the build produced is discarded. This is forage's anti-scooping rule
//! (forage-recipes.md sections 5a, 9 step 6): unlike AUR/pkgit heuristic
//! scooping, nothing undeclared ends up in the package, which closes an
//! injection vector.
//!
//! Layout: each declared path is interpreted relative to the build output and
//! placed by category into the package prefix under its basename:
//! `bin/`, `lib/`, `include/`, `libexec/`, `share/`, with `desktop` going to
//! `share/applications/` and `icon` to `share/icons/`. A declared directory is
//! copied recursively under its basename. Paths that are absolute or contain
//! `..` are rejected, as are two artifacts that would collide on one
//! destination.
//!
//! This slice produces the staging tree and the list of collected files; the
//! `manifest.toml` synthesis, the tar.zst archive and the Ed25519 signature
//! that complete a `.lunpkg` are follow-up slices.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use arlen_forage_recipe::Artifacts;
use thiserror::Error;

/// A failure collecting declared artifacts.
#[derive(Debug, Error)]
pub enum PackageError {
    /// A filesystem error.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// A declared artifact does not exist in the build output.
    #[error("declared artifact not found: {0}")]
    NotFound(String),
    /// A declared artifact path is absolute or escapes the build output.
    #[error("artifact path escapes the build tree: {0}")]
    Escapes(String),
    /// A collected entry is a symlink (rejected: it could point outside the
    /// build tree, and collection runs host-side after the sandboxed build).
    #[error("artifact is or contains a symlink, which is not allowed: {0}")]
    Symlink(String),
    /// A declared artifact has no usable file name.
    #[error("artifact path has no file name: {0}")]
    NoName(String),
    /// Two artifacts resolve to the same package destination.
    #[error("artifact destination collision: {0}")]
    Collision(String),
    /// The staging directory already has contents (it must be fresh so no
    /// undeclared file survives into the package).
    #[error("staging directory is not empty: {0}")]
    StagingNotEmpty(String),
    /// The staging directory overlaps the build tree (one contains the other).
    #[error("staging directory must be outside the build tree: {0}")]
    StagingInsideBuild(String),
}

/// The result of collecting artifacts into a staging tree.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Collection {
    /// Every collected path, relative to the staging root, sorted.
    pub files: Vec<String>,
    /// The package-relative paths of collected binaries (the `[artifacts] bin`
    /// entries), for the manifest's `[binary]`/`[provides]` synthesis.
    pub binaries: Vec<String>,
}

/// One validated artifact, ready to copy.
struct Planned {
    src: PathBuf,
    dest_root: PathBuf,
    rel_root: String,
    is_binary: bool,
}

/// Gather the recipe's declared artifacts from `build_dir` into `staging`,
/// returning what was collected. Only declared paths are copied.
///
/// `staging` must be empty (or not yet exist) and must not overlap `build_dir`,
/// so no undeclared or stale file can survive into the package. All artifacts
/// are validated and their destination roots claimed in a preflight pass before
/// any content is written, so a collision aborts before partial staging.
pub fn collect_artifacts(
    build_dir: &Path,
    artifacts: &Artifacts,
    staging: &Path,
) -> Result<Collection, PackageError> {
    // The canonical build root: every collected source must resolve to a real
    // path within it, so a symlink (leaf or an intermediate path component)
    // cannot pull content from outside the build tree into the package.
    let real_root = build_dir.canonicalize()?;
    prepare_staging(staging, &real_root)?;

    // The declared artifacts as (path, category, is_binary).
    let mut declared: Vec<(&Path, &str, bool)> = Vec::new();
    for p in &artifacts.bin {
        declared.push((p, "bin", true));
    }
    for p in &artifacts.lib {
        declared.push((p, "lib", false));
    }
    for p in &artifacts.include {
        declared.push((p, "include", false));
    }
    for p in &artifacts.libexec {
        declared.push((p, "libexec", false));
    }
    for p in &artifacts.share {
        declared.push((p, "share", false));
    }
    if let Some(desktop) = &artifacts.desktop {
        declared.push((desktop, "share/applications", false));
    }
    if let Some(icon) = &artifacts.icon {
        declared.push((icon, "share/icons", false));
    }

    // Preflight: validate every artifact and claim its destination root before
    // any copy, so a collision aborts before partial content is staged.
    let mut roots: BTreeSet<String> = BTreeSet::new();
    let mut planned: Vec<Planned> = Vec::new();
    for (decl, category, is_binary) in declared {
        let decl_str = decl.display().to_string();
        if !is_contained_relative(decl) {
            return Err(PackageError::Escapes(decl_str));
        }
        let name = decl
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|n| !n.is_empty() && *n != "." && *n != "..")
            .ok_or_else(|| PackageError::NoName(decl_str.clone()))?;

        let src = build_dir.join(decl);
        if !src.exists() {
            return Err(PackageError::NotFound(decl_str));
        }
        // A declared leaf that is itself a symlink is rejected: following it
        // (even to an in-tree target) would copy undeclared content and bypass
        // anti-scooping.
        if src.symlink_metadata()?.file_type().is_symlink() {
            return Err(PackageError::Symlink(decl_str));
        }
        // Confirm the real path stays inside the build tree (defends against a
        // symlinked intermediate component).
        let real_src = src.canonicalize()?;
        if !real_src.starts_with(&real_root) {
            return Err(PackageError::Escapes(decl_str));
        }

        let rel_root = format!("{category}/{name}");
        if !roots.insert(rel_root.clone()) {
            return Err(PackageError::Collision(rel_root));
        }
        planned.push(Planned {
            src,
            dest_root: staging.join(category).join(name),
            rel_root,
            is_binary,
        });
    }

    // Copy pass: every destination root was claimed in preflight, so the only
    // remaining claims are per-file inside copied directories.
    let mut ctx = Collector {
        used: BTreeSet::new(),
        collection: Collection::default(),
    };
    for p in planned {
        if let Some(parent) = p.dest_root.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if p.src.is_dir() {
            ctx.copy_dir(&p.src, &p.dest_root, &p.rel_root)?;
        } else {
            ctx.claim(&p.rel_root)?;
            std::fs::copy(&p.src, &p.dest_root)?;
            ctx.collection.files.push(p.rel_root.clone());
        }
        if p.is_binary {
            ctx.collection.binaries.push(p.rel_root);
        }
    }

    ctx.collection.files.sort();
    ctx.collection.binaries.sort();
    Ok(ctx.collection)
}

/// Ensure `staging` is a fresh directory that does not overlap the build tree,
/// so neither a stale file nor a build-created `.lunpkg` can survive into the
/// package.
fn prepare_staging(staging: &Path, real_build: &Path) -> Result<(), PackageError> {
    if staging.exists() {
        if std::fs::read_dir(staging)?.next().is_some() {
            return Err(PackageError::StagingNotEmpty(staging.display().to_string()));
        }
    } else {
        std::fs::create_dir_all(staging)?;
    }
    let real_staging = staging.canonicalize()?;
    if real_staging.starts_with(real_build) || real_build.starts_with(&real_staging) {
        return Err(PackageError::StagingInsideBuild(
            staging.display().to_string(),
        ));
    }
    Ok(())
}

struct Collector {
    used: BTreeSet<String>,
    collection: Collection,
}

impl Collector {
    /// Recursively copy a directory, recording each file under its staging path.
    fn copy_dir(&mut self, src: &Path, dest: &Path, rel: &str) -> Result<(), PackageError> {
        std::fs::create_dir_all(dest)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let child_src = entry.path();
            // Reject symlinks inside a collected directory: following one could
            // copy content from outside the build tree into the package.
            if entry.file_type()?.is_symlink() {
                return Err(PackageError::Symlink(child_src.display().to_string()));
            }
            let child_dest = dest.join(name);
            let child_rel = format!("{rel}/{name}");
            if child_src.is_dir() {
                self.copy_dir(&child_src, &child_dest, &child_rel)?;
            } else {
                self.claim(&child_rel)?;
                std::fs::copy(&child_src, &child_dest)?;
                self.collection.files.push(child_rel);
            }
        }
        Ok(())
    }

    /// Record a destination as used, erroring if another artifact already
    /// claimed it.
    fn claim(&mut self, rel: &str) -> Result<(), PackageError> {
        if !self.used.insert(rel.to_string()) {
            return Err(PackageError::Collision(rel.to_string()));
        }
        Ok(())
    }
}

/// Whether a path is relative and free of `..`/root/prefix components, so it
/// cannot escape the build tree.
fn is_contained_relative(p: &Path) -> bool {
    if p.is_absolute() {
        return false;
    }
    p.components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn artifacts() -> Artifacts {
        Artifacts {
            bin: Vec::new(),
            lib: Vec::new(),
            include: Vec::new(),
            share: Vec::new(),
            libexec: Vec::new(),
            desktop: None,
            icon: None,
        }
    }

    fn build_tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("target/release")).unwrap();
        fs::write(root.join("target/release/app"), b"ELF...").unwrap();
        fs::write(root.join("target/release/libfoo.so"), b"lib").unwrap();
        fs::create_dir_all(root.join("assets/data")).unwrap();
        fs::write(root.join("assets/data/x.txt"), b"x").unwrap();
        fs::write(root.join("assets/app.desktop"), b"[Desktop Entry]").unwrap();
        fs::write(root.join("secret.key"), b"DO NOT PACKAGE").unwrap();
        dir
    }

    #[test]
    fn collects_only_declared_artifacts() {
        let build = build_tree();
        let staging = tempfile::tempdir().unwrap();
        let mut a = artifacts();
        a.bin = vec!["target/release/app".into()];
        a.lib = vec!["target/release/libfoo.so".into()];
        a.desktop = Some("assets/app.desktop".into());

        let c = collect_artifacts(build.path(), &a, staging.path()).unwrap();

        assert!(staging.path().join("bin/app").exists());
        assert!(staging.path().join("lib/libfoo.so").exists());
        assert!(staging.path().join("share/applications/app.desktop").exists());
        // Anti-scooping: the undeclared secret is never copied.
        assert!(!staging.path().join("secret.key").exists());
        assert_eq!(c.binaries, vec!["bin/app"]);
        assert_eq!(
            c.files,
            vec!["bin/app", "lib/libfoo.so", "share/applications/app.desktop"]
        );
    }

    #[test]
    fn directory_artifact_is_copied_recursively() {
        let build = build_tree();
        let staging = tempfile::tempdir().unwrap();
        let mut a = artifacts();
        a.share = vec!["assets/data".into()];
        let c = collect_artifacts(build.path(), &a, staging.path()).unwrap();
        assert!(staging.path().join("share/data/x.txt").exists());
        assert_eq!(c.files, vec!["share/data/x.txt"]);
    }

    #[test]
    fn absolute_and_parent_paths_are_rejected() {
        let build = build_tree();
        let staging = tempfile::tempdir().unwrap();
        for bad in ["/etc/passwd", "../../etc/passwd", "a/../../b"] {
            let mut a = artifacts();
            a.bin = vec![bad.into()];
            assert!(
                matches!(
                    collect_artifacts(build.path(), &a, staging.path()),
                    Err(PackageError::Escapes(_))
                ),
                "`{bad}` must be rejected"
            );
        }
    }

    #[test]
    fn missing_artifact_errors() {
        let build = build_tree();
        let staging = tempfile::tempdir().unwrap();
        let mut a = artifacts();
        a.bin = vec!["target/release/does-not-exist".into()];
        assert!(matches!(
            collect_artifacts(build.path(), &a, staging.path()),
            Err(PackageError::NotFound(_))
        ));
    }

    #[test]
    fn destination_collision_errors() {
        let build = build_tree();
        // Two different sources with the same basename collide in bin/.
        fs::create_dir_all(build.path().join("other")).unwrap();
        fs::write(build.path().join("other/app"), b"different").unwrap();
        let staging = tempfile::tempdir().unwrap();
        let mut a = artifacts();
        a.bin = vec!["target/release/app".into(), "other/app".into()];
        assert!(matches!(
            collect_artifacts(build.path(), &a, staging.path()),
            Err(PackageError::Collision(_))
        ));
    }

    #[test]
    #[cfg(unix)]
    fn symlink_escapes_are_rejected() {
        use std::os::unix::fs::symlink;
        let build = build_tree();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret"), b"OUTSIDE").unwrap();
        let staging = tempfile::tempdir().unwrap();

        // A declared artifact that is a symlink (leaf) is rejected outright.
        symlink(outside.path().join("secret"), build.path().join("link")).unwrap();
        let mut a = artifacts();
        a.bin = vec!["link".into()];
        assert!(
            matches!(
                collect_artifacts(build.path(), &a, staging.path()),
                Err(PackageError::Symlink(_))
            ),
            "a symlinked artifact must be rejected"
        );

        // A symlink inside a declared directory.
        fs::create_dir_all(build.path().join("pkgshare")).unwrap();
        symlink(outside.path().join("secret"), build.path().join("pkgshare/leak")).unwrap();
        let mut a2 = artifacts();
        a2.share = vec!["pkgshare".into()];
        assert!(
            matches!(
                collect_artifacts(build.path(), &a2, staging.path()),
                Err(PackageError::Symlink(_))
            ),
            "a symlink inside a collected directory must be rejected"
        );
        assert!(!staging.path().join("share/pkgshare/leak").exists());
    }

    #[test]
    #[cfg(unix)]
    fn in_tree_symlink_to_dir_cannot_scoop_undeclared() {
        use std::os::unix::fs::symlink;
        let build = build_tree();
        // An in-tree symlink `bundle -> assets/data` (whose contents are NOT
        // declared). Following it would scoop assets/data without declaration.
        symlink(build.path().join("assets/data"), build.path().join("bundle")).unwrap();
        let staging = tempfile::tempdir().unwrap();
        let mut a = artifacts();
        a.share = vec!["bundle".into()];
        assert!(
            matches!(
                collect_artifacts(build.path(), &a, staging.path()),
                Err(PackageError::Symlink(_))
            ),
            "an in-tree symlinked directory artifact must be rejected (anti-scooping)"
        );
        assert!(!staging.path().join("share/bundle").exists());
    }

    #[test]
    fn non_empty_staging_is_rejected() {
        let build = build_tree();
        let staging = tempfile::tempdir().unwrap();
        fs::write(staging.path().join("stale"), b"left over").unwrap();
        let mut a = artifacts();
        a.bin = vec!["target/release/app".into()];
        assert!(matches!(
            collect_artifacts(build.path(), &a, staging.path()),
            Err(PackageError::StagingNotEmpty(_))
        ));
    }

    #[test]
    fn staging_inside_build_is_rejected() {
        let build = build_tree();
        let staging = build.path().join(".lunpkg-staging");
        let mut a = artifacts();
        a.bin = vec!["target/release/app".into()];
        assert!(matches!(
            collect_artifacts(build.path(), &a, &staging),
            Err(PackageError::StagingInsideBuild(_))
        ));
    }

    #[test]
    fn empty_artifacts_collects_nothing() {
        let build = build_tree();
        let staging = tempfile::tempdir().unwrap();
        let c = collect_artifacts(build.path(), &artifacts(), staging.path()).unwrap();
        assert!(c.files.is_empty());
        assert!(c.binaries.is_empty());
    }
}
