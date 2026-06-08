//! Build subcommand: turn a recipe into a signed `.lunpkg`.
//!
//! This wires the forage backend (`arlen-forage-pipeline`) behind the CLI:
//! resolve and validate the recipe, then run fetch -> extract -> patch -> build
//! -> collect -> sign. The build runs in the pinned base platform by default
//! (its location read from config); `--unsafe-no-sandbox` runs it unconfined for
//! a maintainer testing their own recipe. `tarball` and `git` sources build;
//! `github-release` needs a platform asset target (roadmap D7) and is refused
//! here for now. Installing the produced package is a separate step (`forage
//! install <file>`), so this command only ever produces and reports the path.

use std::path::{Path, PathBuf};
use std::process::exit;

use arlen_forage_build::{BuildContext, ConfinedStepRunner, ProcessRunner, StepRunner};
use arlen_forage_fetch::{GitHubReleaseResolver, ProcessGitFetcher, RedirectingHttpDownloader};
use arlen_forage_pipeline::{build_recipe, PipelineLimits};
use arlen_forage_recipe::{lint, parse, validate as validate_schema, SourceType};
use arlen_forage_signing::BuilderKey;
use arlen_forage_store::Store;
use colored::Colorize;

use crate::commands::build_config::ForageBuildConfig;
use crate::commands::recipe;

/// The persistent builder signing key path (`~/.local/share/arlen/forage/`).
fn builder_key_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("arlen/forage/builder.key")
}

/// The content-addressed source/artifact store root (`~/.cache/arlen/forage/`).
fn store_root() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("arlen/forage/store")
}

/// Build the recipe at `path` into a signed `.lunpkg`, printing the path on
/// success. Exits non-zero on any failure; a zero exit means a real package was
/// produced.
pub async fn run(path: PathBuf, unsafe_no_sandbox: bool, install: bool) {
    let Some(recipe_path) = recipe::resolve_recipe_path(&path) else {
        exit(1);
    };
    let lunpkg = match build_recipe_at(&recipe_path, unsafe_no_sandbox).await {
        Ok(p) => p,
        Err(()) => exit(1),
    };
    println!("{} {}", "built".green().bold(), lunpkg.display());
    if install {
        install_package(&lunpkg).await;
    }
}

/// Build the recipe file at `recipe_path` into a signed `.lunpkg`, returning its
/// path. `unsafe_no_sandbox` runs the build unconfined on the host (dev only,
/// for a recipe the user trusts); otherwise it runs inside the configured base
/// platform. Prints diagnostics and returns `Err(())` on any failure. Shared by
/// `forage build` and the `git+URL` install path, which always builds confined
/// because the remote recipe is untrusted.
pub async fn build_recipe_at(recipe_path: &Path, unsafe_no_sandbox: bool) -> Result<PathBuf, ()> {
    let content = match std::fs::read_to_string(recipe_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{} could not read {}: {e}", "error:".red(), recipe_path.display());
            return Err(());
        }
    };
    let recipe = match parse(&content) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{} {e}", "parse error:".red().bold());
            return Err(());
        }
    };
    let errors = validate_schema(&recipe);
    for w in lint(&recipe) {
        println!("{} {}: {}", "warning:".yellow(), w.field, w.message);
    }
    if !errors.is_empty() {
        for e in &errors {
            eprintln!("{} {}: {}", "error:".red(), e.field, e.message);
        }
        return Err(());
    }
    let recipe_dir = recipe_path.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();

    // github-release needs the platform asset target (roadmap D7); refuse it
    // here rather than half-resolve. tarball and git build through.
    if recipe
        .source
        .first()
        .is_some_and(|s| matches!(s.source_type, SourceType::GithubRelease))
    {
        eprintln!(
            "{} github-release sources are not yet buildable (roadmap D7); \
             use a tarball or git source.",
            "error:".red().bold()
        );
        return Err(());
    }

    let cfg = match ForageBuildConfig::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{} {e}", "error:".red().bold());
            return Err(());
        }
    };
    let key = match BuilderKey::load_or_create(&builder_key_path()) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("{} {e}", "error:".red().bold());
            return Err(());
        }
    };
    let store = match Store::open(store_root()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{} opening the store: {e}", "error:".red().bold());
            return Err(());
        }
    };

    // The build seam: confined in the base platform by default, or unconfined on
    // explicit opt-in. A boxed trait object so both paths share the call below.
    let runner: Box<dyn StepRunner> = if unsafe_no_sandbox {
        eprintln!(
            "{} building unconfined on the host; only safe for a recipe you trust.",
            "warning:".yellow().bold()
        );
        Box::new(ProcessRunner::default())
    } else {
        match cfg.require_base_platform() {
            Ok(base) => Box::new(ConfinedStepRunner::new(base)),
            Err(e) => {
                eprintln!("{} {e}", "error:".red().bold());
                eprintln!("  (or pass --unsafe-no-sandbox to build on the host for local testing)");
                return Err(());
            }
        }
    };

    // SOURCE_DATE_EPOCH and the job count are pinned for reproducibility; a
    // fixed epoch and single job keep the output deterministic until the commit
    // timestamp is threaded through.
    let ctx = BuildContext {
        source_date_epoch: 0,
        jobs: 1,
        build_dir: None,
    };
    // The release resolver is only consulted for github-release sources, refused
    // above, so an empty-target instance is never called here.
    let resolver = GitHubReleaseResolver::new(String::new());

    match build_recipe(
        &recipe,
        &recipe_dir,
        &store,
        &RedirectingHttpDownloader,
        &ProcessGitFetcher,
        &resolver,
        runner.as_ref(),
        &ctx,
        key.signing_key(),
        cfg.out_dir(),
        &PipelineLimits::default(),
    )
    .await
    {
        Ok(outcome) => Ok(outcome.lunpkg),
        Err(e) => {
            eprintln!("{} {e}", "error:".red().bold());
            Err(())
        }
    }
}

/// Install the just-built `.lunpkg` through installd, the same path as
/// `forage install <file>`. Exits non-zero if the daemon is unreachable or the
/// install fails, so `--install` reports an honest overall result.
async fn install_package(lunpkg: &Path) {
    use crate::commands::install_client as client;

    let conn = match client::connect().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{} {e}", "error:".red().bold());
            eprintln!(
                "{}",
                "is arlen-installd running? (systemctl --user start installd)".dimmed()
            );
            exit(1);
        }
    };
    let path = lunpkg.to_str().unwrap_or_default();
    match client::install_package(&conn, path).await {
        Ok(()) => println!("{} {}", "installed".green().bold(), lunpkg.display()),
        Err(e) => {
            eprintln!("{} {e}", "error:".red().bold());
            exit(1);
        }
    }
}
