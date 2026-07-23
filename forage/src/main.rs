//! forage - Arlen OS package manager CLI.
//!
//! See `docs/architecture/module-system.md` and
//! `docs/architecture/distro-package-management.md`.

mod cli;
mod commands;

use clap::Parser;
use cli::{BridgeAction, Cli, Commands, CookbookAction, ModuleAction, RecipeAction, TrashAction};
use colored::Colorize;

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Install { target } => run_async(cmd_install(target)),
        Commands::Remove { app_id } => run_async(cmd_remove(app_id)),
        Commands::List { json } => run_async(cmd_list(json)),
        Commands::Info { app_id, json } => run_async(cmd_info(app_id, json)),
        Commands::Which { app_id, json } => run_async(cmd_which(app_id, json)),
        Commands::Trash { action } => run_async(cmd_trash(action)),
        Commands::Module { action } => match action {
            ModuleAction::Register { path, force } => {
                commands::module::register(&path, force);
            }
            ModuleAction::List => {
                commands::module::list();
            }
            ModuleAction::Info { id } => {
                commands::module::info(&id);
            }
            ModuleAction::Remove { id } => {
                commands::module::remove(&id);
            }
            ModuleAction::Enable { id } => {
                commands::module::enable(&id);
            }
            ModuleAction::Disable { id } => {
                commands::module::disable(&id);
            }
        },
        Commands::Recipe { action } => match action {
            RecipeAction::Init { force } => commands::recipe::init(force),
            RecipeAction::New { id, dir } => commands::recipe::new(&id, dir.as_deref()),
            RecipeAction::Validate { path } => commands::recipe::validate(&path),
        },
        Commands::Build {
            path,
            unsafe_no_sandbox,
            install,
        } => run_async(commands::build::run(path, unsafe_no_sandbox, install)),
        Commands::Cookbook { action } => match action {
            CookbookAction::Add { name, url } => {
                run_async(commands::cookbook::add(name, url))
            }
            CookbookAction::Remove { name } => commands::cookbook::remove(&name),
            CookbookAction::List => commands::cookbook::list(),
            CookbookAction::Update => commands::cookbook::update(),
        },
        Commands::Challenge { target } => commands::challenge::challenge(&target),
        Commands::Bridge { action } => match action {
            BridgeAction::Install { foreign_app, yes } => {
                run_async(cmd_bridge_install(foreign_app, yes))
            }
        },
    }
}

/// Run an async function in a tokio runtime.
fn run_async(fut: impl std::future::Future<Output = ()>) {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime")
        .block_on(fut);
}

