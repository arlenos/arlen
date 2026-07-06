//! `.lunpkg` archive writer for forage.
//!
//! This slice completes packaging: it turns a collected staging tree (the
//! output of [`crate::collect_artifacts`]) plus the [`Recipe`] into a finished
//! `.lunpkg` that the install daemon can install. Two steps:
//!
//! 1. [`synthesize_manifest`] produces the `manifest.toml` string. Its shape is
//!    dictated by `arlen-installd`'s `Manifest` deserialiser, not invented here:
//!    `[package]`, `[binary]`, `[permissions]`, `[desktop]`, `[schemas]` and the
//!    `[provides]` table the recipe declares.
//! 2. [`write_lunpkg`] assembles the `tar.zst`: `manifest.toml`, every staged
//!    file at its relative path and a `signature.sig` that verifies under
//!    installd's Ed25519 verifier.
//!
//! Two properties are load-bearing because this component signs installable
//! packages:
//!
//! * **The signature matches installd's scheme exactly.** installd does not
//!   sign the archive bytes; it signs a SHA-256 hash computed over every file in
//!   the extracted package (`signature.sig` excluded) in sorted relative-path
//!   order, each file contributing its path, a NUL, its length as 8 little-endian
//!   bytes and its content. The writer reproduces that construction over the
//!   exact bytes it is about to archive, so the signature it emits is the one
//!   installd will check.
//! * **The archive is reproducible.** The same inputs produce byte-identical
//!   output: entries are emitted in sorted order, with a fixed mtime of 0, a
//!   normalised mode, zeroed uid/gid and empty owner/group names, so no host
//!   metadata leaks into the package and a rebuild can be compared bit-for-bit.

use std::io::Write;
use std::path::{Component, Path, PathBuf};

use arlen_forage_recipe::Recipe;
use ed25519_dalek::{Signer, SigningKey};
use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::Collection;

/// The manifest entry name at the archive root.
pub const MANIFEST_FILE: &str = "manifest.toml";
/// The signature entry name at the archive root.
pub const SIGNATURE_FILE: &str = "signature.sig";

/// The zstd compression level used for every package, fixed so the same input
/// produces byte-identical output.
const ZSTD_LEVEL: i32 = 19;

