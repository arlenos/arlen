//! `forage cookbook sign`: compile a human-authored cookbook manifest into
//! signed TUF metadata (decision D8, forage-recipes.md section 7a).
//!
//! The maintainer edits Arlen-idiomatic TOML (`cookbook.toml` plus the recipes
//! it indexes). This crate turns that into the four canonical TUF roles (root,
//! targets, snapshot, timestamp) using the maintained [`tough`] TUF library, so
//! no signature format or canonical JSON is ever hand-rolled. Each indexed
//! recipe becomes one TUF target whose content is the recipe's `recipe.toml` at
//! the pinned commit: the signer is given that content, computes its length and
//! sha256, cross-checks the sha256 against the manifest's declared `recipe_hash`
//! (catching a maintainer typo before it is signed), and records the git pointer
//! (`git_url`, `commit`) and the curated `capability_cap` in the target's custom
//! metadata. The target carries no served blob, the recipe is fetched from git
//! by the resolver and verified against this signed hash, so the cookbook stays
//! a pointer index, not a mirror. Per-target metadata is what lets a cookbook
//! glob-delegate a namespace to an upstream's own key later.
//!
//! This first slice signs the personal single-key cookbook (section 7a: "personal
//! is a single key"): one Ed25519 key holds all four roles at threshold one. The
//! M-of-N official-cookbook root and per-role key separation are a later
//! refinement; the signing entry point is shaped to grow into them.

use std::collections::HashMap;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};

use arlen_cookbook_index::{validate, CookbookManifest, RecipeEntry, ValidationError};
use aws_lc_rs::rand::SystemRandom;
use chrono::{DateTime, Duration, Utc};
use ed25519_dalek::pkcs8::EncodePrivateKey;
use ed25519_dalek::SigningKey;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tough::editor::signed::SignedRole;
use tough::editor::RepositoryEditor;
use tough::key_source::{KeySource, LocalKeySource};
use tough::schema::decoded::{Decoded, Hex};
use tough::schema::key::Key;
use tough::schema::{Hashes, KeyHolder, RoleKeys, RoleType, Root, Target};
use tough::TargetName;
use zeroize::Zeroizing;

/// The on-disk seed length of an Ed25519 key.
const SEED_LEN: usize = 32;

/// The TUF metadata version stamped on every role for an initial signing.
/// Re-signing with version bumps (rollback protection over time) is a later
/// increment; this first slice always signs version one.
const ONE: NonZeroU64 = match NonZeroU64::new(1) {
    Some(v) => v,
    None => unreachable!(),
};