/// Resolve the install target and dispatch to the right method.
async fn cmd_install(target: String) {
    use commands::install_client as client;

    // `git+<url>[#<ref>]`: clone the recipe repo, build it (always sandboxed,
    // since a remote recipe is untrusted), then install the produced package
    // through the normal path below.
    if let Some(spec) = target.strip_prefix("git+") {
        let (url, git_ref) = match spec.split_once('#') {
            Some((u, r)) => (u, Some(r)),
            None => (spec, None),
        };
        let clone_dir = match tempfile::tempdir() {
            Ok(d) => d,
            Err(e) => {
                eprintln!("{} scratch dir: {e}", "error:".red().bold());
                std::process::exit(1);
            }
        };
        if let Err(e) = arlen_forage_fetch::clone_recipe_repo(
            url,
            git_ref,
            clone_dir.path(),
            arlen_forage_fetch::DEFAULT_RECIPE_REPO_BYTES,
        )
        .await
        {
            eprintln!("{} cloning {url}: {e}", "error:".red().bold());
            std::process::exit(1);
        }
        let Some(recipe_path) = commands::recipe::resolve_recipe_path(clone_dir.path()) else {
            std::process::exit(1);
        };
        // Untrusted remote recipe: never build it unconfined.
        let lunpkg = match commands::build::build_recipe_at(&recipe_path, false).await {
            Ok(p) => p,
            Err(()) => std::process::exit(1),
        };
        // The package is written to the out dir (not the clone), so it outlives
        // the scratch clone; install it via the normal local-file path.
        Box::pin(cmd_install(lunpkg.to_string_lossy().into_owned())).await;
        return;
    }

    // A bare reverse-DNS recipe name (not a package file or existing path):
    // resolve it through the tracked cookbooks. `flatpak:`/url forms fail
    // is_valid_id (its charset excludes `:` and `/`), so only the .lunpkg and
    // existing-path cases need excluding explicitly.
    if arlen_forage_recipe::is_valid_id(&target)
        && !target.ends_with(".lunpkg")
        && !std::path::Path::new(&target).exists()
    {
        Box::pin(install_by_name(target)).await;
        return;
    }

    // A bare name that is not a package spec (flatpak:/.lunpkg/path/url) may be a
    // foreign app with community bridges: `forage install obsidian` sets up the
    // Obsidian bridge (foreign-app-bridges.md section 4). Check before requiring
    // installd, so a bridge-only install works with installd stopped; a name that
    // matches no bridge falls through to the normal resolution error below.
    let is_package_spec = target.starts_with("flatpak:")
        || target.ends_with(".lunpkg")
        || std::path::Path::new(&target).exists()
        || target.starts_with("http://")
        || target.starts_with("https://");
    if !is_package_spec {
        if let Ok(bridges) = commands::cookbook::bridges_in_cookbooks(&target).await {
            if !bridges.is_empty() {
                install_resolved_bridges(&target, bridges, false).await;
                return;
            }
        }
    }

    let conn = match client::connect().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{} {e}", "error:".red().bold());
            eprintln!(
                "{}",
                "is arlen-installd running? (systemctl --user start installd)"
                    .dimmed()
            );
            std::process::exit(1);
        }
    };

    let result = if target.starts_with("flatpak:") {
        // flatpak:{app_id}
        let app_id = target.strip_prefix("flatpak:").unwrap();
        client::install_flatpak(&conn, app_id).await
    } else if target.ends_with(".lunpkg") || std::path::Path::new(&target).exists() {
        // Local .lunpkg file.
        let abs = std::fs::canonicalize(&target)
            .unwrap_or_else(|_| std::path::PathBuf::from(&target));
        client::install_package(&conn, abs.to_str().unwrap_or(&target)).await
    } else if target.starts_with("http://") || target.starts_with("https://") {
        eprintln!(
            "{} URL installation is not yet implemented",
            "error:".red().bold()
        );
        std::process::exit(1);
    } else {
        eprintln!(
            "{} cannot resolve '{}'. Expected a recipe name, a .lunpkg file, URL, or flatpak:{{app_id}}",
            "error:".red().bold(),
            target
        );
        std::process::exit(1);
    };

    if let Err(e) = result {
        eprintln!("{} {e}", "error:".red().bold());
        std::process::exit(1);
    }
}

/// Install a recipe by its reverse-DNS name through the cookbook system: resolve
/// and verify the pointer against the pinned cookbook root, clone the recipe at
/// the pinned commit, check the cloned `recipe.toml` against the cookbook's
/// signed hash, then build (always sandboxed, the remote recipe is untrusted)
/// and install. Any verification failure aborts.
async fn install_by_name(name: String) {
    use arlen_forage_fetch::{GitFetcher, ProcessGitFetcher, DEFAULT_RECIPE_REPO_BYTES};

    let resolved = match commands::cookbook::resolve_in_cookbooks(&name).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{} {e}", "error:".red().bold());
            std::process::exit(1);
        }
    };

    let url = match clone_url(&resolved.git_url) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("{} {e}", "error:".red().bold());
            std::process::exit(1);
        }
    };

    let clone = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{} scratch dir: {e}", "error:".red().bold());
            std::process::exit(1);
        }
    };
    // fetch_commit verifies the checkout is exactly the pinned commit.
    if let Err(e) =
        ProcessGitFetcher.fetch_commit(&url, &resolved.commit, clone.path(), DEFAULT_RECIPE_REPO_BYTES)
    {
        eprintln!("{} fetching {}@{}: {e}", "error:".red().bold(), url, resolved.commit);
        std::process::exit(1);
    }

    let Some(recipe_path) = commands::recipe::resolve_recipe_path(clone.path()) else {
        std::process::exit(1);
    };
    // The cookbook signed the sha256 of this recipe; a repo serving different
    // content than the cookbook vetted is rejected here.
    let recipe_bytes = match std::fs::read(&recipe_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{} reading recipe: {e}", "error:".red().bold());
            std::process::exit(1);
        }
    };
    if sha256_hex(&recipe_bytes) != resolved.recipe_hash {
        eprintln!(
            "{} the recipe at the pinned commit does not match the cookbook's signed hash; refusing",
            "error:".red().bold()
        );
        std::process::exit(1);
    }

    // Enforce the cookbook's capability cap: the recipe may declare at most the
    // capabilities the cookbook signed (section 7a). A recipe exceeding the
    // curated upper bound is refused. When the cookbook set no cap, the recipe's
    // own declaration stands (and still gates downstream at install and run).
    if let Some(cap) = &resolved.capability_cap {
        let recipe = match arlen_forage_recipe::parse(&String::from_utf8_lossy(&recipe_bytes)) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{} parsing recipe: {e}", "error:".red().bold());
                std::process::exit(1);
            }
        };
        let declared = recipe.capabilities.unwrap_or_default();
        let over = arlen_forage_capabilities::cap_exceeded(&declared, cap);
        if !over.is_empty() {
            eprintln!(
                "{} recipe exceeds the cookbook's capability cap: {}",
                "error:".red().bold(),
                over.join(", ")
            );
            std::process::exit(1);
        }
    }

    // Untrusted remote recipe: never build it unconfined.
    let lunpkg = match commands::build::build_recipe_at(&recipe_path, false).await {
        Ok(p) => p,
        Err(()) => std::process::exit(1),
    };
    Box::pin(cmd_install(lunpkg.to_string_lossy().into_owned())).await;
}

