//! Recipe subcommands: init, new, validate.
//!
//! These wire the `arlen-forage-recipe` schema crate into the CLI: `init` and
//! `new` scaffold a valid starting `recipe.toml`, and `validate` parses and
//! checks one against the section 5a schema.

use std::path::{Path, PathBuf};
use std::process::exit;

use arlen_forage_recipe::{is_valid_id, lint, parse, validate as validate_schema};
use colored::Colorize;

/// Resolve a recipe path: a directory resolves to its `recipe.toml`, a file is
/// used as-is. Returns `None` (after printing an error) when nothing is found.
pub fn resolve_recipe_path(path: &Path) -> Option<PathBuf> {
    let candidate = if path.is_dir() {
        path.join("recipe.toml")
    } else {
        path.to_path_buf()
    };
    if candidate.exists() {
        Some(candidate)
    } else {
        eprintln!("{} no recipe found at {}", "error:".red(), candidate.display());
        None
    }
}

/// Scaffold a `recipe.toml` in the current directory.
pub fn init(force: bool) {
    write_template(Path::new("recipe.toml"), "org.example.app", "App", force);
}

/// Scaffold a `recipe.toml` for a reverse-DNS id in a destination directory.
pub fn new(id: &str, dir: Option<&Path>) {
    // Validate the id before deriving any path from it: a reverse-DNS id has
    // only `[A-Za-z0-9_-]` segments, so the derived name cannot contain a
    // separator, `..`, or an absolute path.
    if !is_valid_id(id) {
        eprintln!(
            "{} id '{id}' must be reverse-DNS notation (e.g. org.example.app)",
            "error:".red()
        );
        exit(1);
    }
    let name = id.rsplit('.').next().unwrap_or(id);
    let dest_dir = match dir {
        Some(d) => d.to_path_buf(),
        None => PathBuf::from(name),
    };
    if let Err(e) = std::fs::create_dir_all(&dest_dir) {
        eprintln!("{} could not create {}: {e}", "error:".red(), dest_dir.display());
        exit(1);
    }
    write_template(&dest_dir.join("recipe.toml"), id, name, false);
}

fn write_template(path: &Path, id: &str, name: &str, force: bool) {
    if path.exists() && !force {
        eprintln!(
            "{} {} already exists (use --force to overwrite)",
            "error:".red(),
            path.display()
        );
        exit(1);
    }
    let body = template(id, name);
    if let Err(e) = std::fs::write(path, &body) {
        eprintln!("{} could not write {}: {e}", "error:".red(), path.display());
        exit(1);
    }
    println!("{} {}", "created".green().bold(), path.display());
    println!("  edit the source pin and capabilities, then `forage build`.");
}

/// Parse and validate a `recipe.toml`, printing fatal errors and warnings.
pub fn validate(path: &Path) {
    let Some(recipe_path) = resolve_recipe_path(path) else {
        exit(1);
    };
    let content = match std::fs::read_to_string(&recipe_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{} could not read {}: {e}", "error:".red(), recipe_path.display());
            exit(1);
        }
    };

    let recipe = match parse(&content) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{} {e}", "parse error:".red().bold());
            exit(1);
        }
    };

    let errors = validate_schema(&recipe);
    let warnings = lint(&recipe);

    for w in &warnings {
        println!("{} {}: {}", "warning:".yellow(), w.field, w.message);
    }
    for e in &errors {
        eprintln!("{} {}: {}", "error:".red(), e.field, e.message);
    }

    if errors.is_empty() {
        println!(
            "{} {} ({} warning{})",
            "valid".green().bold(),
            recipe.recipe.id,
            warnings.len(),
            if warnings.len() == 1 { "" } else { "s" }
        );
    } else {
        eprintln!(
            "{} {} fatal error{}",
            "invalid:".red().bold(),
            errors.len(),
            if errors.len() == 1 { "" } else { "s" }
        );
        exit(1);
    }
}

/// A commented, schema-valid starting recipe for the given id and name.
fn template(id: &str, name: &str) -> String {
    format!(
        r#"# forage recipe - see docs/architecture/forage-recipes.md
[recipe]
id = "{id}"
name = "{name}"
version = "0.1.0"
summary = "One-line description"
license = "MIT"
maintainer = "key:REPLACE_ME"

# Where the code comes from. Replace the placeholder below with a full commit
# SHA, never a branch or tag. `forage recipe validate` flags it until you do.
[[source]]
type = "git"
url = "https://github.com/example/{name}"
commit = "0000000000000000000000000000000000000000"

# How it is built (the system is often autodetected).
[build]
system = "cargo"

# What is collected into the package; anything undeclared is discarded.
[artifacts]
bin = ["{name}"]

# Declared runtime capabilities, reviewable before the build.
[capabilities]
filesystem = []
network = []
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaffolded_template_flags_only_the_unset_pin() {
        // The template is complete except the source commit, which is a null
        // placeholder the user must pin. `recipe validate` should guide them to
        // exactly that and nothing else.
        let body = template("org.example.app", "app");
        let recipe = parse(&body).expect("template parses");
        let errors = validate_schema(&recipe);
        assert_eq!(errors.len(), 1, "only the unset pin is flagged: {errors:?}");
        assert_eq!(errors[0].field, "source[0].commit");
    }
}
