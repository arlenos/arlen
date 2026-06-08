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
use arlen_forage_fetch::{fetch_source, Downloader, FetchError, GitFetcher, ReleaseResolver};
use arlen_forage_package::{
    collect_artifacts, synthesize_manifest, write_lunpkg, Collection, ManifestError, PackageError,
    WriteError,
};
use arlen_forage_recipe::Recipe;
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
}

impl Default for PipelineLimits {
    fn default() -> Self {
        PipelineLimits {
            fetch_max_bytes: arlen_forage_fetch::DEFAULT_MAX_BYTES,
            extract: ExtractLimits::default(),
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
/// `runner` is the build seam (production wraps it in the sandbox). `downloader`
/// and `git_fetcher` are the fetch seams. The fetched source is rooted in the
/// store under the recipe id.
#[allow(clippy::too_many_arguments)]
pub async fn build_recipe(
    recipe: &Recipe,
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

    // 3. Plan and run the build (through the runner seam) in the build dir.
    let plan = plan_build(build, ctx)?;
    execute_plan(&plan, runner, build_dir.path())?;

    // 4. Collect only the declared artifacts into a fresh staging tree (a
    //    sibling of the build dir, so it never overlaps it).
    let staging = tempfile::tempdir()?;
    let collection = collect_artifacts(build_dir.path(), artifacts, staging.path())?;

    // 5. Synthesise the manifest and write the signed .lunpkg.
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

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_forage_build::BuildCommand;
    use arlen_forage_recipe::{Artifacts, Build, BuildStep, BuildSystem, RecipeMeta, Source, SourceType};
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

    fn recipe_for(sha: &str) -> Recipe {
        Recipe {
            recipe: RecipeMeta {
                id: "org.example.demo".into(),
                name: "demo".into(),
                version: Some("1.0.0".into()),
                summary: Some("demo".into()),
                license: Some("MIT".into()),
                homepage: None,
                maintainer: "key:demo".into(),
                recipe_revision: 1,
                category: Vec::new(),
            },
            source: vec![Source {
                source_type: SourceType::Tarball,
                url: Some("https://example.org/src.tar".into()),
                commit: None,
                sha256: Some(sha.into()),
                asset: None,
                tag: None,
                patches: Vec::new(),
            }],
            build: Some(Build {
                system: Some(BuildSystem::Custom),
                host_deps: Vec::new(),
                config_opts: Vec::new(),
                env: Default::default(),
                steps: vec![BuildStep {
                    tool: "true".into(),
                    args: Vec::new(),
                    workdir: None,
                }],
                offline: true,
                jobs: None,
                fetch_lock: None,
            }),
            artifacts: Some(Artifacts {
                bin: vec!["app".into()],
                lib: Vec::new(),
                include: Vec::new(),
                share: Vec::new(),
                libexec: Vec::new(),
                desktop: None,
                icon: None,
            }),
            capabilities: None,
            provides: None,
            depends: None,
            reproducible: None,
        }
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
            &store,
            &CannedDownloader(tarball),
            &UnusedGit,
            &UnusedResolver,
            &ArtifactWritingRunner { rel: "app".into() },
            &BuildContext {
                source_date_epoch: 0,
                jobs: 1,
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
    async fn missing_phases_are_rejected() {
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        let out = tempfile::tempdir().unwrap();
        let mut recipe = recipe_for(&"a".repeat(64));
        recipe.build = None;
        let err = build_recipe(
            &recipe,
            &store,
            &CannedDownloader(Vec::new()),
            &UnusedGit,
            &UnusedResolver,
            &ArtifactWritingRunner { rel: "app".into() },
            &BuildContext { source_date_epoch: 0, jobs: 1 },
            &SigningKey::from_bytes(&[9u8; 32]),
            out.path(),
            &PipelineLimits::default(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, PipelineError::NoBuild));
    }
}
