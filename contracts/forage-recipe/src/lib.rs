//! Parser and validator for the forage `recipe.toml` format.
//!
//! This crate owns the normative schema described in `forage-recipes.md`
//! section 5a: a declarative, capability-declaring build recipe whose output is
//! a `.lunpkg`. It is the single source of truth for the recipe shape, shared by
//! the forage CLI, the build pipeline and `installd`.
//!
//! The idiom mirrors `arlen-modules`: [`parse`] turns a TOML string into a
//! [`Recipe`], [`load`] reads and validates a file, [`validate`] returns the
//! fatal schema violations (an empty list means the recipe is structurally
//! sound) and [`lint`] returns non-fatal recommendations.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors and diagnostics
// ---------------------------------------------------------------------------

/// A failure while reading, parsing or validating a recipe.
#[derive(Debug, Error)]
pub enum RecipeError {
    /// The recipe file could not be read.
    #[error("IO: {0}")]
    Io(#[from] std::io::Error),
    /// The TOML did not parse against the schema.
    #[error("parse: {0}")]
    Parse(String),
    /// The recipe parsed but failed fatal validation.
    #[error("validation: {0}")]
    Validation(String),
}

/// A fatal schema violation: the recipe cannot be built as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    /// The dotted field path the violation concerns (e.g. `source[0].commit`).
    pub field: String,
    /// A human-readable description of what is wrong.
    pub message: String,
}

/// A non-fatal recommendation: the recipe builds but could be improved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationWarning {
    /// The dotted field path the recommendation concerns.
    pub field: String,
    /// A human-readable description of the recommendation.
    pub message: String,
}

// ---------------------------------------------------------------------------
// Top-level recipe
// ---------------------------------------------------------------------------

/// A parsed `recipe.toml`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    /// Identity and metadata (`[recipe]`).
    pub recipe: RecipeMeta,
    /// Where the code comes from (`[[source]]`, also accepts a single
    /// `[source]` table).
    #[serde(default, deserialize_with = "one_or_many_source")]
    pub source: Vec<Source>,
    /// How the code is transformed (`[build]`).
    #[serde(default)]
    pub build: Option<Build>,
    /// What is collected into the `.lunpkg` (`[artifacts]`).
    #[serde(default)]
    pub artifacts: Option<Artifacts>,
    /// Declared runtime containment (`[capabilities]`).
    #[serde(default)]
    pub capabilities: Option<Capabilities>,
    /// What the package registers (`[provides]`).
    #[serde(default)]
    pub provides: Option<Provides>,
    /// Runtime and platform dependencies (`[depends]`).
    #[serde(default)]
    pub depends: Option<Depends>,
    /// Reproducibility status (`[reproducible]`).
    #[serde(default)]
    pub reproducible: Option<Reproducible>,
}

/// Identity and metadata (`[recipe]`).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeMeta {
    /// Reverse-DNS id, identical to the `.lunpkg` manifest id.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Semver version; omitted with `github-release` (which follows tags).
    #[serde(default)]
    pub version: Option<String>,
    /// One-line summary.
    #[serde(default)]
    pub summary: Option<String>,
    /// SPDX license expression.
    #[serde(default)]
    pub license: Option<String>,
    /// Project homepage.
    #[serde(default)]
    pub homepage: Option<String>,
    /// Key id that signs the recipe.
    pub maintainer: String,
    /// Bumps on a recipe change that carries no version bump.
    #[serde(default = "default_revision")]
    pub recipe_revision: u32,
    /// Store-browsing categories.
    #[serde(default)]
    pub category: Vec<String>,
}

