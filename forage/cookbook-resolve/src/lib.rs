//! Verifying resolution of a signed forage cookbook (decision D8,
//! forage-recipes.md section 7a).
//!
//! A cookbook is added once, pinning its TUF `root` keys (trust on first use);
//! thereafter every resolution loads the cookbook's signed metadata and verifies
//! the whole chain against that pinned root before trusting a single byte. This
//! crate is the verifying half of the cookbook system: [`read_root_for_pinning`]
//! captures the anchor on add, and [`resolve`] loads + verifies the metadata and
//! returns one recipe's authenticated pointer (`git_url@commit`, the sha256
//! content pin, and the curated capability cap). It fails closed on a missing,
//! tampered, expired or otherwise unverifiable cookbook: a resolution either
//! returns a fully-verified pointer or an error, never an unchecked guess.
//!
//! The trust model is deliberate. The pinned root is the anchor the caller
//! stores on `cookbook add`; passing those same bytes to [`resolve`] on every
//! later call means `tough` verifies any root rotation against the pinned keys
//! (TUF root-key continuity), so a cookbook cannot silently swap its trust root.
//! Fetching the recipe content from `git_url@commit` and checking it against the
//! returned `recipe_hash`, plus enforcing the capability cap, are the caller's
//! next steps (the recipe lives in git, not in the cookbook), built on top of
//! this verified pointer.

use std::path::Path;

use arlen_forage_recipe::Capabilities;
use thiserror::Error;
use tough::{RepositoryLoader, TargetName};
use url::Url;

/// A failure resolving a recipe from a cookbook. Every variant is terminal: the
/// caller must not proceed with an unverified pointer.
#[derive(Debug, Error)]
pub enum ResolveError {
    /// The cookbook's `root.json` could not be read for pinning.
    #[error("read cookbook root at {path}: {source}")]
    Io {
        /// The path that could not be read.
        path: String,
        /// The underlying error.
        source: std::io::Error,
    },
    /// The metadata directory could not be expressed as a `file://` URL.
    #[error("metadata directory {0} is not an absolute path")]
    BadUrl(String),
    /// The requested recipe name is not a valid TUF target name.
    #[error("invalid recipe name `{0}`")]
    BadName(String),
    /// The cookbook metadata did not verify against the pinned root (bad
    /// signature, expired role, rollback, or a broken chain).
    #[error("cookbook failed verification: {0}")]
    Verify(String),
    /// The cookbook verified but does not index the requested recipe.
    #[error("recipe `{0}` is not in this cookbook")]
    NotFound(String),
    /// The recipe's signed target is missing a pointer field this crate needs.
    #[error("recipe `{name}` target is malformed: {reason}")]
    Malformed {
        /// The recipe name.
        name: String,
        /// What was wrong with the target.
        reason: String,
    },
}

/// A recipe pointer resolved and authenticated from a verified cookbook.
#[derive(Debug, Clone)]
pub struct ResolvedRecipe {
    /// The recipe's reverse-DNS name.
    pub name: String,
    /// The git repository the recipe lives in.
    pub git_url: String,
    /// The pinned commit to fetch the recipe at.
    pub commit: String,
    /// The recipe's authenticated sha256 content pin (lowercase hex), taken from
    /// the signed target's `hashes`. The caller fetches `git_url@commit` and
    /// checks the recipe against this.
    pub recipe_hash: String,
    /// The curated capability upper bound the cookbook signed, if any.
    pub capability_cap: Option<Capabilities>,
}

/// Read a cookbook's `root.json` to pin as the trust anchor on `cookbook add`
/// (trust on first use). The caller stores the returned bytes and passes them to
/// [`resolve`] on every later call.
pub fn read_root_for_pinning(metadata_dir: &Path) -> Result<Vec<u8>, ResolveError> {
    let path = metadata_dir.join("root.json");
    std::fs::read(&path).map_err(|source| ResolveError::Io {
        path: path.display().to_string(),
        source,
    })
}

/// Verify a cookbook against its pinned root and resolve one recipe's pointer.
///
/// Loads the cookbook's TUF metadata from `metadata_dir`, verifying the root ->
/// timestamp -> snapshot -> targets chain against `pinned_root` (and enforcing
/// expiry), then returns the named recipe's authenticated pointer. Any
/// verification failure, a missing recipe, or a malformed target is an error.
pub async fn resolve(
    pinned_root: &[u8],
    metadata_dir: &Path,
    recipe_name: &str,
) -> Result<ResolvedRecipe, ResolveError> {
    let url = Url::from_directory_path(metadata_dir)
        .map_err(|_| ResolveError::BadUrl(metadata_dir.display().to_string()))?;
    // The pinned root is the trust anchor; default ExpirationEnforcement fails
    // closed on an expired role, and the default transport loads file:// URLs.
    let repo = RepositoryLoader::new(&pinned_root, url.clone(), url)
        .load()
        .await
        .map_err(|e| ResolveError::Verify(e.to_string()))?;

    let name = TargetName::new(recipe_name).map_err(|_| ResolveError::BadName(recipe_name.into()))?;
    let target = repo
        .targets()
        .signed
        .targets
        .get(&name)
        .ok_or_else(|| ResolveError::NotFound(recipe_name.into()))?;

    resolved_from_target(target, recipe_name)
}

