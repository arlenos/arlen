//! The forage build pipeline: compose the per-phase crates into a single
//! `recipe -> .lunpkg` flow (forage-recipes.md section 9).
//!
//! [`build_recipe`] runs the phases in order:
//! 1. **Fetch** the (primary) source into the content-addressed store
//!    (`arlen-forage-fetch`), verified against its pin.
//! 2. **Extract** the stored archive into a build directory, defended against
//!    traversal/symlink/bomb (`arlen-forage-extract`).
//! 3. **Build** by planning the recipe's `[build]` and running it through a
//!    [`StepRunner`] (`arlen-forage-build`). The runner is a **seam**: the
//!    production runner wraps the steps in the build sandbox (no net, ro source,
//!    no privilege). Until that sandbox lands (roadmap decision D1) a caller
//!    must not run an untrusted recipe through an unsandboxed runner.
//! 4. **Collect** only the declared `[artifacts]` into a staging tree
//!    (`arlen-forage-package`, anti-scooping).
//! 5. **Sign and package** the staging tree into a `.lunpkg` whose signature
//!    verifies under installd.
//!
//! Network is confined to phase 1; everything after operates on the verified,
//! stored source. The actual sandboxed build runner and the installd install
//! step are the remaining seams/gates (roadmap D1/D2). Multi-source recipes
//! (vendored deps, patches) are a follow-up; this builds the primary source.

use std::path::{Path, PathBuf};

use arlen_forage_build::{execute_plan, plan_build, BuildContext, BuildError, StepRunner};
use arlen_forage_extract::{extract_tar, ExtractError, ExtractLimits};
use arlen_forage_patch::{apply_patches, PatchError, PatchLimits};
use arlen_forage_fetch::{fetch_source, Downloader, FetchError, GitFetcher, ReleaseResolver};
use arlen_forage_package::{
    collect_artifacts, find_upstream_metainfo, synthesize_manifest, write_lunpkg, write_metainfo,
    Collection, ManifestError, PackageError, WriteError,
};
use arlen_forage_recipe::{Recipe, Source, SourceType, CATALOG_ORIGIN};
use arlen_forage_store::{ContentHash, Store, StoreError};
use ed25519_dalek::SigningKey;
use thiserror::Error;

/// Resource bounds for the pipeline's fetch and extract phases.
#[derive(Debug, Clone)]
pub struct PipelineLimits {
    /// Cap on a fetched source artifact.
    pub fetch_max_bytes: u64,
    /// Bounds on extracting the source archive.
    pub extract: ExtractLimits,
    /// Bounds on applying source patches.
    pub patch: PatchLimits,
}

impl Default for PipelineLimits {
    fn default() -> Self {
        PipelineLimits {
            fetch_max_bytes: arlen_forage_fetch::DEFAULT_MAX_BYTES,
            extract: ExtractLimits::default(),
            patch: PatchLimits::default(),
        }
    }
}