/// Where a source artifact comes from (`[[source]]`).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    /// The kind of source.
    #[serde(rename = "type")]
    pub source_type: SourceType,
    /// Fetch URL (the host must be on the declared allowlist).
    #[serde(default)]
    pub url: Option<String>,
    /// Pinned git commit; floating refs are rejected.
    #[serde(default)]
    pub commit: Option<String>,
    /// Content-address for a tarball or release asset.
    #[serde(default)]
    pub sha256: Option<String>,
    /// Crate version for a `crate` source (e.g. `"1.2.3"`); when omitted it
    /// defaults to the recipe's own `version`, so a recipe that packages exactly
    /// one crate need not repeat it. Lets a recipe pin a vendored crate at a
    /// version independent of the package version (decision D6).
    #[serde(default)]
    pub version: Option<String>,
    /// `github-release` asset template, e.g. `{version}-{target}`.
    #[serde(default)]
    pub asset: Option<String>,
    /// Human label only, never the pin.
    #[serde(default)]
    pub tag: Option<String>,
    /// Patches applied deterministically after fetch.
    #[serde(default)]
    pub patches: Vec<PathBuf>,
}

/// The kind of a source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceType {
    /// A git repository pinned to a commit.
    Git,
    /// A tarball verified by sha256.
    Tarball,
    /// A GitHub release that follows tags.
    GithubRelease,
    /// A crates.io crate.
    Crate,
    /// A path local to the recipe.
    Local,
}

/// How the source is transformed into artifacts (`[build]`).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Build {
    /// Build system; autodetected when omitted.
    #[serde(default)]
    pub system: Option<BuildSystem>,
    /// Build-time host packages resolved from the pinned build-root.
    #[serde(default)]
    pub host_deps: Vec<String>,
    /// Flags passed to the build system (not shell).
    #[serde(default)]
    pub config_opts: Vec<String>,
    /// Deterministic environment variables; the rest is normalised away.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Declarative build steps, each a direct exec with no shell.
    #[serde(default)]
    pub steps: Vec<BuildStep>,
    /// Whether the build runs without network (the default).
    #[serde(default = "default_true")]
    pub offline: bool,
    /// Deterministic job count for reproducibility.
    #[serde(default)]
    pub jobs: Option<u32>,
    /// Record-then-replay fetch lock, required when `offline = false`.
    #[serde(default)]
    pub fetch_lock: Option<FetchLock>,
}

/// A build system the executor knows how to drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BuildSystem {
    /// Rust cargo.
    Cargo,
    /// CMake.
    Cmake,
    /// Meson.
    Meson,
    /// Plain make.
    Make,
    /// GNU autotools.
    Autotools,
    /// Go modules.
    Go,
    /// Python.
    Python,
    /// Zig.
    Zig,
    /// Nim.
    Nim,
    /// npm.
    Npm,
    /// pnpm.
    Pnpm,
    /// Custom: only `[[build.steps]]` drive the build.
    Custom,
}

/// One declarative build step: a direct exec, never a shell line.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BuildStep {
    /// The executable to run (resolved from the build-root, no shell).
    pub tool: String,
    /// Arguments passed verbatim.
    #[serde(default)]
    pub args: Vec<String>,
    /// Working directory relative to the source root.
    #[serde(default)]
    pub workdir: Option<String>,
}

/// A generated record-then-replay fetch lock (`[build.fetch_lock]`).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FetchLock {
    /// Everything fetched during the one online lock run.
    #[serde(default)]
    pub entries: Vec<FetchLockEntry>,
}

/// One entry in a fetch lock.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FetchLockEntry {
    /// The fetched URL.
    pub url: String,
    /// The content-address of what was fetched.
    pub sha256: String,
}

/// What is collected into the `.lunpkg`; anything undeclared is discarded
/// (`[artifacts]`).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Artifacts {
    /// Installed binaries.
    #[serde(default)]
    pub bin: Vec<PathBuf>,
    /// Installed libraries.
    #[serde(default)]
    pub lib: Vec<PathBuf>,
    /// Installed headers.
    #[serde(default)]
    pub include: Vec<PathBuf>,
    /// Installed shared data.
    #[serde(default)]
    pub share: Vec<PathBuf>,
    /// Installed internal helper binaries.
    #[serde(default)]
    pub libexec: Vec<PathBuf>,
    /// The `.desktop` entry.
    #[serde(default)]
    pub desktop: Option<PathBuf>,
    /// The application icon.
    #[serde(default)]
    pub icon: Option<PathBuf>,
}