/// Install every cookbook bridge tagged for a foreign app (foreign-app-bridges.md
/// section 4): enumerate the tracked cookbooks for bridges whose `[bridge]
/// foreign_app` matches, take ONE install-time consent for the batch, fetch and
/// verify each against its signed pointer, then install both halves atomically
/// (Arlen-side schema/mapping + foreign-side plugin) and grant each a revocable KG
/// write scope. A failing batch is rolled back whole.
///
/// The foreign-side destination (e.g. `$VAULT` for Obsidian) resolves from the
/// environment; a missing token is reported by name rather than guessed.
async fn cmd_bridge_install(foreign_app: String, assume_yes: bool) {
    let resolved = match commands::cookbook::bridges_in_cookbooks(&foreign_app).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{} {e}", "error:".red().bold());
            std::process::exit(1);
        }
    };
    if resolved.is_empty() {
        println!("No bridges found for '{foreign_app}' in the tracked cookbooks.");
        return;
    }
    install_resolved_bridges(&foreign_app, resolved, assume_yes).await;
}

/// Install a pre-enumerated batch of bridges for `foreign_app`: one consent, then
/// fetch+verify+prepare each and install both halves atomically (shared by the
/// explicit `bridge install` command and the `forage install <foreign_app>`
/// fallback, so both take exactly the same consent + atomic path).
async fn install_resolved_bridges(
    foreign_app: &str,
    resolved: Vec<arlen_cookbook_resolve::ResolvedRecipe>,
    assume_yes: bool,
) {
    use arlen_forage_fetch::ProcessGitFetcher;

    println!("{} bridge(s) for {foreign_app}:", resolved.len());
    for r in &resolved {
        println!("  - {} ({})", r.name, r.git_url);
    }
    // The one install-time consent for the whole batch (section 4). Fail-closed: a
    // non-interactive or empty answer never installs without an explicit yes.
    if !assume_yes
        && !confirm(&format!(
            "Bridge {foreign_app} into your knowledge graph? \
             It installs the plugin and gets a revocable write scope. [y/N] "
        ))
    {
        println!("Skipped.");
        return;
    }

    // Fetch + verify + prepare each bridge; hold the checkouts alive until install
    // has copied the files out. A prepare failure aborts before anything is placed.
    let fetcher = ProcessGitFetcher;
    let mut checkouts = Vec::new();
    let mut prepared = Vec::new();
    for r in &resolved {
        match commands::bridge::prepare_bridge(&fetcher, r) {
            Ok((checkout, pb)) => {
                checkouts.push(checkout);
                prepared.push(pb);
            }
            Err(e) => {
                eprintln!("{} preparing bridge '{}': {e}", "error:".red().bold(), r.name);
                std::process::exit(1);
            }
        }
    }

    // Foreign-side destinations resolve from the environment (e.g. $VAULT -> the
    // user's vault). Report any unset token rather than installing halfway.
    let (tokens, missing) = commands::bridge::tokens_from_env(&prepared);
    if !missing.is_empty() {
        let names = missing
            .iter()
            .map(|n| format!("${n}"))
            .collect::<Vec<_>>()
            .join(", ");
        eprintln!(
            "{} set {names} to the foreign app's data location, then re-run",
            "error:".red().bold()
        );
        std::process::exit(1);
    }

    match commands::bridge::install_prepared_bridges(&prepared, &tokens) {
        Ok(results) => println!(
            "{} installed {} bridge(s) for {foreign_app}",
            "ok:".green().bold(),
            results.len()
        ),
        Err(e) => {
            eprintln!("{} {e} (rolled back)", "error:".red().bold());
            std::process::exit(1);
        }
    }
    drop(checkouts);
}