/// Build the authenticated [`ResolvedRecipe`] from a verified target + its name.
/// Shared by [`resolve`] (one named recipe) and [`enumerate_bridges_for`] (the
/// bridges tagged for a foreign app).
fn resolved_from_target(
    target: &tough::schema::Target,
    recipe_name: &str,
) -> Result<ResolvedRecipe, ResolveError> {
    let recipe_hash = hex::encode(&*target.hashes.sha256);
    let git_url = string_field(target, "git_url", recipe_name)?;
    let commit = string_field(target, "commit", recipe_name)?;
    let capability_cap = match target.custom.get("capability_cap") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => serde_json::from_value::<Option<Capabilities>>(value.clone()).map_err(|e| {
            ResolveError::Malformed {
                name: recipe_name.into(),
                reason: format!("capability_cap: {e}"),
            }
        })?,
    };

    Ok(ResolvedRecipe {
        name: recipe_name.into(),
        git_url,
        commit,
        recipe_hash,
        capability_cap,
    })
}

/// Verify a cookbook and return every BRIDGE recipe tagged for `foreign_app`.
///
/// A bridge recipe carries `foreign_app` in its signed target custom metadata
/// (foreign-app-bridges.md §4); `forage install <foreign_app>` uses this to
/// auto-install the matching bridges WITHOUT fetching each recipe. Loads +
/// verifies the cookbook exactly like [`resolve`] (the same pinned-root chain +
/// expiry enforcement), then filters the authenticated targets by their
/// `foreign_app` tag, deterministically ordered by recipe name. A malformed
/// MATCHING target is an error (fail-closed); a non-bridge or non-matching target
/// is skipped.
pub async fn enumerate_bridges_for(
    pinned_root: &[u8],
    metadata_dir: &Path,
    foreign_app: &str,
) -> Result<Vec<ResolvedRecipe>, ResolveError> {
    let url = Url::from_directory_path(metadata_dir)
        .map_err(|_| ResolveError::BadUrl(metadata_dir.display().to_string()))?;
    let repo = RepositoryLoader::new(&pinned_root, url.clone(), url)
        .load()
        .await
        .map_err(|e| ResolveError::Verify(e.to_string()))?;

    let mut out = Vec::new();
    for (name, target) in &repo.targets().signed.targets {
        let is_match = matches!(
            target.custom.get("foreign_app"),
            Some(serde_json::Value::String(fa)) if fa == foreign_app
        );
        if is_match {
            out.push(resolved_from_target(target, name.raw())?);
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Read a required string field from a target's custom metadata.
fn string_field(
    target: &tough::schema::Target,
    key: &str,
    recipe_name: &str,
) -> Result<String, ResolveError> {
    match target.custom.get(key) {
        Some(serde_json::Value::String(s)) => Ok(s.clone()),
        _ => Err(ResolveError::Malformed {
            name: recipe_name.into(),
            reason: format!("missing or non-string `{key}`"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_cookbook_sign::{generate_signing_key, sign_cookbook, Expiries, SignParams};
    use chrono::{Duration, Utc};
    use sha2::{Digest, Sha256};
    use std::collections::HashMap;

    fn sha256_hex(bytes: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(bytes);
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Sign a one-recipe cookbook into `out_dir` with the given expiries and
    /// return the recipe content's hash, for the resolve tests to assert.
    async fn sign_fixture(out_dir: &Path, key_path: &Path, expiries: Expiries) -> String {
        generate_signing_key(key_path).expect("keygen");
        let recipe_bytes = b"name = \"com.example.Tool\"\nversion = \"1.0\"\n".to_vec();
        let hash = sha256_hex(&recipe_bytes);
        let toml = format!(
            "[[recipe]]\nname = \"com.example.Tool\"\ngit_url = \"github.com/o/r\"\ncommit = \"deadbeefdeadbeefdeadbeefdeadbeefdeadbeef\"\nrecipe_hash = \"{hash}\"\n"
        );
        let manifest = arlen_cookbook_index::parse(&toml).unwrap();
        let mut recipes = HashMap::new();
        recipes.insert("com.example.Tool".to_string(), recipe_bytes);
        sign_cookbook(SignParams {
            manifest: &manifest,
            recipes: &recipes,
            key_path,
            out_dir,
            expiries,
        })
        .await
        .expect("sign");
        hash
    }

    #[tokio::test]
    async fn resolves_a_verified_recipe_pointer() {
        let dir = tempfile::tempdir().unwrap();
        let md = dir.path().join("md");
        let key = dir.path().join("k.der");
        let hash = sign_fixture(&md, &key, Expiries::defaults_from(Utc::now())).await;

        let root = read_root_for_pinning(&md).expect("pin root");
        let r = resolve(&root, &md, "com.example.Tool").await.expect("resolve");
        assert_eq!(r.git_url, "github.com/o/r");
        assert_eq!(r.commit, "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef");
        assert_eq!(r.recipe_hash, hash, "content pin comes from the signed target");
        assert!(r.capability_cap.is_none());
    }

    /// Sign a two-recipe cookbook: a normal recipe + an Obsidian BRIDGE recipe
    /// tagged `foreign_app = "obsidian"`, for the enumerate test.
    async fn sign_bridge_fixture(out_dir: &Path, key_path: &Path) {
        generate_signing_key(key_path).expect("keygen");
        let commit = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
        let normal = b"name = \"com.example.Tool\"\n".to_vec();
        let bridge = b"name = \"md.obsidian.bridge\"\n".to_vec();
        let toml = format!(
            "[[recipe]]\nname = \"com.example.Tool\"\ngit_url = \"github.com/o/r\"\ncommit = \"{commit}\"\nrecipe_hash = \"{nh}\"\n\n\
             [[recipe]]\nname = \"md.obsidian.bridge\"\ngit_url = \"github.com/m/b\"\ncommit = \"{commit}\"\nrecipe_hash = \"{bh}\"\nforeign_app = \"obsidian\"\n",
            nh = sha256_hex(&normal),
            bh = sha256_hex(&bridge),
        );
        let manifest = arlen_cookbook_index::parse(&toml).unwrap();
        let mut recipes = HashMap::new();
        recipes.insert("com.example.Tool".to_string(), normal);
        recipes.insert("md.obsidian.bridge".to_string(), bridge);
        sign_cookbook(SignParams {
            manifest: &manifest,
            recipes: &recipes,
            key_path,
            out_dir,
            expiries: Expiries::defaults_from(Utc::now()),
        })
        .await
        .expect("sign");
    }

    #[tokio::test]
    async fn enumerate_bridges_returns_only_the_matching_foreign_app_bridge() {
        let dir = tempfile::tempdir().unwrap();
        let md = dir.path().join("md");
        let key = dir.path().join("k.der");
        sign_bridge_fixture(&md, &key).await;
        let root = read_root_for_pinning(&md).expect("pin root");

        // Only the recipe tagged foreign_app=obsidian is returned (the normal
        // recipe carries no tag), with its authenticated pointer.
        let bridges = enumerate_bridges_for(&root, &md, "obsidian").await.expect("enumerate");
        assert_eq!(bridges.len(), 1, "only the tagged bridge: {bridges:?}");
        assert_eq!(bridges[0].name, "md.obsidian.bridge");
        assert_eq!(bridges[0].git_url, "github.com/m/b");

        // A foreign app with no bridge in the cookbook yields none, not an error.
        let none = enumerate_bridges_for(&root, &md, "notion").await.expect("enumerate");
        assert!(none.is_empty(), "no bridge tagged for notion: {none:?}");
    }

    #[tokio::test]
    async fn an_unknown_recipe_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let md = dir.path().join("md");
        let key = dir.path().join("k.der");
        sign_fixture(&md, &key, Expiries::defaults_from(Utc::now())).await;
        let root = read_root_for_pinning(&md).unwrap();
        let err = resolve(&root, &md, "com.example.Missing").await.unwrap_err();
        assert!(matches!(err, ResolveError::NotFound(_)), "{err:?}");
    }

    #[tokio::test]
    async fn tampered_metadata_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let md = dir.path().join("md");
        let key = dir.path().join("k.der");
        sign_fixture(&md, &key, Expiries::defaults_from(Utc::now())).await;
        let root = read_root_for_pinning(&md).unwrap();

        // Corrupt the timestamp role: its signature no longer matches.
        let ts = md.join("timestamp.json");
        let mut bytes = std::fs::read(&ts).unwrap();
        // Flip a byte well inside the signed content.
        let i = bytes.len() / 2;
        bytes[i] ^= 0xff;
        std::fs::write(&ts, &bytes).unwrap();

        let err = resolve(&root, &md, "com.example.Tool").await.unwrap_err();
        assert!(matches!(err, ResolveError::Verify(_)), "{err:?}");
    }

    #[tokio::test]
    async fn expired_metadata_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let md = dir.path().join("md");
        let key = dir.path().join("k.der");
        // Timestamp already expired; the rest valid.
        let now = Utc::now();
        let expiries = Expiries {
            root: now + Duration::days(365),
            targets: now + Duration::days(90),
            snapshot: now + Duration::days(7),
            timestamp: now - Duration::days(1),
        };
        sign_fixture(&md, &key, expiries).await;
        let root = read_root_for_pinning(&md).unwrap();

        let err = resolve(&root, &md, "com.example.Tool").await.unwrap_err();
        assert!(matches!(err, ResolveError::Verify(_)), "{err:?}");
    }
}