/// Declared runtime containment, enforced by the permission model
/// (`[capabilities]`).
///
/// The named categories below mirror the permission system. The schema is
/// deliberately open (the design lists these "and so on"): unrecognised
/// categories are captured in [`Capabilities::extra`] rather than rejected or
/// silently dropped, so the capability model can map them later without losing
/// declared permissions.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Capabilities {
    /// Filesystem scopes the app may access.
    #[serde(default)]
    pub filesystem: Vec<String>,
    /// Network destinations as `host:port`.
    #[serde(default)]
    pub network: Vec<String>,
    /// Knowledge-graph scopes as prefixed strings, e.g. `read:File`,
    /// `write:Tag` (matches `forage-recipes.md` section 5).
    #[serde(default)]
    pub graph: Vec<String>,
    /// Whether the app may post notifications.
    #[serde(default)]
    pub notifications: bool,
    /// Whether the app may access the clipboard.
    #[serde(default)]
    pub clipboard: bool,
    /// Whether the app may access audio.
    #[serde(default)]
    pub audio: bool,
    /// Any further capability categories the design lists as "and so on",
    /// preserved verbatim for the capability mapper rather than dropped.
    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

/// What the package registers with the system (`[provides]`).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Provides {
    /// Knowledge-graph entity types.
    #[serde(default)]
    pub schemas: Vec<String>,
    /// Command names.
    #[serde(default)]
    pub binaries: Vec<String>,
    /// MIME handlers.
    #[serde(default)]
    pub mime: Vec<String>,
}

/// Runtime and platform dependencies (`[depends]`).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Depends {
    /// A named, versioned platform recipe to build and run against.
    #[serde(default)]
    pub platform: Option<String>,
    /// Runtime dependencies resolved by forage/installd, otherwise bundled.
    #[serde(default)]
    pub runtime: Vec<String>,
}

/// Reproducibility status (`[reproducible]`, mostly system-managed).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Reproducible {
    /// The current reproducibility status.
    #[serde(default)]
    pub status: ReproducibleStatus,
    /// An author-asserted expected output content-address.
    #[serde(default)]
    pub expected_output: Option<String>,
}

/// The reproducibility status of a recipe's output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReproducibleStatus {
    /// Not yet checked.
    #[default]
    Unverified,
    /// An expected output is asserted but unconfirmed.
    Expected,
    /// Independently reproduced.
    Verified,
    /// Known not to reproduce bit-for-bit.
    Unreproducible,
}

// ---------------------------------------------------------------------------
// serde helpers
// ---------------------------------------------------------------------------

fn default_true() -> bool {
    true
}

fn default_revision() -> u32 {
    1
}

/// Accept either a single `[source]` table or repeated `[[source]]` tables.
fn one_or_many_source<'de, D>(deserializer: D) -> Result<Vec<Source>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(Source),
        Many(Vec<Source>),
    }

    Ok(match OneOrMany::deserialize(deserializer)? {
        OneOrMany::One(s) => vec![s],
        OneOrMany::Many(v) => v,
    })
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Parse a recipe from a TOML string without validating it.
pub fn parse(toml_str: &str) -> Result<Recipe, RecipeError> {
    toml::from_str(toml_str).map_err(|e| RecipeError::Parse(e.to_string()))
}

/// Read, parse and fatally validate a recipe from a file.
///
/// Returns the recipe only when it parses and has no [`ValidationError`]s. Soft
/// recommendations are not consulted here; call [`lint`] for those.
pub fn load(path: &Path) -> Result<Recipe, RecipeError> {
    let content = std::fs::read_to_string(path)?;
    let recipe = parse(&content)?;
    let errors = validate(&recipe);
    if !errors.is_empty() {
        let joined = errors
            .iter()
            .map(|e| format!("{}: {}", e.field, e.message))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(RecipeError::Validation(joined));
    }
    Ok(recipe)
}

// ---------------------------------------------------------------------------
// Validation (fatal)
// ---------------------------------------------------------------------------