/// Prompt for a yes/no confirmation on the terminal, defaulting to no. A read
/// error or any answer other than `y`/`yes` is a no, so a non-interactive run
/// never proceeds without an explicit yes.
fn confirm(prompt: &str) -> bool {
    use std::io::Write;
    print!("{prompt}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return false;
    }
    matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// Build a clonable https URL from a cookbook's signed `git_url`. The host is
/// pinned by the cookbook signature and the content by the commit; this only
/// rejects an insecure transport and supplies https for a scheme-less host.
pub(crate) fn clone_url(git_url: &str) -> Result<String, String> {
    if let Some(rest) = git_url.strip_prefix("https://") {
        if rest.is_empty() {
            return Err(format!("git_url '{git_url}' has no host"));
        }
        Ok(git_url.to_string())
    } else if git_url.starts_with("http://") {
        Err(format!("git_url '{git_url}' uses insecure http; refusing"))
    } else {
        Ok(format!("https://{git_url}"))
    }
}

/// Lowercase-hex sha256 of `bytes`.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

async fn cmd_remove(app_id: String) {
    use commands::install_client as client;

    let conn = match client::connect().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{} {e}", "error:".red().bold());
            std::process::exit(1);
        }
    };

    if let Err(e) = client::uninstall_routed(&conn, &app_id).await {
        eprintln!("{} {e}", "error:".red().bold());
        std::process::exit(1);
    }
}

async fn cmd_list(json: bool) {
    use commands::install_client as client;

    let conn = match client::connect().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{} {e}", "error:".red().bold());
            std::process::exit(1);
        }
    };

    if let Err(e) = client::list_installed(&conn, json).await {
        eprintln!("{} {e}", "error:".red().bold());
        std::process::exit(1);
    }
}

async fn cmd_info(app_id: String, json: bool) {
    if let Err(e) = commands::install_client::info_app(&app_id, json).await {
        eprintln!("{} {e}", "error:".red().bold());
        std::process::exit(1);
    }
}

async fn cmd_which(app_id: String, json: bool) {
    if let Err(e) = commands::install_client::which_app(&app_id, json).await {
        eprintln!("{} {e}", "error:".red().bold());
        std::process::exit(1);
    }
}

async fn cmd_trash(action: TrashAction) {
    use commands::install_client as client;

    match action {
        TrashAction::List { json } => {
            let conn = match client::connect().await {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("{} {e}", "error:".red().bold());
                    std::process::exit(1);
                }
            };
            if let Err(e) = client::list_trashed(&conn, json).await {
                eprintln!("{} {e}", "error:".red().bold());
                std::process::exit(1);
            }
        }
        TrashAction::Restore { app_id } => {
            let conn = match client::connect().await {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("{} {e}", "error:".red().bold());
                    std::process::exit(1);
                }
            };
            if let Err(e) = client::restore_app(&conn, &app_id).await {
                eprintln!("{} {e}", "error:".red().bold());
                std::process::exit(1);
            }
        }
        TrashAction::Cleanup => {
            let conn = match client::connect().await {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("{} {e}", "error:".red().bold());
                    std::process::exit(1);
                }
            };
            if let Err(e) = client::cleanup_trash(&conn).await {
                eprintln!("{} {e}", "error:".red().bold());
                std::process::exit(1);
            }
        }
    }
}
