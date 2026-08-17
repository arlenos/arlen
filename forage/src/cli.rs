//! CLI argument definitions via clap derive.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "forage", about = "Arlen OS package manager", version)]
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
    /// Update an installed app to what its cookbook now offers. Explicit and
    /// foreground: nothing here runs on a timer.
    Update {
        /// The app's reverse-DNS name, as its cookbook lists it.
        name: String,
        /// Apply without asking, even when the update wants something new.
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Remove an installed app (staged deletion with 30-day grace period).
    Remove {
        /// App ID (e.g. com.example.app).
        app_id: String,
    },
    /// List all installed apps (lunpkg + flatpak).
    List {
        /// Output a JSON array instead of the formatted table (for scripts
        /// and other tools, e.g. the store app).
        #[arg(long)]
        json: bool,
    },
    /// Show details for an installed app.
    Info {
        /// App ID.
        app_id: String,
        /// Output a JSON object instead of the formatted details (for scripts
        /// and other tools, e.g. the store app's detail view).
        #[arg(long)]
        json: bool,
    },
    /// Show the install path of an app.
    Which {
        /// App ID.
        app_id: String,
        /// Output a JSON object instead of the bare path (for scripts and
        /// other tools, e.g. the store app).
        #[arg(long)]
        json: bool,
    },
    /// Manage the 30-day trash.
    Trash {
        #[command(subcommand)]
        action: TrashAction,
    },
    /// Manage Arlen modules.
    Module {
        #[command(subcommand)]
        action: ModuleAction,
    },
    /// Work with `recipe.toml` build recipes.
    Recipe {
        #[command(subcommand)]
        action: RecipeAction,
    },
    // Design: forage-recipes.md R1 (build pipeline). Kept out of the `///`
    // below because clap prints that to whoever runs `forage --help`, and the
    // docs repo is private - a citation they cannot follow is noise at best.
    /// Build a recipe into an installable `.lunpkg`.
    Build {
        /// Path to the recipe or a directory containing `recipe.toml`
        /// (defaults to the current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Run the build directly on the host instead of inside the sandbox.
        /// This executes the recipe's untrusted build steps unconfined, so it is
        /// for a maintainer testing their own recipe locally, never for building
        /// untrusted recipes. Without it the build runs in the pinned base
        /// platform, which must be configured.
        #[arg(long = "unsafe-no-sandbox")]
        unsafe_no_sandbox: bool,
        /// After a successful build, install the produced `.lunpkg` through
        /// installd (the same path as `forage install <file>`).
        #[arg(long)]
        install: bool,
    },
    // Design: forage-recipes.md section 7.
    /// Manage cookbooks: the recipe indexes you install from.
    Cookbook {
        #[command(subcommand)]
        action: CookbookAction,
    },
    // Design: forage-recipes.md section 8a.
    /// Rebuild a package independently and check it comes out byte-identical.
    Challenge {
        /// App ID or recipe id to challenge.
        target: String,
    },
    // Design: foreign-app-bridges.md section 4.
    /// Install a community bridge so a foreign app can feed your knowledge graph.
    Bridge {
        #[command(subcommand)]
        action: BridgeAction,
    },
}

#[derive(Subcommand)]
pub enum BridgeAction {
    /// Install every cookbook bridge tagged for a foreign app, granting each a
    /// revocable KG write scope.
    Install {
        /// The foreign app's short id (e.g. `obsidian`), matched against each
        /// bridge recipe's `[bridge] foreign_app`.
        foreign_app: String,
        /// Skip the interactive consent prompt (for scripted installs).
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
pub enum RecipeAction {
    /// Scaffold a `recipe.toml` in the current directory.
    Init {
        /// Overwrite an existing `recipe.toml`.
        #[arg(short, long)]
        force: bool,
    },
    /// Scaffold a `recipe.toml` for a given reverse-DNS id.
    New {
        /// Reverse-DNS id, e.g. `org.example.hello`.
        id: String,
        /// Destination directory (defaults to the id's last segment).
        #[arg(short, long)]
        dir: Option<PathBuf>,
    },
    /// Parse and validate a `recipe.toml`, reporting errors and warnings.
    Validate {
        /// Path to the recipe or a directory containing `recipe.toml`
        /// (defaults to the current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

#[derive(Subcommand)]
pub enum CookbookAction {
    /// Add a cookbook by name and git URL.
    Add {
        /// Local name for the cookbook.
        name: String,
        /// Git URL of the cookbook index.
        url: String,
    },
    /// Remove a cookbook by name.
    Remove {
        /// Local name of the cookbook.
        name: String,
    },
    /// List configured cookbooks.
    List,
    /// Update cookbook indexes from their remotes.
    Update,
}

#[derive(Subcommand)]
pub enum TrashAction {
    /// List apps in the 30-day trash.
    List {
        /// Output a JSON array instead of the formatted table (for scripts
        /// and other tools, e.g. the store app's trash view).
        #[arg(long)]
        json: bool,
    },
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
