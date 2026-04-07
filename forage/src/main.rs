/// forage - Lunaris OS package manager CLI.
///
/// See `docs/architecture/module-system.md` and
/// `docs/architecture/distro-package-management.md`.

mod cli;
mod commands;

use clap::Parser;
use cli::{Cli, Commands, ModuleAction};

fn main() {
    let cli = Cli::parse();

    match cli.command {
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
