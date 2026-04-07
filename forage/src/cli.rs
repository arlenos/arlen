/// CLI argument definitions via clap derive.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "forage", about = "Lunaris OS package manager", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Manage Lunaris modules.
    Module {
        #[command(subcommand)]
        action: ModuleAction,
    },
}

#[derive(Subcommand)]
pub enum ModuleAction {
    /// Register a module from a local directory.
    Register {
        /// Path to the module directory (must contain manifest.toml).
        path: PathBuf,
        /// Overwrite if a user module with the same ID already exists.
        #[arg(short, long)]
        force: bool,
    },
    /// List all installed modules.
    List,
    /// Show details for a module.
    Info {
        /// Module ID (e.g. com.example.calculator).
        id: String,
    },
    /// Remove a user-installed module.
    Remove {
        /// Module ID.
        id: String,
    },
    /// Enable a module.
    Enable {
        /// Module ID.
        id: String,
    },
    /// Disable a module.
    Disable {
        /// Module ID.
        id: String,
    },
}