/// Return the fatal schema violations for a recipe.
///
/// An empty list means the recipe is structurally sound. Each error names the
/// offending field so the caller can report it precisely.
pub fn validate(recipe: &Recipe) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    if recipe.recipe.id.trim().is_empty() {
        errors.push(err("recipe.id", "must not be empty"));
    } else if !is_reverse_domain(&recipe.recipe.id) {
        // The id must equal the `.lunpkg` manifest id, which is reverse-DNS,
        // so a malformed id is a build-blocking contract violation, not a hint.
        errors.push(err(
            "recipe.id",
            "must be reverse-DNS notation (it is the .lunpkg manifest id)",
        ));
    }
    if recipe.recipe.name.trim().is_empty() {
        errors.push(err("recipe.name", "must not be empty"));
    }
    if recipe.recipe.maintainer.trim().is_empty() {
        errors.push(err("recipe.maintainer", "must not be empty"));
    }

    if recipe.source.is_empty() {
        errors.push(err("source", "at least one source is required"));
    }

    for (i, source) in recipe.source.iter().enumerate() {
        validate_source(source, i, &mut errors);
    }

    if let Some(build) = &recipe.build {
        validate_build(build, &mut errors);
    }

    errors
}

fn validate_source(source: &Source, i: usize, errors: &mut Vec<ValidationError>) {
    let at = |field: &str| format!("source[{i}].{field}");
    let present = |v: &Option<String>| v.as_ref().is_some_and(|s| !s.trim().is_empty());

    match source.source_type {
        SourceType::Git => {
            if !present(&source.url) {
                errors.push(err(&at("url"), "git source requires a url"));
            }
            match &source.commit {
                c if !present(c) => errors.push(err(
                    &at("commit"),
                    "git source must be pinned to a commit (no floating ref)",
                )),
                Some(c) if !is_git_commit(c) => errors.push(err(
                    &at("commit"),
                    "git commit must be a full object id (40 or 64 hex chars), not a branch or tag",
                )),
                Some(c) if is_null_oid(c) => errors.push(err(
                    &at("commit"),
                    "git commit is the null object id (an unset placeholder); pin a real commit",
                )),
                _ => {}
            }
        }
        SourceType::Tarball => {
            if !present(&source.url) {
                errors.push(err(&at("url"), "tarball source requires a url"));
            }
            match &source.sha256 {
                s if !present(s) => {
                    errors.push(err(&at("sha256"), "tarball source requires a sha256"))
                }
                Some(s) if !is_sha256(s) => errors.push(err(
                    &at("sha256"),
                    "sha256 must be a 64-character hex content-address",
                )),
                _ => {}
            }
        }
        SourceType::GithubRelease => {
            // A github-release follows tags (version is omitted, the asset is a
            // `{version}` template), so its content-address is resolved and
            // locked per release at fetch time, not pinned in the recipe. The
            // repo `url` is required and must be a `github.com/{owner}/{repo}`
            // url (decision D7): an explicit, host-checkable repo beats deriving
            // owner/repo from another field, and the fetch host-checks the same.
            match &source.url {
                u if !present(u) => errors.push(err(
                    &at("url"),
                    "github-release source requires a url (github.com/{owner}/{repo})",
                )),
                Some(u) if !is_github_repo_url(u) => errors.push(err(
                    &at("url"),
                    "github-release url must be a github.com/{owner}/{repo} repository",
                )),
                _ => {}
            }
            if !present(&source.asset) {
                errors.push(err(
                    &at("asset"),
                    "github-release source requires an asset template",
                ));
            }
        }
        SourceType::Crate => {
            // `url` is the bare crate name; `sha256` pins the downloaded `.crate`
            // (decision D6). `version` is optional (it defaults to the recipe
            // version), so it is not required here.
            if !present(&source.url) {
                errors.push(err(&at("url"), "crate source requires a url (the crate name)"));
            }
            match &source.sha256 {
                s if !present(s) => {
                    errors.push(err(&at("sha256"), "crate source requires a sha256"))
                }
                Some(s) if !is_sha256(s) => errors.push(err(
                    &at("sha256"),
                    "sha256 must be a 64-character hex content-address",
                )),
                _ => {}
            }
        }
        SourceType::Local => {
            if !present(&source.url) {
                errors.push(err(&at("url"), "local source requires a path url"));
            }
        }
    }
}

