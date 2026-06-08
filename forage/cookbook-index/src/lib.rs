//! The forage cookbook manifest: `cookbook.toml`, the human-authored index a
//! maintainer edits, and the payload `forage cookbook sign` later compiles into
//! canonical TUF `targets` metadata (decision D8, forage-recipes.md §7a).
//!
//! A cookbook points at recipes rather than copying them: each entry names a
//! recipe, the git repo + pinned commit it lives at, the recipe's content hash,
//! and an optional **capability cap** (a curated upper bound the cookbook
//! enforces against the in-repo recipe's declared capabilities, so a cookbook
//! adds vetting without forking). This crate is only the human-authored TOML
//! schema (parse + validate). The TUF signing and signature verification are
//! done by a real TUF library (`tough`), never hand-rolled crypto, and the
//! layered resolution that consumes a verified index are the following slices.

use arlen_forage_recipe::{is_valid_id, Capabilities};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// One entry in a cookbook's `targets` payload: a recipe pointer (§7a).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeEntry {
    /// The recipe's reverse-DNS name (e.g. `dev.zed.Zed`).
    pub name: String,
    /// The git repository the recipe lives in (`github.com/{owner}/{repo}` or
    /// another git host the fetch allows).
    pub git_url: String,
    /// The pinned commit the recipe is taken from (a full object id, never a
    /// floating ref).
    pub commit: String,
    /// The recipe's content hash (sha256), so the resolved recipe is verified
    /// against what the cookbook signed.
    pub recipe_hash: String,
    /// The curated capability upper bound: the in-repo recipe may declare at
    /// most these capabilities. Omitted means the cookbook sets no cap (the
    /// recipe's own declaration stands).
    #[serde(default)]
    pub capability_cap: Option<Capabilities>,
}

/// A cookbook manifest (`cookbook.toml`): the recipes this cookbook indexes.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CookbookManifest {
    /// The indexed recipes (the `targets` payload).
    #[serde(default)]
    pub recipe: Vec<RecipeEntry>,
}

/// A validation violation, naming the field it concerns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    /// The dotted field path (e.g. `recipe[0].commit`).
    pub field: String,
    /// What is wrong.
    pub message: String,
}

/// A failure parsing a cookbook manifest.
#[derive(Debug, Error)]
pub enum ParseError {
    /// The TOML was malformed or had the wrong shape.
    #[error("invalid cookbook.toml: {0}")]
    Toml(#[from] toml::de::Error),
}

/// Parse a `cookbook.toml` manifest from its text.
pub fn parse(text: &str) -> Result<CookbookManifest, ParseError> {
    Ok(toml::from_str(text)?)
}

/// Validate a parsed manifest, returning every fatal violation. An empty list
/// means the manifest is well-formed (signature verification is separate, done
/// by the TUF layer).
pub fn validate(manifest: &CookbookManifest) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    if manifest.recipe.is_empty() {
        errors.push(err("recipe", "a cookbook must index at least one recipe"));
    }
    for (i, entry) in manifest.recipe.iter().enumerate() {
        let at = |field: &str| format!("recipe[{i}].{field}");
        if !is_valid_id(&entry.name) {
            errors.push(err(
                &at("name"),
                "recipe name must be reverse-DNS notation (e.g. dev.zed.Zed)",
            ));
        }
        if !is_github_or_git_url(&entry.git_url) {
            errors.push(err(&at("git_url"), "git_url must be a host/owner/repo url"));
        }
        if !is_full_hex(&entry.commit, &[40, 64]) || is_all_zeros(&entry.commit) {
            errors.push(err(
                &at("commit"),
                "commit must be a full git object id (40 or 64 hex), not a floating ref",
            ));
        }
        if !is_full_hex(&entry.recipe_hash, &[64]) {
            errors.push(err(
                &at("recipe_hash"),
                "recipe_hash must be a 64-character hex sha256",
            ));
        }
    }
    errors
}

fn err(field: &str, message: &str) -> ValidationError {
    ValidationError {
        field: field.to_string(),
        message: message.to_string(),
    }
}

/// Whether `s` is all-hex and one of the allowed lengths.
fn is_full_hex(s: &str, lengths: &[usize]) -> bool {
    lengths.contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Whether a hex id is all zeros (the unset/null sentinel, never a real pin).
fn is_all_zeros(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b == b'0')
}

/// A loose host/owner/repo url check (a real host, two path segments). The
/// fetch resolves and pins the host; this is validate-time feedback.
fn is_github_or_git_url(url: &str) -> bool {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    let Some((host, path)) = rest.split_once('/') else {
        return false;
    };
    if host.is_empty() || !host.contains('.') {
        return false;
    }
    let mut segs = path.trim_end_matches('/').split('/').filter(|s| !s.is_empty());
    segs.next().is_some() && segs.next().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"
[[recipe]]
name = "dev.zed.Zed"
git_url = "github.com/zed-industries/zed"
commit = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
recipe_hash = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
"#;

    #[test]
    fn a_well_formed_manifest_validates() {
        let m = parse(VALID).unwrap();
        assert_eq!(m.recipe.len(), 1);
        assert!(validate(&m).is_empty(), "{:?}", validate(&m));
    }

    #[test]
    fn a_capability_cap_round_trips() {
        let toml = format!(
            "{VALID}capability_cap = {{ network = [\"api.example.com:443\"], filesystem = [\"home\"] }}\n"
        );
        let m = parse(&toml).unwrap();
        let cap = m.recipe[0].capability_cap.as_ref().expect("cap present");
        assert_eq!(cap.network, vec!["api.example.com:443".to_string()]);
        assert_eq!(cap.filesystem, vec!["home".to_string()]);
        assert!(validate(&m).is_empty());
    }

    #[test]
    fn empty_manifest_is_rejected() {
        assert!(validate(&CookbookManifest::default())
            .iter()
            .any(|e| e.field == "recipe"));
    }

    #[test]
    fn a_floating_or_null_commit_is_fatal() {
        let toml = r#"
[[recipe]]
name = "dev.zed.Zed"
git_url = "github.com/zed-industries/zed"
commit = "main"
recipe_hash = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
"#;
        assert!(validate(&parse(toml).unwrap())
            .iter()
            .any(|e| e.field == "recipe[0].commit"));

        let null = VALID.replace(
            "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
            "0000000000000000000000000000000000000000",
        );
        assert!(validate(&parse(&null).unwrap())
            .iter()
            .any(|e| e.field == "recipe[0].commit"));
    }

    #[test]
    fn a_bad_name_or_hash_is_fatal() {
        let toml = r#"
[[recipe]]
name = "not-reverse-dns"
git_url = "github.com/o/r"
commit = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
recipe_hash = "tooshort"
"#;
        let errors = validate(&parse(toml).unwrap());
        assert!(errors.iter().any(|e| e.field == "recipe[0].name"));
        assert!(errors.iter().any(|e| e.field == "recipe[0].recipe_hash"));
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let toml = format!("{VALID}surprise = true\n");
        assert!(parse(&toml).is_err(), "deny_unknown_fields catches typos");
    }
}