/// A failure signing a cookbook. Every variant is terminal.
#[derive(Debug, Error)]
pub enum SignError {
    /// The manifest is not well-formed, so it must not be signed.
    #[error("invalid cookbook manifest: {0:?}")]
    Manifest(Vec<ValidationError>),
    /// A recipe indexed by the manifest had no content supplied to sign.
    #[error("no recipe content supplied for `{name}`")]
    MissingRecipe {
        /// The recipe name the manifest indexed but the caller did not provide.
        name: String,
    },
    /// The supplied recipe content does not hash to the manifest's declared
    /// `recipe_hash`; signing it would pin a recipe the cookbook never vetted.
    #[error("recipe `{name}` content sha256 {actual} does not match the manifest recipe_hash {declared}")]
    HashMismatch {
        /// The recipe name.
        name: String,
        /// The hash the manifest declared.
        declared: String,
        /// The hash of the supplied content.
        actual: String,
    },
    /// A filesystem operation failed.
    #[error("io at {path}: {source}")]
    Io {
        /// The path being read or written.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// Key generation or PKCS8 encoding failed.
    #[error("key generation: {0}")]
    KeyGen(String),
    /// The TUF library rejected an operation (signing, target add, write).
    #[error("tuf: {0}")]
    Tuf(String),
}

/// Generate a fresh Ed25519 signing key and write it as a PKCS8 DER file at
/// mode 0600, the format [`tough`]'s `LocalKeySource` reads for Ed25519 (its
/// parser feeds the raw file bytes to `Ed25519KeyPair::from_pkcs8`, which wants
/// DER, not PEM).
///
/// The seed comes from the OS CSPRNG and is zeroized after use; the DER document
/// is held in a zeroizing [`SecretDocument`](ed25519_dalek::pkcs8::der::SecretDocument)
/// so the secret does not linger in freed memory. The file is created `0600` so
/// it never exists group- or world-readable.
pub fn generate_signing_key(path: &Path) -> Result<(), SignError> {
    let mut seed = Zeroizing::new([0u8; SEED_LEN]);
    getrandom::getrandom(seed.as_mut()).map_err(|e| SignError::KeyGen(e.to_string()))?;
    let key = SigningKey::from_bytes(&seed);
    let der = key
        .to_pkcs8_der()
        .map_err(|e| SignError::KeyGen(e.to_string()))?;
    write_private(path, der.as_bytes())
}

/// Write `bytes` to `path`, creating it new at mode 0600 (no clobber).
#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), SignError> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|source| SignError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    f.write_all(bytes).map_err(|source| SignError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    f.sync_all().map_err(|source| SignError::Io {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), SignError> {
    std::fs::write(path, bytes).map_err(|source| SignError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// The inputs to [`sign_cookbook`].
pub struct SignParams<'a> {
    /// The validated cookbook manifest (the recipe index).
    pub manifest: &'a CookbookManifest,
    /// The content of each indexed recipe's `recipe.toml`, keyed by recipe name.
    /// The signer computes the TUF target descriptor from this and cross-checks
    /// the hash against the manifest.
    pub recipes: &'a HashMap<String, Vec<u8>>,
    /// The PKCS8 DER signing key (see [`generate_signing_key`]).
    pub key_path: &'a Path,
    /// Where to write the signed metadata. Created if it does not exist.
    pub out_dir: &'a Path,
    /// Per-role expiries. TUF gives snapshot and timestamp short windows so a
    /// mirror cannot freeze or replay stale metadata; root and targets are
    /// long-lived. A single shared expiry would neuter that freshness defense,
    /// so the roles are set independently (see [`Expiries`]).
    pub expiries: Expiries,
}

/// The expiry stamped on each TUF role. Snapshot and timestamp are the freshness
/// defense (forage-recipes.md section 7a: "short expiry, defending against a
/// mirror's freeze / rollback / replay"), so they are deliberately separate from
/// the long-lived root and targets.
#[derive(Debug, Clone, Copy)]
pub struct Expiries {
    /// The root role's expiry (rarely changed, long-lived).
    pub root: DateTime<Utc>,
    /// The targets role's expiry (the recipe index).
    pub targets: DateTime<Utc>,
    /// The snapshot role's expiry (short, freshness).
    pub snapshot: DateTime<Utc>,
    /// The timestamp role's expiry (shortest, freshness).
    pub timestamp: DateTime<Utc>,
}

impl Expiries {
    /// Sensible TUF defaults measured from `now`: root one year, targets ninety
    /// days, snapshot seven days, timestamp one day. A maintainer who re-signs
    /// more or less often overrides these directly.
    pub fn defaults_from(now: DateTime<Utc>) -> Self {
        Self {
            root: now + Duration::days(365),
            targets: now + Duration::days(90),
            snapshot: now + Duration::days(7),
            timestamp: now + Duration::days(1),
        }
    }
}

/// Sign a cookbook manifest into TUF metadata written under `out_dir`.
///
/// Bootstraps a single-key root (the key holds all four roles at threshold one),
/// signs it, then adds one TUF target per indexed recipe and signs the targets,
/// snapshot and timestamp roles. The recipe content supplied in
/// [`SignParams::recipes`] must hash to the manifest's `recipe_hash` or the call
/// fails closed rather than sign a recipe the cookbook did not vet.
pub async fn sign_cookbook(params: SignParams<'_>) -> Result<(), SignError> {
    let errors = validate(params.manifest);
    if !errors.is_empty() {
        return Err(SignError::Manifest(errors));
    }

    tokio::fs::create_dir_all(params.out_dir)
        .await
        .map_err(|source| SignError::Io {
            path: params.out_dir.to_path_buf(),
            source,
        })?;

    // Validate every recipe's content against its declared hash and build the
    // target descriptors before touching any key material, so a content or hash
    // problem fails closed without producing partial signed state.
    let mut targets: Vec<(TargetName, Target)> = Vec::new();
    for entry in &params.manifest.recipe {
        let bytes = params
            .recipes
            .get(&entry.name)
            .ok_or_else(|| SignError::MissingRecipe {
                name: entry.name.clone(),
            })?;
        let actual = sha256_hex(bytes);
        if actual != entry.recipe_hash {
            return Err(SignError::HashMismatch {
                name: entry.name.clone(),
                declared: entry.recipe_hash.clone(),
                actual,
            });
        }
        let target = target_for(entry, bytes)?;
        let name = TargetName::new(entry.name.clone()).map_err(|e| SignError::Tuf(e.to_string()))?;
        targets.push((name, target));
    }

    let key_source = LocalKeySource {
        path: params.key_path.to_path_buf(),
    };
    let tuf_key = key_source
        .as_sign()
        .await
        .map_err(|e| SignError::Tuf(e.to_string()))?
        .tuf_key();

    // Bootstrap and sign the root.
    let mut root = Root {
        spec_version: "1.0.0".to_string(),
        consistent_snapshot: true,
        version: ONE,
        expires: params.expiries.root,
        keys: HashMap::new(),
        roles: HashMap::new(),
        _extra: HashMap::new(),
    };
    add_key(
        &mut root,
        &[
            RoleType::Root,
            RoleType::Snapshot,
            RoleType::Targets,
            RoleType::Timestamp,
        ],
        tuf_key,
    )?;

    let key_sources: Vec<Box<dyn KeySource>> = vec![Box::new(LocalKeySource {
        path: params.key_path.to_path_buf(),
    })];
    let signed_root = SignedRole::new(
        root.clone(),
        &KeyHolder::Root(root.clone()),
        &key_sources,
        &SystemRandom::new(),
    )
    .await
    .map_err(|e| SignError::Tuf(e.to_string()))?;

    // The unversioned `root.json` is the bootstrap anchor a client TOFU-pins on
    // first fetch and the file `RepositoryEditor::new` loads here. The editor's
    // later write also emits the versioned `1.root.json` (root's filename is
    // always version-prefixed); both are byte-identical signed root metadata.
    let root_path = params.out_dir.join("root.json");
    tokio::fs::write(&root_path, signed_root.buffer())
        .await
        .map_err(|source| SignError::Io {
            path: root_path.clone(),
            source,
        })?;

    // Add a target per recipe and sign targets/snapshot/timestamp.
    let mut editor = RepositoryEditor::new(&root_path)
        .await
        .map_err(|e| SignError::Tuf(e.to_string()))?;

    for (name, target) in targets {
        editor
            .add_target(name, target)
            .map_err(|e| SignError::Tuf(e.to_string()))?;
    }

    editor
        .targets_version(ONE)
        .map_err(|e| SignError::Tuf(e.to_string()))?
        .targets_expires(params.expiries.targets)
        .map_err(|e| SignError::Tuf(e.to_string()))?
        .snapshot_version(ONE)
        .snapshot_expires(params.expiries.snapshot)
        .timestamp_version(ONE)
        .timestamp_expires(params.expiries.timestamp);

    let signed = editor
        .sign(&key_sources)
        .await
        .map_err(|e| SignError::Tuf(e.to_string()))?;
    signed
        .write(params.out_dir)
        .await
        .map_err(|e| SignError::Tuf(e.to_string()))?;
    Ok(())
}

/// Replicate tuftool's `add_key`: register a key under its key id and add that
/// id to each named role. Dedupes by key identity and by id. Roles are created
/// at threshold one (the single-key personal cookbook).
fn add_key(root: &mut Root, roles: &[RoleType], key: Key) -> Result<(), SignError> {
    let key_id = match root.keys.iter().find(|(_, k)| key.eq(k)) {
        Some((id, _)) => id.clone(),
        None => {
            let id = key.key_id().map_err(|e| SignError::Tuf(e.to_string()))?;
            root.keys.insert(id.clone(), key);
            id
        }
    };
    for role in roles {
        let entry = root.roles.entry(*role).or_insert_with(|| RoleKeys {
            keyids: Vec::new(),
            threshold: NonZeroU64::new(1).expect("1 is non-zero"),
            _extra: HashMap::new(),
        });
        if !entry.keyids.contains(&key_id) {
            entry.keyids.push(key_id.clone());
        }
    }
    Ok(())
}

/// Build the TUF target descriptor for one recipe: its length and sha256 plus
/// the git pointer and capability cap in custom metadata.
fn target_for(entry: &RecipeEntry, bytes: &[u8]) -> Result<Target, SignError> {
    let sha = hex::decode(&entry.recipe_hash).map_err(|e| SignError::Tuf(e.to_string()))?;
    let mut custom = HashMap::new();
    custom.insert(
        "git_url".to_string(),
        serde_json::Value::String(entry.git_url.clone()),
    );
    custom.insert(
        "commit".to_string(),
        serde_json::Value::String(entry.commit.clone()),
    );
    let cap = serde_json::to_value(&entry.capability_cap).map_err(|e| SignError::Tuf(e.to_string()))?;
    custom.insert("capability_cap".to_string(), cap);
    // A bridge recipe carries its foreign-app trigger in the signed target so
    // `forage install <foreign_app>` can enumerate + auto-install it without
    // fetching each recipe (foreign-app-bridges.md §4). Only emitted for bridge
    // recipes; a normal recipe omits it entirely.
    if let Some(foreign_app) = &entry.foreign_app {
        custom.insert(
            "foreign_app".to_string(),
            serde_json::Value::String(foreign_app.clone()),
        );
    }
    Ok(Target {
        length: bytes.len() as u64,
        hashes: Hashes {
            sha256: Decoded::<Hex>::from(sha),
            _extra: HashMap::new(),
        },
        custom,
        _extra: HashMap::new(),
    })
}

/// Lowercase-hex sha256 of `bytes`.
fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_for(name: &str, hash: &str) -> CookbookManifest {
        let toml = format!(
            "[[recipe]]\nname = \"{name}\"\ngit_url = \"github.com/o/r\"\ncommit = \"deadbeefdeadbeefdeadbeefdeadbeefdeadbeef\"\nrecipe_hash = \"{hash}\"\n"
        );
        arlen_cookbook_index::parse(&toml).unwrap()
    }