/// Failures while synthesising a `manifest.toml`.
#[derive(Debug, Error)]
pub enum ManifestError {
    /// The collection declared no binary, so `[binary].path` cannot be set.
    #[error("recipe collects no binary; a .lunpkg requires a [binary] path")]
    NoBinary,
    /// A capability string was not in the expected `prefix:Type` shape.
    #[error("malformed graph capability (expected 'read:Type' or 'write:Type'): {0}")]
    MalformedGraphCapability(String),
    /// The manifest could not be serialised to TOML.
    #[error("serialise manifest: {0}")]
    Serialize(#[from] toml::ser::Error),
}

/// Failures while writing a `.lunpkg`.
#[derive(Debug, Error)]
pub enum WriteError {
    /// A filesystem error.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// A staged path is absolute or escapes the staging root, so it cannot be a
    /// safe archive entry.
    #[error("staged path escapes the staging tree: {0}")]
    Escapes(String),
    /// A staged entry is a symlink. Collection already rejects these; the writer
    /// refuses again so a tampered staging tree cannot smuggle one into the
    /// archive.
    #[error("staged path is a symlink, which is not allowed: {0}")]
    Symlink(String),
    /// A staged path has no usable UTF-8 form for an archive entry name.
    #[error("staged path is not valid UTF-8: {0}")]
    NonUtf8(String),
    /// The staging tree contains a reserved entry name the writer owns.
    #[error("staging tree contains a reserved entry: {0}")]
    ReservedEntry(String),
}

// ---------------------------------------------------------------------------
// Manifest synthesis
// ---------------------------------------------------------------------------

/// `manifest.toml` top level, mirroring `arlen-installd`'s `Manifest`.
#[derive(Debug, Serialize)]
struct Manifest {
    package: PackageInfo,
    binary: BinaryInfo,
    #[serde(skip_serializing_if = "DesktopInfo::is_empty")]
    desktop: DesktopInfo,
    #[serde(skip_serializing_if = "PermissionInfo::is_empty")]
    permissions: PermissionInfo,
    #[serde(skip_serializing_if = "SchemaInfo::is_empty")]
    schemas: SchemaInfo,
    #[serde(skip_serializing_if = "ProvidesInfo::is_empty")]
    provides: ProvidesInfo,
}

/// `[package]` section.
#[derive(Debug, Serialize)]
struct PackageInfo {
    id: String,
    name: String,
    version: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    description: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    author: String,
}

/// `[binary]` section.
#[derive(Debug, Serialize)]
struct BinaryInfo {
    path: String,
}

/// `[desktop]` section.
#[derive(Debug, Default, Serialize)]
struct DesktopInfo {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    categories: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    keywords: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    mime_types: Vec<String>,
}

impl DesktopInfo {
    fn is_empty(&self) -> bool {
        self.categories.is_empty() && self.keywords.is_empty() && self.mime_types.is_empty()
    }
}

/// `[permissions]` section.
#[derive(Debug, Default, Serialize)]
struct PermissionInfo {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    graph_read: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    graph_write: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    filesystem: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    network: Vec<String>,
    #[serde(skip_serializing_if = "is_false")]
    notifications: bool,
    #[serde(skip_serializing_if = "is_false")]
    clipboard: bool,
}

impl PermissionInfo {
    fn is_empty(&self) -> bool {
        self.graph_read.is_empty()
            && self.graph_write.is_empty()
            && self.filesystem.is_empty()
            && self.network.is_empty()
            && !self.notifications
            && !self.clipboard
    }
}

/// `[schemas]` section.
#[derive(Debug, Default, Serialize)]
struct SchemaInfo {
    files: Vec<String>,
}

impl SchemaInfo {
    fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// `[provides]` section. installd does not read this today, but the recipe
/// declares it and the format reserves the table, so it is recorded faithfully.
#[derive(Debug, Default, Serialize)]
struct ProvidesInfo {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    schemas: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    binaries: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    mime: Vec<String>,
}

impl ProvidesInfo {
    fn is_empty(&self) -> bool {
        self.schemas.is_empty() && self.binaries.is_empty() && self.mime.is_empty()
    }
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Synthesise an installd-compatible `manifest.toml` from a recipe and the
/// collected staging tree.
///
/// The `[package]` identity comes from `recipe.recipe`. `[binary].path` is the
/// first collected binary (an error if the collection has none, since installd
/// requires a non-empty binary path). `[permissions]` is mapped from
/// `recipe.capabilities`: `filesystem`/`network`/`notifications`/`clipboard`
/// directly, and the prefixed graph scopes (`read:Type`, `write:Type`) split
/// into `graph_read`/`graph_write`. `[desktop]` and `[provides]` are filled from
/// what the recipe declares; `[schemas]` from `recipe.provides.schemas`.
///
/// The returned string parses back into installd's `Manifest`.
pub fn synthesize_manifest(
    recipe: &Recipe,
    collection: &Collection,
) -> Result<String, ManifestError> {
    let meta = &recipe.recipe;

    let binary_path = collection
        .binaries
        .first()
        .cloned()
        .ok_or(ManifestError::NoBinary)?;

    let package = PackageInfo {
        id: meta.id.clone(),
        name: meta.name.clone(),
        // installd rejects an empty version. A github-release recipe omits the
        // version (it follows tags); fall back to a placeholder the daemon
        // accepts rather than emitting an invalid manifest. The release pipeline
        // is expected to substitute the resolved tag before packaging.
        version: meta
            .version
            .clone()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "0.0.0".to_string()),
        description: meta.summary.clone().unwrap_or_default(),
        author: meta.maintainer.clone(),
    };

