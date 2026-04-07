/// Module subcommands: register, list, info, remove, enable, disable.

use std::path::{Path, PathBuf};

use colored::Colorize;
use lunaris_modules::{load_manifest, parse_manifest, validate_manifest};

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

fn user_modules_dir() -> PathBuf {
    let data = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|_| PathBuf::from("/tmp"));
    data.join("lunaris/modules")
}

fn system_modules_dir() -> PathBuf {
    PathBuf::from("/usr/share/lunaris/modules")
}

fn modules_config_path() -> PathBuf {
    let config = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|_| PathBuf::from("/tmp"));
    config.join("lunaris/modules.toml")
}

/// Read the disabled list from modules.toml.
fn read_disabled() -> Vec<String> {
    let path = modules_config_path();
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    #[derive(serde::Deserialize, Default)]
    struct Config {
        #[serde(default)]
        disabled: Disabled,
    }
    #[derive(serde::Deserialize, Default)]
    struct Disabled {
        #[serde(default)]
        modules: Vec<String>,
    }

    toml::from_str::<Config>(&content)
        .map(|c| c.disabled.modules)
        .unwrap_or_default()
}

/// Write the disabled list to modules.toml.
fn write_disabled(modules: &[String]) {
    let path = modules_config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let content = format!(
        "[disabled]\nmodules = [{}]\n",
        modules
            .iter()
            .map(|m| format!("\"{m}\""))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let _ = std::fs::write(&path, content);
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Register a module from a local directory.
pub fn register(path: &Path, force: bool) {
    // 1. Check manifest exists.
    let manifest_path = path.join("manifest.toml");
    if !manifest_path.exists() {
        eprintln!(
            "{} manifest.toml not found in {}",
            "error:".red().bold(),
            path.display()
        );
        std::process::exit(1);
    }

    // 2. Parse and validate.
    let manifest = match load_manifest(&manifest_path) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{} {e}", "error:".red().bold());
            std::process::exit(1);
        }
    };

    let warnings = validate_manifest(&manifest);
    for w in &warnings {
        eprintln!(
            "{} {}: {}",
            "warning:".yellow().bold(),
            w.field,
            w.message
        );
    }

    let id = &manifest.module.id;
    let dest = user_modules_dir().join(id);

    // 3. Check for existing.
    if dest.exists() {
        if !force {
            eprintln!(
                "{} module {} already installed at {}. Use --force to overwrite.",
                "error:".red().bold(),
                id,
                dest.display()
            );
            std::process::exit(1);
        }
        std::fs::remove_dir_all(&dest).unwrap_or_else(|e| {
            eprintln!("{} could not remove existing: {e}", "error:".red().bold());
            std::process::exit(1);
        });
    }

    // Check system module.
    let sys = system_modules_dir().join(id);
    if sys.exists() {
        eprintln!(
            "{} {} is a system module and cannot be overridden",
            "error:".red().bold(),
            id
        );
        std::process::exit(1);
    }

    // 4. Copy directory.
    copy_dir_recursive(path, &dest).unwrap_or_else(|e| {
        eprintln!("{} copy failed: {e}", "error:".red().bold());
        std::process::exit(1);
    });

    println!(
        "{} {} v{} registered at {}",
        "ok:".green().bold(),
        manifest.module.name,
        manifest.module.version,
        dest.display()
    );
}

/// List all installed modules.
pub fn list() {
    let disabled = read_disabled();

    println!(
        "{:<35} {:<20} {:<10} {:<12} {:<8} {}",
        "ID".bold(),
        "NAME".bold(),
        "VERSION".bold(),
        "TYPE".bold(),
        "SOURCE".bold(),
        "ENABLED".bold()
    );

    let mut found = false;

    for (dir, source) in [
        (system_modules_dir(), "system"),
        (user_modules_dir(), "user"),
    ] {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let manifest_path = path.join("manifest.toml");
            if !manifest_path.exists() {
                continue;
            }
            let content = match std::fs::read_to_string(&manifest_path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let manifest = match parse_manifest(&content) {
                Ok(m) => m,
                Err(_) => continue,
            };

            let id = &manifest.module.id;
            let enabled = !disabled.contains(id);
            let enabled_str = if enabled {
                "yes".green().to_string()
            } else {
                "no".red().to_string()
            };

            println!(
                "{:<35} {:<20} {:<10} {:<12} {:<8} {}",
                id,
                manifest.module.name,
                manifest.module.version,
                format!("{:?}", manifest.module.module_type).to_lowercase(),
                source,
                enabled_str
            );
            found = true;
        }
    }

    if !found {
        println!("{}", "No modules installed.".dimmed());
    }
}

/// Show details for a module.
pub fn info(id: &str) {
    let (path, source) = find_module(id);
    let manifest_path = path.join("manifest.toml");
    let content = std::fs::read_to_string(&manifest_path).unwrap_or_else(|e| {
        eprintln!("{} {e}", "error:".red().bold());
        std::process::exit(1);
    });
    let manifest = parse_manifest(&content).unwrap_or_else(|e| {
        eprintln!("{} {e}", "error:".red().bold());
        std::process::exit(1);
    });

    let disabled = read_disabled();
    let enabled = !disabled.contains(&id.to_string());

    println!("{}: {}", "ID".bold(), manifest.module.id);
    println!("{}: {}", "Name".bold(), manifest.module.name);
    println!("{}: {}", "Version".bold(), manifest.module.version);
    println!("{}: {:?}", "Type".bold(), manifest.module.module_type);
    println!("{}: {}", "Source".bold(), source);
    println!("{}: {}", "Path".bold(), path.display());
    println!(
        "{}: {}",
        "Enabled".bold(),
        if enabled { "yes".green() } else { "no".red() }
    );
    if !manifest.module.description.is_empty() {
        println!("{}: {}", "Description".bold(), manifest.module.description);
    }
    if manifest.waypointer.is_some() {
        println!("{}: waypointer", "Extensions".bold());
    }
    if manifest.topbar.is_some() {
        println!("{}: topbar", "Extensions".bold());
    }
    if manifest.settings.is_some() {
        println!("{}: settings", "Extensions".bold());
    }
}

/// Remove a user module.
pub fn remove(id: &str) {
    let user_path = user_modules_dir().join(id);
    if !user_path.exists() {
        eprintln!(
            "{} module {} not found in user modules",
            "error:".red().bold(),
            id
        );
        std::process::exit(1);
    }

    std::fs::remove_dir_all(&user_path).unwrap_or_else(|e| {
        eprintln!("{} {e}", "error:".red().bold());
        std::process::exit(1);
    });

    println!("{} removed {}", "ok:".green().bold(), id);
}

/// Enable a module (remove from disabled list).
pub fn enable(id: &str) {
    let mut disabled = read_disabled();
    disabled.retain(|m| m != id);
    write_disabled(&disabled);
    println!("{} enabled {}", "ok:".green().bold(), id);
}

/// Disable a module (add to disabled list).
pub fn disable(id: &str) {
    let mut disabled = read_disabled();
    if !disabled.contains(&id.to_string()) {
        disabled.push(id.to_string());
    }
    write_disabled(&disabled);
    println!("{} disabled {}", "ok:".green().bold(), id);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Find a module by ID (user first, then system).
fn find_module(id: &str) -> (PathBuf, &'static str) {
    let user = user_modules_dir().join(id);
    if user.exists() {
        return (user, "user");
    }
    let sys = system_modules_dir().join(id);
    if sys.exists() {
        return (sys, "system");
    }
    eprintln!("{} module {} not found", "error:".red().bold(), id);
    std::process::exit(1);
}

/// Recursively copy a directory.
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}