    #[tokio::test]
    async fn sign_then_load_and_resolve_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("cookbook.der");
        generate_signing_key(&key_path).expect("keygen");

        let recipe_bytes = b"name = \"com.example.Tool\"\nversion = \"1.0\"\n".to_vec();
        let hash = sha256_hex(&recipe_bytes);
        let manifest = manifest_for("com.example.Tool", &hash);
        let mut recipes = HashMap::new();
        recipes.insert("com.example.Tool".to_string(), recipe_bytes.clone());

        // Deliberately do not pre-create out_dir: sign_cookbook must create it.
        let out_dir = dir.path().join("metadata");

        sign_cookbook(SignParams {
            manifest: &manifest,
            recipes: &recipes,
            key_path: &key_path,
            out_dir: &out_dir,
            expiries: Expiries::defaults_from(Utc::now()),
        })
        .await
        .expect("sign");

        // Load the signed repo back (this verifies the whole signature chain:
        // root -> timestamp -> snapshot -> targets) and resolve our target's
        // custom metadata. A successful load is the real proof the four roles
        // were written and signed consistently; filenames are version-prefixed
        // under consistent_snapshot, so this does not assert specific names.
        let root_bytes = std::fs::read(out_dir.join("root.json")).unwrap();
        let metadata_url = url::Url::from_directory_path(&out_dir).unwrap();
        let targets_url = metadata_url.clone();
        let repo = tough::RepositoryLoader::new(&root_bytes, metadata_url, targets_url)
            .load()
            .await
            .expect("load+verify");
        let name = TargetName::new("com.example.Tool").unwrap();
        let target = repo.targets().signed.targets.get(&name).expect("target present");
        assert_eq!(target.length, recipe_bytes.len() as u64);
        assert_eq!(
            target.custom.get("git_url").unwrap(),
            &serde_json::Value::String("github.com/o/r".to_string())
        );
    }

    #[tokio::test]
    async fn a_content_hash_mismatch_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("cookbook.der");
        generate_signing_key(&key_path).unwrap();

        // Manifest declares a hash that the supplied content does not match.
        let manifest = manifest_for(
            "com.example.Tool",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        );
        let mut recipes = HashMap::new();
        recipes.insert("com.example.Tool".to_string(), b"different".to_vec());
        let out_dir = dir.path().join("metadata");

        let err = sign_cookbook(SignParams {
            manifest: &manifest,
            recipes: &recipes,
            key_path: &key_path,
            out_dir: &out_dir,
            expiries: Expiries::defaults_from(Utc::now()),
        })
        .await
        .expect_err("must reject a hash mismatch");
        assert!(matches!(err, SignError::HashMismatch { .. }));
    }
}