    let binary = BinaryInfo { path: binary_path };

    let mut permissions = PermissionInfo::default();
    let mut provides_info = ProvidesInfo::default();
    if let Some(caps) = &recipe.capabilities {
        permissions.filesystem = caps.filesystem.clone();
        permissions.network = caps.network.clone();
        permissions.notifications = caps.notifications;
        permissions.clipboard = caps.clipboard;
        for scope in &caps.graph {
            map_graph_scope(scope, &mut permissions)?;
        }
    }
    if let Some(provides) = &recipe.provides {
        provides_info.schemas = provides.schemas.clone();
        provides_info.binaries = provides.binaries.clone();
        provides_info.mime = provides.mime.clone();
    }

    // installd reads schema *files* from the package, whereas the recipe's
    // `provides.schemas` lists entity-type names, not file paths. Leave
    // `[schemas].files` empty here: nothing in the collection is a schema file,
    // and emitting type names as file paths would make installd look for files
    // that do not exist. The declared types are preserved under `[provides]`.
    let schemas = SchemaInfo::default();

    let manifest = Manifest {
        package,
        binary,
        desktop: DesktopInfo::default(),
        permissions,
        schemas,
        provides: provides_info,
    };

    Ok(toml::to_string(&manifest)?)
}

/// Split a recipe graph capability (`read:Type` / `write:Type`) into the
/// matching installd permission list.
fn map_graph_scope(scope: &str, permissions: &mut PermissionInfo) -> Result<(), ManifestError> {
    use arlen_forage_recipe::GraphScope;
    match arlen_forage_recipe::parse_graph_scope(scope) {
        Some(GraphScope::Read(t)) => permissions.graph_read.push(t),
        Some(GraphScope::Write(t)) => permissions.graph_write.push(t),
        None => return Err(ManifestError::MalformedGraphCapability(scope.to_string())),
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Archive writing
// ---------------------------------------------------------------------------

/// One entry to archive: its in-archive relative path and its content.
struct Entry {
    name: String,
    content: Vec<u8>,
    /// True for files that should be installed executable (binaries).
    executable: bool,
}

/// Assemble the `.lunpkg` at `out`: a `tar.zst` containing `manifest.toml`,
/// every file under `staging` at its relative path and a `signature.sig` that
/// verifies under installd's Ed25519 scheme.
///
/// The archive is reproducible: entries are sorted, mtime is fixed at 0, uid/gid
/// are zeroed, owner/group names are empty and the mode is normalised, so the
/// same inputs yield byte-identical output.
///
/// `manifest_toml` is the string produced by [`synthesize_manifest`]. The
/// signature is computed over the manifest plus every staged file (the signature
/// itself excluded) exactly as installd recomputes it, so a successful write is a
/// package installd will accept.
pub fn write_lunpkg(
    staging: &Path,
    manifest_toml: &str,
    signing_key: &SigningKey,
    out: &Path,
) -> Result<(), WriteError> {
    // Gather staged files first; the manifest and signature are added by the
    // writer and must not already be present in the staging tree.
    let mut entries: Vec<Entry> = Vec::new();
    collect_staged(staging, staging, &mut entries)?;
    // Reject reserved names as files AND as path prefixes: a staged
    // `signature.sig/x` or `manifest.toml/x` would otherwise pass the exact
    // check, and the writer's root `signature.sig`/`manifest.toml` files would
    // then collide with those directories in the sorted tar so installd's
    // unpack (a file cannot host a child) fails, yielding an un-installable
    // package.
    for e in &entries {
        let reserved = [MANIFEST_FILE, SIGNATURE_FILE].iter().any(|r| {
            e.name == *r || e.name.starts_with(&format!("{r}/"))
        });
        if reserved {
            return Err(WriteError::ReservedEntry(e.name.clone()));
        }
    }

    entries.push(Entry {
        name: MANIFEST_FILE.to_string(),
        content: manifest_toml.as_bytes().to_vec(),
        executable: false,
    });

    // Sort by archive name so both the signed hash and the archive layout are
    // deterministic and independent of directory-read order.
    entries.sort_by(|a, b| a.name.cmp(&b.name));

    // Sign over the same content installd will recompute (signature excluded).
    let hash = content_hash(&entries);
    let signature = signing_key.sign(&hash);

    entries.push(Entry {
        name: SIGNATURE_FILE.to_string(),
        content: signature.to_bytes().to_vec(),
        executable: false,
    });
    // Re-sort so the signature lands in its sorted position; the hash already
    // excluded it, matching installd which excludes signature.sig from the hash.
    entries.sort_by(|a, b| a.name.cmp(&b.name));

    write_archive(out, &entries)
}

/// Compute installd's content hash over the given entries.
///
/// Mirrors `arlen-installd`'s `compute_content_hash`: files in sorted
/// relative-path order, each contributing its path bytes, a NUL, its length as
/// 8 little-endian bytes and its content. `signature.sig` is never part of the
/// input here (the caller hashes before appending it).
fn content_hash(entries: &[Entry]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    // Entries are pre-sorted by the caller; sort defensively so the hash never
    // depends on insertion order.
    let mut sorted: Vec<&Entry> = entries.iter().filter(|e| e.name != SIGNATURE_FILE).collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    for e in sorted {
        hasher.update(e.name.as_bytes());
        hasher.update(b"\0");
        hasher.update((e.content.len() as u64).to_le_bytes());
        hasher.update(&e.content);
    }
    hasher.finalize().into()
}

/// Recursively gather regular files under `current`, recording each as an entry
/// named by its path relative to `base`.
fn collect_staged(base: &Path, current: &Path, out: &mut Vec<Entry>) -> Result<(), WriteError> {
    let mut children: Vec<PathBuf> = std::fs::read_dir(current)?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<_, _>>()?;
    children.sort();

    for path in children {
        let meta = std::fs::symlink_metadata(&path)?;
        let rel = path
            .strip_prefix(base)
            .map_err(|_| WriteError::Escapes(path.display().to_string()))?;
        let name = archive_name(rel)
            .ok_or_else(|| WriteError::NonUtf8(path.display().to_string()))?;

        if meta.file_type().is_symlink() {
            return Err(WriteError::Symlink(name));
        }
        if meta.is_dir() {
            collect_staged(base, &path, out)?;
        } else {
            let content = std::fs::read(&path)?;
            out.push(Entry {
                name,
                content,
                executable: is_executable(&path, &meta),
            });
        }
    }
    Ok(())
}

/// Turn a relative staging path into a forward-slash archive name, rejecting any
/// component that is not a normal segment so nothing can escape the tree.
fn archive_name(rel: &Path) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for c in rel.components() {
        match c {
            Component::Normal(p) => parts.push(p.to_str()?),
            Component::CurDir => {}
            _ => return None,
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

/// Whether a staged file should be archived as executable.
///
/// A file under `bin/` is always executable (installd chmods these to 0755 on
/// install regardless); otherwise honour the owner-execute bit so a build that
/// produced an executable helper keeps it.
#[cfg(unix)]
fn is_executable(path: &Path, meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    if path
        .components()
        .next()
        .and_then(|c| match c {
            Component::Normal(p) => p.to_str(),
            _ => None,
        })
        .is_some_and(|first| first == "bin")
    {
        return true;
    }
    meta.mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(path: &Path, _meta: &std::fs::Metadata) -> bool {
    path.components().next().is_some_and(|c| match c {
        Component::Normal(p) => p.to_str() == Some("bin"),
        _ => false,
    })
}

/// Write the sorted entries as a deterministic `tar.zst` to `out`.
fn write_archive(out: &Path, entries: &[Entry]) -> Result<(), WriteError> {
    let file = std::fs::File::create(out)?;
    let encoder = zstd::Encoder::new(file, ZSTD_LEVEL)?;
    let mut builder = tar::Builder::new(encoder);
    // Never copy host metadata into the archive.
    builder.mode(tar::HeaderMode::Deterministic);

    for e in entries {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_size(e.content.len() as u64);
        header.set_mtime(0);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mode(if e.executable { 0o755 } else { 0o644 });
        // Empty owner/group names so no host user leaks in. Errors are mapped to
        // IO; these calls only fail on an invalid (non-empty, oversized) name.
        header
            .set_username("")
            .map_err(|e| WriteError::Io(std::io::Error::other(e.to_string())))?;
        header
            .set_groupname("")
            .map_err(|e| WriteError::Io(std::io::Error::other(e.to_string())))?;
        header.set_cksum();

        builder
            .append_data(&mut header, &e.name, e.content.as_slice())
            .map_err(WriteError::Io)?;
    }

    let encoder = builder.into_inner().map_err(WriteError::Io)?;
    let mut file = encoder.finish().map_err(WriteError::Io)?;
    file.flush()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Collection;
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    use std::fs;
    use std::path::Path;

    const COMMIT: &str = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn recipe_with_caps(caps: &str) -> Recipe {
        let toml = format!(
            r#"
[recipe]
id = "org.example.hello"
name = "Hello World"
version = "1.2.3"
summary = "a friendly greeter"
maintainer = "key:abc"

[[source]]
type = "git"
url = "https://github.com/example/hello"
commit = "{COMMIT}"
{caps}"#
        );
        arlen_forage_recipe::parse(&toml).expect("recipe parses")
    }

    fn collection() -> Collection {
        Collection {
            files: vec!["bin/hello".to_string(), "share/data/x.txt".to_string()],
            binaries: vec!["bin/hello".to_string()],
        }
    }

    /// Build a small staging tree mirroring a collection.
    fn make_staging(dir: &Path) {
        fs::create_dir_all(dir.join("bin")).unwrap();
        fs::write(dir.join("bin/hello"), b"#!/bin/sh\necho hi").unwrap();
        fs::create_dir_all(dir.join("share/data")).unwrap();
        fs::write(dir.join("share/data/x.txt"), b"payload").unwrap();
    }

    // ── manifest synthesis ─────────────────────────────────────────────

    #[test]
    fn synthesizes_minimal_manifest() {
        let recipe = recipe_with_caps("");
        let toml = synthesize_manifest(&recipe, &collection()).unwrap();
        // Parses as TOML and carries the identity.
        let parsed: toml::Value = toml::from_str(&toml).unwrap();
        assert_eq!(parsed["package"]["id"].as_str(), Some("org.example.hello"));
        assert_eq!(parsed["package"]["version"].as_str(), Some("1.2.3"));
        assert_eq!(parsed["binary"]["path"].as_str(), Some("bin/hello"));
        assert_eq!(
            parsed["package"]["description"].as_str(),
            Some("a friendly greeter")
        );
    }

    #[test]
    fn no_binary_is_an_error() {
        let recipe = recipe_with_caps("");
        let empty = Collection {
            files: vec![],
            binaries: vec![],
        };
        assert!(matches!(
            synthesize_manifest(&recipe, &empty),
            Err(ManifestError::NoBinary)
        ));
    }

    #[test]
    fn maps_capabilities_to_permissions() {
        let recipe = recipe_with_caps(
            r#"[capabilities]
filesystem = ["~/Documents"]
network = ["api.example.org:443"]
notifications = true
clipboard = true
graph = ["read:File", "write:Tag", "read:Project"]
"#,
        );
        let toml = synthesize_manifest(&recipe, &collection()).unwrap();
        let parsed: toml::Value = toml::from_str(&toml).unwrap();
        let perms = &parsed["permissions"];
        assert_eq!(perms["notifications"].as_bool(), Some(true));
        assert_eq!(perms["clipboard"].as_bool(), Some(true));
        assert_eq!(
            perms["filesystem"].as_array().unwrap()[0].as_str(),
            Some("~/Documents")
        );
        assert_eq!(
            perms["network"].as_array().unwrap()[0].as_str(),
            Some("api.example.org:443")
        );
        let read: Vec<&str> = perms["graph_read"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(read, vec!["File", "Project"]);
        let write: Vec<&str> = perms["graph_write"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(write, vec!["Tag"]);
    }

    #[test]
    fn malformed_graph_scope_errors() {
        let recipe = recipe_with_caps(
            r#"[capabilities]
graph = ["File"]
"#,
        );
        assert!(matches!(
            synthesize_manifest(&recipe, &collection()),
            Err(ManifestError::MalformedGraphCapability(_))
        ));

        let bad_prefix = recipe_with_caps(
            r#"[capabilities]
graph = ["delete:File"]
"#,
        );
        assert!(matches!(
            synthesize_manifest(&bad_prefix, &collection()),
            Err(ManifestError::MalformedGraphCapability(_))
        ));
    }

    #[test]
    fn empty_graph_scope_type_is_rejected_and_whitespace_is_trimmed() {
        // An empty type after the colon is malformed (the `read:`/`read: ` branch
        // the existing missing-colon and unknown-prefix cases do not exercise).
        for bad in ["read:", "read: ", "write:"] {
            let recipe = recipe_with_caps(&format!("[capabilities]\ngraph = [\"{bad}\"]\n"));
            assert!(
                matches!(
                    synthesize_manifest(&recipe, &collection()),
                    Err(ManifestError::MalformedGraphCapability(_))
                ),
                "{bad:?} must be malformed"
            );
        }
        // Surrounding whitespace on the prefix and type is trimmed, so a padded
        // but otherwise valid scope maps cleanly.
        let padded = recipe_with_caps("[capabilities]\ngraph = [\" read : File \"]\n");
        let toml = synthesize_manifest(&padded, &collection()).unwrap();
        let parsed: toml::Value = toml::from_str(&toml).unwrap();
        let read: Vec<&str> = parsed["permissions"]["graph_read"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(read, vec!["File"]);
    }

    #[test]
    fn github_release_recipe_gets_placeholder_version() {
        // No version on a release recipe; installd requires a non-empty one.
        let toml = r#"
[recipe]
id = "dev.zed.Zed"
name = "Zed"
maintainer = "key:abc"

[[source]]
type = "github-release"
asset = "zed-{version}-{target}.tar.gz"
"#;
        let recipe = arlen_forage_recipe::parse(toml).unwrap();
        let manifest = synthesize_manifest(&recipe, &collection()).unwrap();
        let parsed: toml::Value = toml::from_str(&manifest).unwrap();
        assert_eq!(parsed["package"]["version"].as_str(), Some("0.0.0"));
    }

    // ── archive writing + round trip ────────────────────────────────────

    /// installd's content-hash recomputation, over an *extracted* directory.
    /// Replicated here so the round trip asserts the writer's signature matches
    /// what installd would verify, without depending on installd's crate.
    fn installd_content_hash(dir: &Path) -> [u8; 32] {
        fn collect(base: &Path, cur: &Path, files: &mut Vec<String>) {
            for entry in fs::read_dir(cur).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    collect(base, &path, files);
                } else {
                    let rel = path.strip_prefix(base).unwrap().to_string_lossy().to_string();
                    if rel != SIGNATURE_FILE {
                        files.push(rel);
                    }
                }
            }
        }
        let mut files = Vec::new();
        collect(dir, dir, &mut files);
        files.sort();
        let mut hasher = Sha256::new();
        for rel in &files {
            let content = fs::read(dir.join(rel)).unwrap();
            hasher.update(rel.as_bytes());
            hasher.update(b"\0");
            hasher.update((content.len() as u64).to_le_bytes());
            hasher.update(&content);
        }
        hasher.finalize().into()
    }

    /// Extract a tar.zst the way installd does.
    fn extract(pkg: &Path, into: &Path) {
        let file = fs::File::open(pkg).unwrap();
        let decoder = zstd::Decoder::new(file).unwrap();
        let mut archive = tar::Archive::new(decoder);
        archive.unpack(into).unwrap();
    }

    /// installd's manifest deserialiser, replicated minimally so the round trip
    /// proves the synthesised manifest parses into the daemon's shape.
    #[derive(serde::Deserialize)]
    struct InstalldManifest {
        package: InstalldPackage,
        binary: InstalldBinary,
        #[serde(default)]
        permissions: InstalldPermissions,
    }
    #[derive(serde::Deserialize)]
    struct InstalldPackage {
        id: String,
        name: String,
        version: String,
        #[serde(default)]
        description: String,
        #[serde(default)]
        author: String,
    }
    #[derive(serde::Deserialize)]
    struct InstalldBinary {
        path: String,
    }
    #[derive(Default, serde::Deserialize)]
    struct InstalldPermissions {
        #[serde(default)]
        graph_read: Vec<String>,
        #[serde(default)]
        graph_write: Vec<String>,
        #[serde(default)]
        notifications: bool,
    }

    #[test]
    fn round_trip_extract_and_verify() {
        let staging = tempfile::tempdir().unwrap();
        make_staging(staging.path());

        let recipe = recipe_with_caps(
            r#"[capabilities]
notifications = true
graph = ["read:File", "write:Tag"]
"#,
        );
        let manifest = synthesize_manifest(&recipe, &collection()).unwrap();

        let sk = signing_key();
        let out_dir = tempfile::tempdir().unwrap();
        let pkg = out_dir.path().join("hello.lunpkg");
        write_lunpkg(staging.path(), &manifest, &sk, &pkg).unwrap();
        assert!(pkg.exists());

        // Extract.
        let extracted = tempfile::tempdir().unwrap();
        extract(&pkg, extracted.path());

        // manifest.toml is present and parses into installd's shape.
        let mtoml = fs::read_to_string(extracted.path().join(MANIFEST_FILE)).unwrap();
        let parsed: InstalldManifest = toml::from_str(&mtoml).unwrap();
        assert_eq!(parsed.package.id, "org.example.hello");
        assert_eq!(parsed.package.name, "Hello World");
        assert_eq!(parsed.package.version, "1.2.3");
        assert_eq!(parsed.package.description, "a friendly greeter");
        assert_eq!(parsed.package.author, "key:abc");
        assert_eq!(parsed.binary.path, "bin/hello");
        assert_eq!(parsed.permissions.graph_read, vec!["File"]);
        assert_eq!(parsed.permissions.graph_write, vec!["Tag"]);
        assert!(parsed.permissions.notifications);

        // All staged files are present.
        assert!(extracted.path().join("bin/hello").exists());
        assert!(extracted.path().join("share/data/x.txt").exists());

        // signature.sig exists and verifies under the matching Ed25519 scheme.
        let sig_bytes = fs::read(extracted.path().join(SIGNATURE_FILE)).unwrap();
        assert_eq!(sig_bytes.len(), 64);
        let signature =
            Signature::from_bytes(sig_bytes.as_slice().try_into().unwrap());
        let hash = installd_content_hash(extracted.path());
        let vk: VerifyingKey = sk.verifying_key();
        assert!(
            vk.verify(&hash, &signature).is_ok(),
            "signature must verify over installd's content hash"
        );
    }

    #[test]
    fn tampering_breaks_verification() {
        let staging = tempfile::tempdir().unwrap();
        make_staging(staging.path());
        let recipe = recipe_with_caps("");
        let manifest = synthesize_manifest(&recipe, &collection()).unwrap();
        let sk = signing_key();
        let out_dir = tempfile::tempdir().unwrap();
        let pkg = out_dir.path().join("hello.lunpkg");
        write_lunpkg(staging.path(), &manifest, &sk, &pkg).unwrap();

        let extracted = tempfile::tempdir().unwrap();
        extract(&pkg, extracted.path());

        // Tamper with a file after signing.
        fs::write(extracted.path().join("bin/hello"), b"EVIL").unwrap();

        let sig_bytes = fs::read(extracted.path().join(SIGNATURE_FILE)).unwrap();
        let signature =
            Signature::from_bytes(sig_bytes.as_slice().try_into().unwrap());
        let hash = installd_content_hash(extracted.path());
        let vk = sk.verifying_key();
        assert!(
            vk.verify(&hash, &signature).is_err(),
            "tampered content must fail verification"
        );
    }

    #[test]
    fn output_is_byte_identical_across_writes() {
        let staging = tempfile::tempdir().unwrap();
        make_staging(staging.path());
        let recipe = recipe_with_caps("");
        let manifest = synthesize_manifest(&recipe, &collection()).unwrap();
        let sk = signing_key();
        let out_dir = tempfile::tempdir().unwrap();

        let pkg_a = out_dir.path().join("a.lunpkg");
        let pkg_b = out_dir.path().join("b.lunpkg");
        write_lunpkg(staging.path(), &manifest, &sk, &pkg_a).unwrap();
        write_lunpkg(staging.path(), &manifest, &sk, &pkg_b).unwrap();

        let a = fs::read(&pkg_a).unwrap();
        let b = fs::read(&pkg_b).unwrap();
        assert_eq!(a, b, "the same inputs must produce byte-identical archives");
    }

    #[test]
    fn reserved_entry_in_staging_is_rejected() {
        let staging = tempfile::tempdir().unwrap();
        make_staging(staging.path());
        // A stray manifest.toml in staging would collide with the one the
        // writer adds; the writer refuses rather than silently overwriting.
        fs::write(staging.path().join(MANIFEST_FILE), b"stray").unwrap();
        let recipe = recipe_with_caps("");
        let manifest = synthesize_manifest(&recipe, &collection()).unwrap();
        let sk = signing_key();
        let out = tempfile::tempdir().unwrap().path().join("x.lunpkg");
        assert!(matches!(
            write_lunpkg(staging.path(), &manifest, &sk, &out),
            Err(WriteError::ReservedEntry(_))
        ));
    }

    #[test]
    fn reserved_directory_prefix_in_staging_is_rejected() {
        let staging = tempfile::tempdir().unwrap();
        make_staging(staging.path());
        // A directory named like a reserved file (with a child) would otherwise
        // pass the exact-name check and produce an un-installable package.
        fs::create_dir_all(staging.path().join(SIGNATURE_FILE)).unwrap();
        fs::write(staging.path().join(SIGNATURE_FILE).join("x"), b"sneaky").unwrap();
        let recipe = recipe_with_caps("");
        let manifest = synthesize_manifest(&recipe, &collection()).unwrap();
        let sk = signing_key();
        let out = tempfile::tempdir().unwrap().path().join("x.lunpkg");
        assert!(matches!(
            write_lunpkg(staging.path(), &manifest, &sk, &out),
            Err(WriteError::ReservedEntry(_))
        ));
    }

    #[test]
    #[cfg(unix)]
    fn symlink_in_staging_is_rejected() {
        use std::os::unix::fs::symlink;
        let staging = tempfile::tempdir().unwrap();
        make_staging(staging.path());
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret"), b"OUT").unwrap();
        symlink(outside.path().join("secret"), staging.path().join("bin/link")).unwrap();

        let recipe = recipe_with_caps("");
        let manifest = synthesize_manifest(&recipe, &collection()).unwrap();
        let sk = signing_key();
        let out = tempfile::tempdir().unwrap().path().join("x.lunpkg");
        assert!(matches!(
            write_lunpkg(staging.path(), &manifest, &sk, &out),
            Err(WriteError::Symlink(_))
        ));
    }
}
