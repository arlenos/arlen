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
/// Reports a forage build as one composite job to the shell's Activity/Jobs
/// zone (job-progress-surface.md step 2). Entirely best-effort: the daemon may
/// be absent (a headless CI run or no desktop session), and a report failure
/// must never affect the build, so every call swallows its error. The job is
/// indeterminate - a build's true progress is not a clean count - so the shell
/// shows the title and says it is running, and draws no bar - there is nothing
/// measured to draw one from. Not a spinner: the kit's `Progress` is
/// determinate-only, so an indeterminate row is a title and a word until that
/// changes, which is honest if plainer than this comment used to claim.
struct BuildJob {
    proxy: notification_proto::client::JobViewServerProxy<'static>,
    id: u64,
}

/// The flag a build watches, and the one the shell's Cancel raises.
type CancelFlag = std::sync::Arc<std::sync::atomic::AtomicBool>;

impl BuildJob {
    /// Register the build job, or `None` when the job server is unreachable.
    ///
    /// KILLABLE, now that a raised flag actually stops the build: the runner
    /// polls it between short waits and kills the command's whole process group.
    /// It answered `false` for as long as there was no way to interrupt a build,
    /// which was the right answer then - a Cancel button that stops nothing is
    /// the promise this surface exists not to make.
    ///
    /// Not suspendable: pausing a build means holding an unbounded wait inside a
    /// sandbox with the source tree open, and nothing resumes it cleanly.
    async fn start(title: &str, cancel: CancelFlag) -> Option<BuildJob> {
        let conn = zbus::Connection::session().await.ok()?;
        let proxy = notification_proto::client::JobViewServerProxy::new(&conn)
            .await
            .ok()?;
        // total=0/determinate=false -> indeterminate; no egress line at this layer.
        let id = proxy
            .register("forage", title, "items", 0, false, true, false, "", &[])
            .await
            .ok()?;

        // Every producer receives every CancelRequested, so this acts only on
        // its own id. Best-effort like the rest: no stream means no cancel from
        // the shell, and the build runs as it did before.
        if let Ok(mut stream) = proxy.receive_cancel_requested().await {
            tokio::spawn(async move {
                use futures_util::StreamExt;
                while let Some(signal) = stream.next().await {
                    if let Ok(args) = signal.args() {
                        if *args.id() == id {
                            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                            return;
                        }
                    }
                }
            });
        }
        Some(BuildJob { proxy, id })
    }

    /// Terminate the job and say how it ended.
    ///
    /// A STOPPED BUILD IS `done`, NOT AN ERROR - the convention the file manager
    /// set for a cancelled copy: the person asked for it, so the row says what
    /// became of the work rather than reporting a failure they caused on purpose.
    async fn finish(self, outcome: Outcome) {
        let (state, message) = match outcome {
            Outcome::Built => ("done", ""),
            Outcome::Stopped => ("done", "The build was stopped. Nothing was packaged."),
            Outcome::Failed => ("error-fatal", "build failed"),
        };
        let _ = self.proxy.set_state(self.id, state, message).await;
        let _ = self.proxy.finish(self.id).await;
    }
}

/// How a build ended, kept apart from `Result` because a stop is not a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// A package was produced.
    Built,
    /// Somebody asked it to stop, and it did.
    Stopped,
    /// It went wrong.
    Failed,
}

/// A human title for the build job, derived from the recipe's directory (the
/// project being built), falling back to a generic label.
fn build_title(recipe_path: &Path) -> String {
    let name = recipe_path
        .parent()
        .and_then(|p| {
            // A `.forage/recipe.toml` layout: name after the project dir.
            if p.file_name().map(|n| n == ".forage").unwrap_or(false) {
                p.parent().and_then(|g| g.file_name())
            } else {
                p.file_name()
            }
        })
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty());
    match name {
        Some(n) => format!("Building {n}"),
        None => "Building package".to_string(),
    }
}

pub async fn run(path: PathBuf, unsafe_no_sandbox: bool, install: bool) {
    let Some(recipe_path) = recipe::resolve_recipe_path(&path) else {
        exit(1);
    };
    // Report the build as one composite job to the shell (best-effort).
    let cancel: CancelFlag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let job = BuildJob::start(&build_title(&recipe_path), std::sync::Arc::clone(&cancel)).await;
    let result = build_recipe_at(&recipe_path, unsafe_no_sandbox, Some(cancel.clone())).await;
    let stopped = cancel.load(std::sync::atomic::Ordering::Relaxed);
    if let Some(job) = job {
        job.finish(match (&result, stopped) {
            (Ok(_), _) => Outcome::Built,
            (Err(()), true) => Outcome::Stopped,
            (Err(()), false) => Outcome::Failed,
        })
        .await;
    }
    let lunpkg = match result {
        Ok(p) => p,
        // A stop is not a failure, and the exit code says so: somebody asked for
        // it. The shell's row already said what became of the work; this is the
        // same sentence for whoever ran the command in a terminal.
        Err(()) if stopped => {
            println!("{} the build was stopped", "cancelled".yellow().bold());
            exit(0)
        }
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
pub async fn build_recipe_at(
    recipe_path: &Path,
    unsafe_no_sandbox: bool,
    cancel: Option<CancelFlag>,
) -> Result<PathBuf, ()> {
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
        Box::new(ProcessRunner::with_limits(arlen_forage_build::BuildLimits {
            cancel: cancel.clone(),
            ..Default::default()
        }))
    } else {
        match cfg.require_base_platform() {
            Ok(base) => Box::new(ConfinedStepRunner::new(base).with_limits(
                arlen_forage_build::BuildLimits {
                    cancel: cancel.clone(),
                    ..Default::default()
                },
            )),
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
            // A STOP IS NOT AN ERROR AND MUST NOT BE ANNOUNCED AS ONE. The
            // pipeline reports a cancel through the same `Err` as a real
            // failure, so without this the run printed a red
            // `error: build: build cancelled` and then, one line later,
            // `cancelled the build was stopped` - two sentences disagreeing
            // about what had just happened, the second one right. The caller
            // owns the sentence for a stop; this branch is for things that went
            // wrong.
            if !cancel.is_some_and(|f| f.load(std::sync::atomic::Ordering::Relaxed)) {
                eprintln!("{} {e}", "error:".red().bold());
            }
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
