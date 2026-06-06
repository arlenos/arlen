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
    /// Install a package, URL, or Flatpak app.
    Install {
        /// Package path (.lunpkg), URL, or flatpak:{app_id}.
        target: String,
    },
    /// Remove an installed app (staged deletion with 30-day grace period).
    Remove {
        /// App ID (e.g. com.example.app).
        app_id: String,
    },
    /// List all installed apps (lunpkg + flatpak).
    List,
    /// Show details for an installed app.
    Info {
        /// App ID.
        app_id: String,
    },
    /// Show the install path of an app.
    Which {
        /// App ID.
        app_id: String,
    },
    /// Manage the 30-day trash.
    Trash {
        #[command(subcommand)]
        action: TrashAction,
    },
    /// Manage Lunaris modules.
    Module {
        #[command(subcommand)]
        action: ModuleAction,
    },
}

#[derive(Subcommand)]
pub enum TrashAction {
    /// List apps in the 30-day trash.
    List,
    /// Restore an app from trash.
    Restore {
        /// App ID.
        app_id: String,
    },
    /// Permanently delete expired trash entries.
    Cleanup,
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