fn validate_build(build: &Build, errors: &mut Vec<ValidationError>) {
    if !build.offline {
        match &build.fetch_lock {
            None => errors.push(err(
                "build.fetch_lock",
                "an online build (offline = false) requires a fetch_lock",
            )),
            Some(lock) if lock.entries.is_empty() => errors.push(err(
                "build.fetch_lock.entries",
                "an online build requires a non-empty fetch_lock",
            )),
            Some(lock) => {
                for (i, entry) in lock.entries.iter().enumerate() {
                    let at = |field: &str| format!("build.fetch_lock.entries[{i}].{field}");
                    if entry.url.trim().is_empty() {
                        errors.push(err(&at("url"), "fetch-lock entry requires a url"));
                    }
                    if !is_sha256(&entry.sha256) {
                        errors.push(err(
                            &at("sha256"),
                            "fetch-lock entry requires a 64-character hex sha256",
                        ));
                    }
                }
            }
        }
    }

    for (i, step) in build.steps.iter().enumerate() {
        if step.tool.trim().is_empty() {
            errors.push(err(
                &format!("build.steps[{i}].tool"),
                "a build step requires a non-empty tool",
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// Linting (non-fatal)
// ---------------------------------------------------------------------------

/// Return non-fatal recommendations for a recipe.
pub fn lint(recipe: &Recipe) -> Vec<ValidationWarning> {
    let mut warnings = Vec::new();

    // Note: a malformed reverse-DNS id is a fatal `validate` error, not a lint.

    match &recipe.recipe.version {
        Some(v) if !is_semver_like(v) => warnings.push(warn(
            "recipe.version",
            format!("'{v}' is not a semver-like version"),
        )),
        _ => {}
    }

    if recipe.recipe.summary.is_none() {
        warnings.push(warn("recipe.summary", "a one-line summary is recommended"));
    }
    if recipe.recipe.license.is_none() {
        warnings.push(warn("recipe.license", "an SPDX license is recommended"));
    }

    // A github-release source follows tags, so a fixed version is redundant.
    let has_release = recipe
        .source
        .iter()
        .any(|s| s.source_type == SourceType::GithubRelease);
    if has_release && recipe.recipe.version.is_some() {
        warnings.push(warn(
            "recipe.version",
            "version is ignored for a github-release source (it follows tags)",
        ));
    }

    if recipe
        .artifacts
        .as_ref()
        .is_none_or(|a| a.bin.is_empty())
    {
        warnings.push(warn(
            "artifacts.bin",
            "no binaries are declared; nothing user-runnable will be collected",
        ));
    }

    warnings
}

// ---------------------------------------------------------------------------
// small helpers
// ---------------------------------------------------------------------------

fn err(field: &str, message: &str) -> ValidationError {
    ValidationError {
        field: field.to_string(),
        message: message.to_string(),
    }
}

fn warn(field: &str, message: impl Into<String>) -> ValidationWarning {
    ValidationWarning {
        field: field.to_string(),
        message: message.into(),
    }
}

/// Whether a string is all-hex of the given length (case-insensitive).
fn is_hex_len(s: &str, len: usize) -> bool {
    s.len() == len && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Whether a string is a full git object id: 40 hex (SHA-1) or 64 hex
/// (SHA-256). Branch names, tags and abbreviated hashes are rejected so a
/// source pin cannot resolve to mutable upstream content.
fn is_git_commit(s: &str) -> bool {
    is_hex_len(s, 40) || is_hex_len(s, 64)
}

/// Whether a string is a 64-character hex sha256 content-address.
fn is_sha256(s: &str) -> bool {
    is_hex_len(s, 64)
}

/// Whether `url` is a `github.com/{owner}/{repo}` repository url (optionally
/// scheme-prefixed), the form a `github-release` source requires (D7). A loose
/// validate-time check; the fetch resolves and pins the same host.
fn is_github_repo_url(url: &str) -> bool {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    let Some(path) = rest
        .strip_prefix("github.com/")
        .or_else(|| rest.strip_prefix("github.com./"))
    else {
        return false;
    };
    // Require at least an owner and a repo path segment.
    let mut segs = path.trim_end_matches('/').split('/').filter(|s| !s.is_empty());
    segs.next().is_some() && segs.next().is_some()
}

/// Whether a hex object id is the git null id (all zeros), used as an "unset"
/// sentinel and never a real commit.
fn is_null_oid(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b == b'0')
}

/// Whether a string is a valid recipe id (reverse-DNS notation). Exposed so
/// callers (e.g. the forage CLI's `recipe new`) can reject a bad id before
/// deriving filesystem paths from it.
pub fn is_valid_id(id: &str) -> bool {
    is_reverse_domain(id)
}

/// Whether a string looks like reverse-DNS notation (at least two
/// dot-separated, non-empty segments of `[A-Za-z0-9_-]`).
fn is_reverse_domain(s: &str) -> bool {
    let segments: Vec<&str> = s.split('.').collect();
    segments.len() >= 2
        && segments.iter().all(|seg| {
            !seg.is_empty()
                && seg
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
}

/// Whether a string looks semver-like (`major.minor[.patch][-pre][+build]`,
/// leading `v` tolerated).
fn is_semver_like(s: &str) -> bool {
    let core = s.strip_prefix('v').unwrap_or(s);
    let core = core.split(['-', '+']).next().unwrap_or(core);
    let parts: Vec<&str> = core.split('.').collect();
    (2..=3).contains(&parts.len())
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A full 40-hex git object id, valid as an immutable pin.
    const COMMIT: &str = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
    /// A valid 64-hex sha256 (the content-address of the empty string).
    const SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    fn git_recipe(extra: &str) -> String {
        format!(
            r#"
[recipe]
id = "org.example.hello"
name = "Hello"
maintainer = "key:abc"

[[source]]
type = "git"
url = "https://github.com/example/hello"
commit = "{COMMIT}"
{extra}"#
        )
    }

    #[test]
    fn parses_minimal_and_defaults() {
        let r = parse(&git_recipe("")).expect("parses");
        assert_eq!(r.recipe.id, "org.example.hello");
        assert_eq!(r.recipe.recipe_revision, 1, "recipe_revision defaults to 1");
        assert_eq!(r.source.len(), 1);
        assert_eq!(r.source[0].source_type, SourceType::Git);
        assert!(validate(&r).is_empty(), "minimal recipe is valid: {:?}", validate(&r));
    }

    #[test]
    fn single_source_table_accepted() {
        let toml = format!(
            r#"
[recipe]
id = "org.example.hello"
name = "Hello"
maintainer = "key:abc"

[source]
type = "tarball"
url = "https://example.org/hello.tar.gz"
sha256 = "{SHA256}"
"#
        );
        let r = parse(&toml).expect("single [source] table parses");
        assert_eq!(r.source.len(), 1);
        assert_eq!(r.source[0].source_type, SourceType::Tarball);
        assert!(validate(&r).is_empty());
    }

    #[test]
    fn git_source_requires_commit() {
        let toml = r#"
[recipe]
id = "org.example.hello"
name = "Hello"
maintainer = "key:abc"

[[source]]
type = "git"
url = "https://github.com/example/hello"
"#;
        let errors = validate(&parse(toml).expect("parses"));
        assert!(
            errors.iter().any(|e| e.field == "source[0].commit"),
            "missing commit is fatal: {errors:?}"
        );
    }

    #[test]
    fn floating_git_ref_is_fatal() {
        for floating in ["main", "v1.2.3", "deadbeef", "HEAD"] {
            let toml = format!(
                r#"
[recipe]
id = "org.example.hello"
name = "Hello"
maintainer = "key:abc"

[[source]]
type = "git"
url = "https://github.com/example/hello"
commit = "{floating}"
"#
            );
            let errors = validate(&parse(&toml).unwrap());
            assert!(
                errors.iter().any(|e| e.field == "source[0].commit"),
                "'{floating}' is not an immutable pin: {errors:?}"
            );
        }
    }

    #[test]
    fn null_object_id_commit_is_fatal() {
        for null in ["0".repeat(40), "0".repeat(64)] {
            let toml = format!(
                r#"
[recipe]
id = "org.example.hello"
name = "Hello"
maintainer = "key:abc"

[[source]]
type = "git"
url = "https://github.com/example/hello"
commit = "{null}"
"#
            );
            let errors = validate(&parse(&toml).unwrap());
            assert!(
                errors.iter().any(|e| e.field == "source[0].commit"),
                "the null object id is not a real pin: {errors:?}"
            );
        }
    }

    #[test]
    fn tarball_requires_valid_sha256() {
        let missing = r#"
[recipe]
id = "org.example.hello"
name = "Hello"
maintainer = "key:abc"

[[source]]
type = "tarball"
url = "https://example.org/hello.tar.gz"
"#;
        assert!(validate(&parse(missing).unwrap())
            .iter()
            .any(|e| e.field == "source[0].sha256"));

        let malformed = r#"
[recipe]
id = "org.example.hello"
name = "Hello"
maintainer = "key:abc"

[[source]]
type = "tarball"
url = "https://example.org/hello.tar.gz"
sha256 = "abc123"
"#;
        assert!(
            validate(&parse(malformed).unwrap())
                .iter()
                .any(|e| e.field == "source[0].sha256"),
            "a short/malformed sha256 is fatal"
        );
    }

    #[test]
    fn github_release_needs_a_repo_url_and_asset_not_sha256() {
        // D7: an explicit github.com/{owner}/{repo} url plus an asset template;
        // no sha256 (the asset is resolved-and-locked per release at fetch time).
        let toml = r#"
[recipe]
id = "dev.zed.Zed"
name = "Zed"
maintainer = "key:abc"

[[source]]
type = "github-release"
url = "github.com/zed-industries/zed"
asset = "zed-{version}-{target}.tar.gz"
"#;
        let errors = validate(&parse(toml).unwrap());
        assert!(errors.is_empty(), "a release with url + asset is valid: {errors:?}");
    }

    #[test]
    fn github_release_without_a_url_is_fatal() {
        let toml = r#"
[recipe]
id = "dev.zed.Zed"
name = "Zed"
maintainer = "key:abc"

[[source]]
type = "github-release"
asset = "zed-{version}-{target}.tar.gz"
"#;
        let errors = validate(&parse(toml).unwrap());
        assert!(
            errors.iter().any(|e| e.field == "source[0].url"),
            "a github-release without a url is fatal (D7): {errors:?}"
        );
    }

    #[test]
    fn github_release_with_a_non_github_url_is_fatal() {
        let toml = r#"
[recipe]
id = "dev.zed.Zed"
name = "Zed"
maintainer = "key:abc"

[[source]]
type = "github-release"
url = "https://gitlab.com/zed/zed"
asset = "zed-{version}-{target}.tar.gz"
"#;
        let errors = validate(&parse(toml).unwrap());
        assert!(
            errors.iter().any(|e| e.field == "source[0].url"),
            "a non-github-release repo url is fatal: {errors:?}"
        );
    }

    #[test]
    fn online_build_requires_fetch_lock() {
        let errors = validate(&parse(&git_recipe("[build]\nsystem = \"cargo\"\noffline = false\n")).unwrap());
        assert!(
            errors.iter().any(|e| e.field == "build.fetch_lock"),
            "offline=false without a fetch_lock is fatal: {errors:?}"
        );
    }

    #[test]
    fn online_build_rejects_empty_lock_entry() {
        let toml = git_recipe(
            r#"[build]
system = "cargo"
offline = false
[[build.fetch_lock.entries]]
url = ""
sha256 = "bad"
"#,
        );
        let errors = validate(&parse(&toml).unwrap());
        assert!(errors.iter().any(|e| e.field == "build.fetch_lock.entries[0].url"));
        assert!(errors.iter().any(|e| e.field == "build.fetch_lock.entries[0].sha256"));
    }

    #[test]
    fn online_build_accepts_valid_lock() {
        let toml = git_recipe(&format!(
            r#"[build]
system = "cargo"
offline = false
[[build.fetch_lock.entries]]
url = "https://crates.io/x"
sha256 = "{SHA256}"
"#
        ));
        assert!(validate(&parse(&toml).unwrap()).is_empty());
    }

    #[test]
    fn offline_build_is_default_and_needs_no_lock() {
        let r = parse(&git_recipe("[build]\nsystem = \"cargo\"\n")).unwrap();
        assert!(r.build.as_ref().unwrap().offline, "offline defaults to true");
        assert!(validate(&r).is_empty());
    }

    #[test]
    fn build_step_requires_tool() {
        let toml = git_recipe("[build]\nsystem = \"custom\"\n[[build.steps]]\ntool = \"\"\nargs = [\"x\"]\n");
        let errors = validate(&parse(&toml).unwrap());
        assert!(errors.iter().any(|e| e.field == "build.steps[0].tool"));
    }

    #[test]
    fn empty_source_list_is_fatal() {
        let toml = r#"
[recipe]
id = "org.example.hello"
name = "Hello"
maintainer = "key:abc"
"#;
        let errors = validate(&parse(toml).unwrap());
        assert!(errors.iter().any(|e| e.field == "source"));
    }

    #[test]
    fn non_reverse_dns_id_is_fatal() {
        let toml = git_recipe("").replace("org.example.hello", "singlename");
        let errors = validate(&parse(&toml).unwrap());
        assert!(
            errors.iter().any(|e| e.field == "recipe.id"),
            "id must equal the reverse-DNS .lunpkg id: {errors:?}"
        );
    }

    #[test]
    fn unknown_field_is_a_parse_error() {
        let toml = r#"
[recipe]
id = "org.example.hello"
name = "Hello"
maintainer = "key:abc"
bogus = "x"
"#;
        assert!(matches!(parse(toml), Err(RecipeError::Parse(_))));
    }

    #[test]
    fn graph_capabilities_are_prefixed_scope_strings() {
        let toml = git_recipe(
            r#"[capabilities]
network = ["api.example.org:443"]
notifications = true
graph = ["read:File", "write:Tag"]
"#,
        );
        let r = parse(&toml).unwrap();
        assert!(validate(&r).is_empty());
        let caps = r.capabilities.unwrap();
        assert!(caps.notifications);
        assert_eq!(caps.network, vec!["api.example.org:443"]);
        assert_eq!(caps.graph, vec!["read:File", "write:Tag"]);
    }

    #[test]
    fn unknown_capability_categories_are_preserved_not_dropped() {
        let toml = git_recipe(
            r#"[capabilities]
future_category = ["x"]
"#,
        );
        let r = parse(&toml).expect("open capabilities tolerate new categories");
        let caps = r.capabilities.unwrap();
        assert!(
            caps.extra.contains_key("future_category"),
            "unknown categories are captured, not silently dropped"
        );
    }

    #[test]
    fn lint_flags_soft_issues() {
        let toml = format!(
            r#"
[recipe]
id = "org.example.hello"
name = "Hello"
maintainer = "key:abc"
version = "not.semver.x"

[[source]]
type = "git"
url = "https://github.com/example/hello"
commit = "{COMMIT}"
"#
        );
        let r = parse(&toml).unwrap();
        assert!(validate(&r).is_empty(), "soft issues are not fatal: {:?}", validate(&r));
        let warnings = lint(&r);
        assert!(warnings.iter().any(|w| w.field == "recipe.version"));
        assert!(warnings.iter().any(|w| w.field == "recipe.license"));
        assert!(warnings.iter().any(|w| w.field == "artifacts.bin"));
    }

    #[test]
    fn helpers() {
        assert!(is_reverse_domain("org.arlen.app"));
        assert!(is_reverse_domain("com.example"));
        assert!(!is_reverse_domain("single"));
        assert!(!is_reverse_domain("trailing."));
        assert!(is_semver_like("1.2.3"));
        assert!(is_semver_like("v0.1"));
        assert!(is_semver_like("1.2.3-rc1"));
        assert!(!is_semver_like("1"));
        assert!(!is_semver_like("x.y.z"));
        assert!(is_git_commit(COMMIT));
        assert!(is_git_commit(&"a".repeat(64)));
        assert!(!is_git_commit("main"));
        assert!(!is_git_commit("deadbeef"));
        assert!(is_sha256(SHA256));
        assert!(!is_sha256("abc123"));
        assert!(is_null_oid(&"0".repeat(40)));
        assert!(!is_null_oid(COMMIT));
        assert!(is_valid_id("org.arlen.app"));
        assert!(!is_valid_id("singlename"));
    }
}