/// A failure in some phase of the pipeline.
#[derive(Debug, Error)]
pub enum PipelineError {
    /// The recipe declares no source to fetch.
    #[error("recipe has no [[source]]")]
    NoSource,
    /// The recipe declares no `[build]` to run.
    #[error("recipe has no [build]")]
    NoBuild,
    /// The recipe declares no `[artifacts]` to collect.
    #[error("recipe has no [artifacts]")]
    NoArtifacts,
    /// The fetch phase failed.
    #[error("fetch: {0}")]
    Fetch(#[from] FetchError),
    /// Reading the stored source back failed.
    #[error("store: {0}")]
    Store(#[from] StoreError),
    /// The extract phase failed.
    #[error("extract: {0}")]
    Extract(#[from] ExtractError),
    /// Applying a source patch failed.
    #[error("patch: {0}")]
    Patch(#[from] PatchError),
    /// The build phase failed.
    #[error("build: {0}")]
    Build(#[from] BuildError),
    /// The artifact-collection phase failed.
    #[error("collect: {0}")]
    Collect(#[from] PackageError),
    /// Synthesising the manifest failed.
    #[error("manifest: {0}")]
    Manifest(#[from] ManifestError),
    /// Writing the `.lunpkg` failed.
    #[error("write: {0}")]
    Write(#[from] WriteError),
    /// A working-directory error.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// What a successful build produced.
#[derive(Debug)]
pub struct BuildOutcome {
    /// Path to the produced, signed `.lunpkg`.
    pub lunpkg: PathBuf,
    /// Content address of the fetched source.
    pub source: ContentHash,
    /// What was collected into the package.
    pub collection: Collection,
}

/// Build `recipe` into a signed `.lunpkg` under `out_dir`, returning the path.
///
/// `runner` is the build seam (production wraps it in the sandbox). `downloader`,
/// `git_fetcher` and `release_resolver` are the fetch seams. A `github-release`
/// source's asset 3xx-redirects to a CDN, so the production caller must pass a
/// redirect-following `downloader` (`RedirectingHttpDownloader`); the plain
/// `HttpDownloader` would fail such a fetch. The fetched source is rooted in the
/// store under the recipe id.
///
/// `recipe_dir` is the directory the recipe was read from; the primary source's
/// declared `patches` are resolved relative to it and applied to the extracted
/// tree before the build.
#[allow(clippy::too_many_arguments)]
pub async fn build_recipe(
    recipe: &Recipe,
    recipe_dir: &Path,
    store: &Store,
    downloader: &dyn Downloader,
    git_fetcher: &dyn GitFetcher,
    release_resolver: &dyn ReleaseResolver,
    runner: &dyn StepRunner,
    ctx: &BuildContext,
    signing_key: &SigningKey,
    out_dir: &Path,
    limits: &PipelineLimits,
) -> Result<BuildOutcome, PipelineError> {
    let owner = recipe.recipe.id.as_str();
    let source = recipe.source.first().ok_or(PipelineError::NoSource)?;
    // D6: a `crate` source whose `version` is omitted defaults to the recipe's
    // own version, so a recipe packaging exactly one crate need not repeat it.
    let crate_versioned;
    let source = if matches!(source.source_type, SourceType::Crate) && source.version.is_none() {
        crate_versioned = Source {
            version: recipe.recipe.version.clone(),
            ..source.clone()
        };
        &crate_versioned
    } else {
        source
    };
    let build = recipe.build.as_ref().ok_or(PipelineError::NoBuild)?;
    let artifacts = recipe.artifacts.as_ref().ok_or(PipelineError::NoArtifacts)?;

    // 1. Fetch the primary source into the store (verified against its pin).
    let source_hash = fetch_source(
        source,
        owner,
        store,
        downloader,
        git_fetcher,
        release_resolver,
        limits.fetch_max_bytes,
    )
    .await?;

    // 2. Extract the stored archive into a build directory.
    let build_dir = tempfile::tempdir()?;
    let source_bytes = store.read(&source_hash)?;
    extract_tar(&source_bytes, build_dir.path(), &limits.extract)?;

    // 2b. Apply the primary source's declared patches (relative to the recipe
    //     directory) to the extracted tree, before the build sees it.
    if !source.patches.is_empty() {
        apply_patches(build_dir.path(), recipe_dir, &source.patches, &limits.patch)?;
    }

    // 3. Plan and run the build (through the runner seam) in the build dir. The
    //    physical build path is known only here, so the reproducibility
    //    path-remap target is set on a local copy of the context (the caller's
    //    `build_dir` hint is advisory and overridden). A non-UTF-8 path leaves
    //    it unset (no remap) rather than emitting a lossy, mismatched flag.
    let mut ctx = ctx.clone();
    ctx.build_dir = build_dir.path().to_str().map(str::to_string);
    let plan = plan_build(build, &ctx)?;
    execute_plan(&plan, runner, build_dir.path())?;

    // 4. Collect only the declared artifacts into a fresh staging tree (a
    //    sibling of the build dir, so it never overlaps it).
    let staging = tempfile::tempdir()?;
    let collection = collect_artifacts(build_dir.path(), artifacts, staging.path())?;

    // 5. Give the package its AppStream component, so a forage app can land in
    //    the same composed catalog as an apt or Flatpak one rather than being a
    //    special case the store has to know about.
    //
    //    THIS STEP WAS MISSING AND ITS ABSENCE WAS INVISIBLE. `write_metainfo`
    //    has existed since the harvest work, and its own doc says "the forage
    //    pipeline calls this after collecting artifacts" - which nothing did, so
    //    every package this pipeline has ever produced went out with no
    //    component in it. Nothing failed: a `.lunpkg` without metainfo installs
    //    fine and simply never appears in the store.
    //
    //    Upstream's own document wins when the source ships one: it is what the
    //    project wrote about itself, screenshots and all, where the synthesized
    //    one carries only what a recipe declares. Best-effort - a package that
    //    cannot be described is still a package worth installing, so a failure
    //    here costs the store row and not the build.
    if let Err(e) = describe_package(staging.path(), build_dir.path(), recipe) {
        eprintln!("forage: could not write the AppStream component: {e}");
    }

    // 5b. Compose the package's own AppStream catalogue, so the store has the
    //     app's icon on local disk instead of a name with nothing behind it. The
    //     result rides inside the package under `share/swcatalog`, which installd
    //     copies verbatim, so it arrives and departs with the package.
    compose_catalogue(staging.path())?;

    // 6. Synthesise the manifest and write the signed .lunpkg.
    std::fs::create_dir_all(out_dir)?;
    let manifest = synthesize_manifest(recipe, &collection)?;
    let lunpkg = out_dir.join(format!("{}.lunpkg", recipe.recipe.id));
    write_lunpkg(staging.path(), &manifest, signing_key, &lunpkg)?;

    Ok(BuildOutcome {
        lunpkg,
        source: source_hash,
        collection,
    })
}

/// Compose the staged package's own AppStream catalogue into `share/swcatalog`.
///
/// `appstreamcli compose` turns the component document plus the package's desktop
/// entry and icon files into the catalogue form the store reads, and - the part
/// nothing else does - extracts and scales the icon into a cache the store can show
/// without a network round trip.
///
/// Composed into a temporary directory and copied in afterwards, never in place:
/// the tool SCANS the tree it is given, and writing its output inside that tree
/// while it walks it invites it to read its own product back.
///
/// Never fatal, but never silent either. A refused component (compose is strict: no
/// description, no category on a GUI app, a desktop entry with no icon) means this
/// package will install and never appear in the store, and the maintainer building
/// it is the only person who can fix that - so the reason is printed, naming the
/// consequence. What it does NOT do is stop the build: the package is installable
/// and useful, and refusing to produce it over a store listing would be a poor
/// trade. The mistake the missing metainfo step made was saying nothing at all, not
/// failing to abort.
fn compose_catalogue(staging_root: &Path) -> Result<(), PipelineError> {
    let out = tempfile::tempdir()?;
    let result = std::process::Command::new("appstreamcli")
        .arg("compose")
        .arg(format!("--origin={CATALOG_ORIGIN}"))
        // The staging tree is the package's own prefix: `share/...`, no `usr`.
        .arg("--prefix=/")
        .arg(format!("--result-root={}", out.path().display()))
        .arg(format!("--data-dir={}", out.path().join("xml").display()))
        // `icons/<origin>/<size>/<name>`, the layout Debian's `icons-*.tar.gz`
        // unpacks into, so the store resolves a cached name the same way for a
        // forage package as for an archive one. Compose leaves out the origin level
        // when handed an icons directory, so it is put here.
        .arg(format!(
            "--icons-dir={}",
            out.path().join("icons").join(CATALOG_ORIGIN).display()
        ))
        // A build must not reach the network to describe what it just built.
        .arg("--no-net")
        .arg(staging_root)
        .output();
    let output = match result {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("forage: appstreamcli is not installed, so this package ships no catalogue");
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    if !output.status.success() {
        // Its own hint lines, which name the component and the reason. The colour
        // escapes go with them: this is going into a build log, not a terminal.
        let out_text = String::from_utf8_lossy(&output.stdout);
        let why: Vec<String> = out_text
            .lines()
            .filter(|l| l.contains("E: ") || l.contains("W: "))
            .map(|l| strip_ansi(l.trim()))
            .collect();
        let why = if why.is_empty() {
            String::from_utf8_lossy(&output.stderr).trim().to_string()
        } else {
            why.join("; ")
        };
        eprintln!(
            "forage: this package will not appear in the store, because its AppStream \
             component was refused: {why}"
        );
        return Ok(());
    }
    let dest = staging_root.join("share/swcatalog");
    std::fs::create_dir_all(&dest)?;
    for name in ["xml", "icons"] {
        let from = out.path().join(name);
        if from.exists() {
            copy_tree(&from, &dest.join(name))?;
        }
    }
    Ok(())
}

/// Drop ANSI colour escapes, so a hint line reads in a log file.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        // `ESC [ ... <letter>`: skip to the terminating letter.
        for c in chars.by_ref() {
            if c.is_ascii_alphabetic() {
                break;
            }
        }
    }
    out
}

/// Copy a directory tree, creating what it needs. Small and local because the one
/// tree this moves is the compose result, which is a handful of files.
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Put an AppStream component into the staged package.
///
/// Upstream's own `metainfo.xml` if the source ships one - it is what the project
/// wrote about itself, with screenshots and a real description, where the
/// synthesized document carries only what a recipe declares. Otherwise the
/// synthesized fallback, so every forage package describes itself somehow.
///
/// The search walks the BUILD tree rather than the staging one: staging holds only
/// the declared artifacts, and a project's metainfo lives in its source unless the
/// recipe happened to collect it.
fn describe_package(
    staging_root: &Path,
    build_dir: &Path,
    recipe: &Recipe,
) -> std::io::Result<PathBuf> {
    let candidates = walk_files(build_dir, 6);
    if let Some(upstream) = find_upstream_metainfo(&candidates) {
        let dir = staging_root.join("share/metainfo");
        std::fs::create_dir_all(&dir)?;
        // Named for the recipe id rather than kept under upstream's filename: the
        // store keys on the component id, and a document whose name disagrees
        // with the id it declares is the kind of mismatch that reads as two apps.
        let dest = dir.join(format!("{}.metainfo.xml", recipe.recipe.id));
        std::fs::copy(&upstream, &dest)?;
        return Ok(dest);
    }
    write_metainfo(staging_root, &recipe.recipe, recipe.artifacts.as_ref())
}

/// Every file under `root`, to a bounded depth.
///
/// Bounded because a build tree is somebody else's, and an unbounded walk over a
/// source that vendors a dependency tree is a lot of directory reads for a file
/// that lives near the top if it exists at all.
fn walk_files(root: &Path, depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, level)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if level < depth {
                    stack.push((path, level + 1));
                }
            } else {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_forage_build::BuildCommand;
    use async_trait::async_trait;

    /// A downloader that returns a fixed tar archive (a source tree).
    struct CannedDownloader(Vec<u8>);
    #[async_trait]
    impl Downloader for CannedDownloader {
        async fn get(&self, _url: &str, _max: u64) -> Result<Vec<u8>, FetchError> {
            Ok(self.0.clone())
        }
    }

    /// A git fetcher that is never called on the tarball path.
    struct UnusedGit;
    impl GitFetcher for UnusedGit {
        fn fetch_commit(&self, _u: &str, _c: &str, _d: &Path, _m: u64) -> Result<Vec<u8>, FetchError> {
            panic!("git fetcher must not be used for a tarball source")
        }
    }

    /// A release resolver that is never called on the tarball path.
    struct UnusedResolver;
    #[async_trait]
    impl ReleaseResolver for UnusedResolver {
        async fn resolve(
            &self,
            _: &str,
            _: Option<&str>,
            _: &str,
        ) -> Result<arlen_forage_fetch::ResolvedRelease, FetchError> {
            panic!("release resolver must not be used for a tarball source")
        }
    }

    /// A build runner that simulates a build by writing the declared artifact
    /// into the build dir (instead of running real tools).
    struct ArtifactWritingRunner {
        rel: String,
    }
    impl StepRunner for ArtifactWritingRunner {
        fn run(&self, _cmd: &BuildCommand, source_root: &Path) -> Result<(), BuildError> {
            let out = source_root.join(&self.rel);
            if let Some(p) = out.parent() {
                std::fs::create_dir_all(p).unwrap();
            }
            std::fs::write(out, b"BUILT-BINARY").unwrap();
            Ok(())
        }
    }

    /// A runner that records the environment of the command it is handed and
    /// also writes the declared artifact so the pipeline completes.
    struct EnvCapturingRunner {
        rel: String,
        env: std::sync::Arc<std::sync::Mutex<std::collections::BTreeMap<String, String>>>,
    }
    impl StepRunner for EnvCapturingRunner {
        fn run(&self, cmd: &BuildCommand, source_root: &Path) -> Result<(), BuildError> {
            *self.env.lock().unwrap() = cmd.env.clone();
            let out = source_root.join(&self.rel);
            if let Some(p) = out.parent() {
                std::fs::create_dir_all(p).unwrap();
            }
            std::fs::write(out, b"BUILT-BINARY").unwrap();
            Ok(())
        }
    }

    /// A runner that copies a (possibly patched) source file into the artifact
    /// path, so the produced package reflects the source the build actually saw.
    struct SourceCopyingRunner {
        from: String,
        to: String,
    }
    impl StepRunner for SourceCopyingRunner {
        fn run(&self, _cmd: &BuildCommand, source_root: &Path) -> Result<(), BuildError> {
            let content = std::fs::read(source_root.join(&self.from)).unwrap();
            let out = source_root.join(&self.to);
            if let Some(p) = out.parent() {
                std::fs::create_dir_all(p).unwrap();
            }
            std::fs::write(out, content).unwrap();
            Ok(())
        }
    }

    fn source_tarball() -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        let mut h = tar::Header::new_gnu();
        let data = b"the source";
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        h.set_entry_type(tar::EntryType::Regular);
        b.append_data(&mut h, "src/main.rs", &data[..]).unwrap();
        b.into_inner().unwrap()
    }

    /// A minimal tarball recipe, parsed rather than constructed.
    ///
    /// Written as the TOML a recipe author would write, so the fixture tracks
    /// the real schema and adding an optional field to any of these structs
    /// does not break it. The struct-literal version broke on every new field
    /// in `[recipe]`, `[[source]]`, `[build]` or `[artifacts]` - six exhaustive
    /// literals for one fixture.
    fn recipe_for(sha: &str) -> Recipe {
        arlen_forage_recipe::parse(&format!(
            r#"
[recipe]
id = "org.example.demo"
name = "demo"
version = "1.0.0"
summary = "demo"
license = "MIT"
maintainer = "key:demo"

[[source]]
type = "tarball"
url = "https://example.org/src.tar"
sha256 = "{sha}"

[build]
system = "custom"
offline = true

[[build.steps]]
tool = "true"

[artifacts]
bin = ["app"]
"#
        ))
        .expect("the fixture recipe parses")
    }

    #[tokio::test]
    async fn end_to_end_tarball_recipe_produces_a_lunpkg() {
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        let out = tempfile::tempdir().unwrap();

        let tarball = source_tarball();
        let sha = ContentHash::of(&tarball);
        let recipe = recipe_for(sha.as_str());

        let outcome = build_recipe(
            &recipe,
            out.path(),
            &store,
            &CannedDownloader(tarball),
            &UnusedGit,
            &UnusedResolver,
            &ArtifactWritingRunner { rel: "app".into() },
            &BuildContext {
                source_date_epoch: 0,
                jobs: 1,
                build_dir: None,
            },
            &SigningKey::from_bytes(&[9u8; 32]),
            out.path(),
            &PipelineLimits::default(),
        )
        .await
        .expect("pipeline succeeds end to end");

        assert!(outcome.lunpkg.exists(), "a .lunpkg was produced");
        assert_eq!(outcome.collection.binaries, vec!["bin/app"]);
        assert_eq!(outcome.source, sha);

        // The produced package is a real signed .lunpkg: extract and check the
        // manifest + signature file are present and the binary was collected.
        let bytes = std::fs::read(&outcome.lunpkg).unwrap();
        let extracted = tempfile::tempdir().unwrap();
        extract_tar(&bytes, extracted.path(), &ExtractLimits::default()).unwrap();
        assert!(extracted.path().join("manifest.toml").exists());
        assert!(extracted.path().join("signature.sig").exists());
        assert_eq!(
            std::fs::read(extracted.path().join("bin/app")).unwrap(),
            b"BUILT-BINARY"
        );
    }

    #[tokio::test]
    async fn a_built_package_carries_an_appstream_component() {
        // THE case for step 5, and the one whose absence was invisible: a
        // `.lunpkg` with no metainfo installs perfectly and simply never appears
        // in the store, so nothing anywhere failed while every package this
        // pipeline produced was undescribable.
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        let out = tempfile::tempdir().unwrap();
        let tarball = source_tarball();
        let recipe = recipe_for(ContentHash::of(&tarball).as_str());

        let outcome = build_recipe(
            &recipe,
            out.path(),
            &store,
            &CannedDownloader(tarball),
            &UnusedGit,
            &UnusedResolver,
            &ArtifactWritingRunner { rel: "app".into() },
            &BuildContext { source_date_epoch: 0, jobs: 1, build_dir: None },
            &SigningKey::from_bytes(&[9u8; 32]),
            out.path(),
            &PipelineLimits::default(),
        )
        .await
        .expect("pipeline succeeds end to end");

        let bytes = std::fs::read(&outcome.lunpkg).unwrap();
        let extracted = tempfile::tempdir().unwrap();
        extract_tar(&bytes, extracted.path(), &ExtractLimits::default()).unwrap();
        let doc = extracted
            .path()
            .join(format!("share/metainfo/{}.metainfo.xml", recipe.recipe.id));
        assert!(doc.exists(), "the package describes itself at {}", doc.display());
        let xml = std::fs::read_to_string(&doc).unwrap();
        // The id in the document has to be the id the store will key on, or the
        // component and the package are two different apps as far as it knows.
        assert!(
            xml.contains(&format!("<id>{}</id>", recipe.recipe.id)),
            "the component declares the recipe's own id: {xml}"
        );

        // And the composed catalogue beside it, which is what puts the app in the
        // store's list rather than only in its own directory. Conditional on the
        // tool, because the step skips itself without it and a test that asserted
        // regardless would fail for a reason that is not about this code.
        //
        // THE GUARD ASKS WHETHER COMPOSE WORKS, NOT WHETHER A BINARY SPAWNED. It
        // used to be `Command::new("appstreamcli").arg("--version").output().is_ok()`,
        // and `output()` is `Ok` the moment a process starts, whatever it exits
        // with. Debian splits the tool in two - `appstream` carries
        // `/usr/bin/appstreamcli`, `appstream-compose` carries the
        // `/usr/libexec/appstreamcli-compose` the subcommand runs - so on a runner
        // with only the first, `appstreamcli` existed, the guard admitted the run,
        // `compose` did not work, and the test demanded a catalogue nothing could
        // have written. It passed here because this machine has both.
        let catalogue = extracted.path().join("share/swcatalog/xml/forage.xml.gz");
        let compose_works = std::process::Command::new("appstreamcli")
            .args(["compose", "--help"])
            .output()
            .is_ok_and(|o| o.status.success());
        if compose_works {
            assert!(
                catalogue.exists(),
                "the package carries its own catalogue at {}",
                catalogue.display(),
            );
        } else {
            eprintln!(
                "`appstreamcli compose` does not work here, so the catalogue half of this \
                 test did not run. On Debian that means the `appstream-compose` package"
            );
        }
    }

    #[tokio::test]
    async fn missing_phases_are_rejected() {
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        let out = tempfile::tempdir().unwrap();
        let mut recipe = recipe_for(&"a".repeat(64));
        recipe.build = None;
        let err = build_recipe(
            &recipe,
            out.path(),
            &store,
            &CannedDownloader(Vec::new()),
            &UnusedGit,
            &UnusedResolver,
            &ArtifactWritingRunner { rel: "app".into() },
            &BuildContext { source_date_epoch: 0, jobs: 1, build_dir: None },
            &SigningKey::from_bytes(&[9u8; 32]),
            out.path(),
            &PipelineLimits::default(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, PipelineError::NoBuild));
    }

    #[tokio::test]
    async fn pipeline_applies_source_patches_before_build() {
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        let out = tempfile::tempdir().unwrap();
        let recipe_dir = tempfile::tempdir().unwrap();

        // A source tree with a line the patch will rewrite.
        let tarball = {
            let mut b = tar::Builder::new(Vec::new());
            let data = b"the source\n";
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_entry_type(tar::EntryType::Regular);
            b.append_data(&mut h, "src/main.rs", &data[..]).unwrap();
            b.into_inner().unwrap()
        };
        let sha = ContentHash::of(&tarball);

        std::fs::write(
            recipe_dir.path().join("edit.patch"),
            "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1 +1 @@\n-the source\n+patched source\n",
        )
        .unwrap();

        let mut recipe = recipe_for(sha.as_str());
        recipe.source[0].patches = vec![PathBuf::from("edit.patch")];

        build_recipe(
            &recipe,
            recipe_dir.path(),
            &store,
            &CannedDownloader(tarball),
            &UnusedGit,
            &UnusedResolver,
            &SourceCopyingRunner { from: "src/main.rs".into(), to: "app".into() },
            &BuildContext { source_date_epoch: 0, jobs: 1, build_dir: None },
            &SigningKey::from_bytes(&[9u8; 32]),
            out.path(),
            &PipelineLimits::default(),
        )
        .await
        .expect("pipeline with a patch succeeds");

        // The package's artifact carries the patched content, proving the patch
        // was applied to the source the build saw.
        let bytes = std::fs::read(out.path().join("org.example.demo.lunpkg")).unwrap();
        let extracted = tempfile::tempdir().unwrap();
        extract_tar(&bytes, extracted.path(), &ExtractLimits::default()).unwrap();
        assert_eq!(
            std::fs::read_to_string(extracted.path().join("bin/app")).unwrap(),
            "patched source\n"
        );
    }

    #[tokio::test]
    async fn pipeline_wires_the_real_build_dir_into_the_remap_flag() {
        // The reproducibility path-remap is only useful if the pipeline sets the
        // physical build directory (known only at run time) into the context.
        // Capture the command env the runner receives and assert the remap flag
        // names a real absolute path mapped to the canonical mount.
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        let out = tempfile::tempdir().unwrap();
        let tarball = source_tarball();
        let sha = ContentHash::of(&tarball);
        let captured = std::sync::Arc::new(std::sync::Mutex::new(std::collections::BTreeMap::new()));
        build_recipe(
            &recipe_for(sha.as_str()),
            out.path(),
            &store,
            &CannedDownloader(tarball),
            &UnusedGit,
            &UnusedResolver,
            &EnvCapturingRunner { rel: "app".into(), env: captured.clone() },
            // The caller's hint is None; the pipeline overrides it with the real path.
            &BuildContext { source_date_epoch: 0, jobs: 1, build_dir: None },
            &SigningKey::from_bytes(&[9u8; 32]),
            out.path(),
            &PipelineLimits::default(),
        )
        .await
        .expect("pipeline succeeds");

        let env = captured.lock().unwrap();
        let rustflags = env.get("RUSTFLAGS").expect("remap flag injected by the pipeline");
        let prefix = rustflags
            .strip_prefix("--remap-path-prefix=")
            .expect("the remap flag form");
        let (from, to) = prefix.split_once('=').expect("from=to");
        assert!(Path::new(from).is_absolute(), "the real build dir is absolute: {from}");
        assert_eq!(to, "/build", "mapped to the canonical mount");
    }
}
