/// forage - Lunaris OS package manager CLI.
///
/// See `docs/architecture/module-system.md` and
/// `docs/architecture/distro-package-management.md`.

mod cli;
mod commands;

use clap::Parser;
use cli::{Cli, Commands, ModuleAction, TrashAction};
use colored::Colorize;

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Install { target } => run_async(cmd_install(target)),
        Commands::Remove { app_id } => run_async(cmd_remove(app_id)),
        Commands::List => run_async(cmd_list()),
        Commands::Info { app_id } => run_async(cmd_info(app_id)),
        Commands::Which { app_id } => run_async(cmd_which(app_id)),
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

    let conn = match client::connect().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{} {e}", "error:".red().bold());
            eprintln!(
                "{}",
                "is lunaris-installd running? (systemctl --user start installd)"
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
            "{} cannot resolve '{}'. Expected a .lunpkg file, URL, or flatpak:{{app_id}}",
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

async fn cmd_remove(app_id: String) {
    use commands::install_client as client;

    let conn = match client::connect().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{} {e}", "error:".red().bold());
            std::process::exit(1);
        }
    };

    if let Err(e) = client::uninstall(&conn, &app_id).await {
        eprintln!("{} {e}", "error:".red().bold());
        std::process::exit(1);
    }
}

async fn cmd_list() {
    use commands::install_client as client;

    let conn = match client::connect().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{} {e}", "error:".red().bold());
            std::process::exit(1);
        }
    };

    if let Err(e) = client::list_installed(&conn).await {
        eprintln!("{} {e}", "error:".red().bold());
        std::process::exit(1);
    }
}

async fn cmd_info(app_id: String) {
    if let Err(e) = commands::install_client::info_app(&app_id).await {
        eprintln!("{} {e}", "error:".red().bold());
        std::process::exit(1);
    }
}

async fn cmd_which(app_id: String) {
    if let Err(e) = commands::install_client::which_app(&app_id).await {
        eprintln!("{} {e}", "error:".red().bold());
        std::process::exit(1);
    }
}

async fn cmd_trash(action: TrashAction) {
    use commands::install_client as client;

    match action {
        TrashAction::List => {
            let conn = match client::connect().await {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("{} {e}", "error:".red().bold());
                    std::process::exit(1);
                }
            };
            if let Err(e) = client::list_trashed(&conn).await {
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
