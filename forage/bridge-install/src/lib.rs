//! Install the two halves of a foreign-app bridge from a fetched recipe source
//! (`foreign-app-bridges.md` §4). A bridge recipe's `[install]` manifest names an
//! Arlen side (the `entities.toml` schema + `bridge.toml` mapping that register the
//! bridge with the ingest daemon) and a foreign side (the plugin files dropped into
//! the foreign app's own config dir). This crate does the path-safe copy of both
//! out of the fetched, verified recipe source into their destinations.
//!
//! The security floor: every declared file is confined to the source tree on read
//! (declared paths are re-validated safe-relative and symlinks are refused, so a
//! recipe cannot exfiltrate `/etc/passwd` by naming a symlink) and to its
//! destination on write (safe-relative + destination-rooted, so it cannot escape
//! the bridge dir or the foreign plugin dir). A pre-flight pass validates every
//! path and source file before anything is written, so a bad manifest fails without
//! leaving a partial install; whatever a late I/O error does write is reported for
//! the caller to roll back.

use arlen_forage_recipe::Install;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

/// The canonical Arlen-side directory for an installed bridge's registration files,
/// `$XDG_DATA_HOME/arlen/bridges/<recipe_id>/` (else `$HOME/.local/share/...`). One
/// bridge-ingest daemon instance is pointed at the `bridge.toml` under it. Returns
/// `None` when neither env var is set (the caller fails closed rather than guessing).
pub fn arlen_bridge_dir(recipe_id: &str) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))?;
    Some(base.join("arlen").join("bridges").join(recipe_id))
}

/// What a bridge install placed, so the caller can roll it back (on a later step's
/// failure) and record it for the revocable grant.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstalledBridge {
    /// Absolute paths written under the Arlen bridge dir.
    pub arlen_files: Vec<PathBuf>,
    /// Absolute paths written under the foreign app's plugin dir.
    pub foreign_files: Vec<PathBuf>,
}

/// A bridge install failure. Every variant leaves the destinations unchanged (a
/// pre-flight failure) or names what was written (a write failure) so the caller
/// can roll back.
#[derive(Debug, thiserror::Error)]
pub enum BridgeInstallError {
    /// A declared file is not a safe relative path (absolute or `..`).
    #[error("unsafe install path: {0}")]
    UnsafePath(String),
    /// A declared source file is missing or is not a regular file (e.g. a symlink,
    /// which is refused so a recipe cannot read outside its own source tree).
    #[error("source file is missing or not a regular file: {0}")]
    BadSource(String),
    /// An I/O error during the copy; `wrote` is what had already been placed.
    #[error("install I/O error at {path}: {source}")]
    Io {
        /// The path being written when the error occurred.
        path: String,
        /// The underlying error.
        source: std::io::Error,
        /// What was written before the error, for rollback.
        wrote: InstalledBridge,
    },
}

/// Namespaces a bridge may never claim: a delegated grant for one of these would
/// let a community bridge write the OS's own or the cross-app shared graph. Mirrors
/// the knowledge daemon's `RESERVED_NAMESPACES`; that daemon's `NamespaceGrant::new`
/// is the AUTHORITATIVE gate (it re-checks every write), so this is a fail-fast
/// defense-in-depth reject at install time, not the security boundary.
const RESERVED_NAMESPACES: &[&str] = &["system", "shared"];

/// A failure reading or validating a bridge's declared namespace.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NamespaceError {
    /// The `entities.toml` could not be parsed.
    #[error("entities.toml parse error: {0}")]
    Parse(String),
    /// No top-level `namespace` field.
    #[error("entities.toml has no `namespace`")]
    Missing,
    /// The namespace is empty, reserved (`system`/`shared`), or malformed.
    #[error("invalid bridge namespace: {0}")]
    Invalid(String),
}

/// The minimal shape of a bridge `entities.toml`: only the top-level `namespace` is
/// read here (the entity type definitions are the ingest daemon's concern).
#[derive(Debug, serde::Deserialize)]
struct EntitiesHeader {
    #[serde(default)]
    namespace: Option<String>,
}

/// Read and validate a bridge's delegated namespace from its `entities.toml`. The
/// namespace (e.g. `md.obsidian`) is what the install grants in the bridge-ingest
/// profile's `delegated_namespaces`. Validation is intentionally light and
/// fail-closed - the knowledge daemon's `NamespaceGrant` is the real gate; here we
/// only reject an obviously-bad namespace before provisioning a profile from it: a
/// reserved prefix (`system`/`shared`), an empty value, or a non-`[a-z0-9.-]` id.
pub fn bridge_namespace(entities_toml: &str) -> Result<String, NamespaceError> {
    let header: EntitiesHeader =
        toml::from_str(entities_toml).map_err(|e| NamespaceError::Parse(e.to_string()))?;
    let ns = header.namespace.ok_or(NamespaceError::Missing)?;
    if !is_valid_delegated_namespace(&ns) {
        return Err(NamespaceError::Invalid(ns));
    }
    Ok(ns)
}

/// The canonical bridge-ingest identity: one shared FirstParty daemon identity that
/// every installed bridge writes under, its profile accumulating each bridge's
/// delegated namespace. Matches `sdk/permissions/identity.rs` + the FirstParty
/// tier entry, so a written `bridge-ingest.toml` is the profile the daemon loads.
pub const BRIDGE_INGEST_APP_ID: &str = "bridge-ingest";

/// A failure provisioning the bridge-ingest profile.
#[derive(Debug, thiserror::Error)]
pub enum ProvisionError {
    /// The namespace is empty, reserved, or malformed.
    #[error("invalid bridge namespace: {0}")]
    InvalidNamespace(String),
    /// The profile path could not be resolved.
    #[error("profile path: {0}")]
    Path(String),
    /// The existing profile could not be parsed.
    #[error("existing bridge-ingest profile parse error: {0}")]
    Parse(String),
    /// An I/O error reading or writing the profile.
    #[error("profile I/O error at {path}: {source}")]
    Io {
        /// The path being read or written.
        path: String,
        /// The underlying error.
        source: std::io::Error,
    },
}

/// Add `namespace` to a profile's `graph.delegated_namespaces`, idempotently.
/// Returns `true` if it was added, `false` if already present. Pure - the caller
/// persists. The namespace must already be validated ([`bridge_namespace`] /
/// [`is_valid_delegated_namespace`]); this only mutates.
pub fn add_delegated_namespace(
    profile: &mut arlen_permissions::PermissionProfile,
    namespace: &str,
) -> bool {
    if profile.graph.delegated_namespaces.iter().any(|n| n == namespace) {
        return false;
    }
    profile.graph.delegated_namespaces.push(namespace.to_string());
    true
}

/// Remove `namespace` from a profile's `graph.delegated_namespaces`, idempotently.
/// Returns `true` if it was removed, `false` if it was already absent. Pure - the
/// caller persists. The rollback complement of [`add_delegated_namespace`]: when a
/// batch install fails after a bridge granted its namespace, the transaction undoes
/// the grant so no namespace is left behind with no bridge behind it.
pub fn remove_delegated_namespace(
    profile: &mut arlen_permissions::PermissionProfile,
    namespace: &str,
) -> bool {
    let before = profile.graph.delegated_namespaces.len();
    profile.graph.delegated_namespaces.retain(|n| n != namespace);
    profile.graph.delegated_namespaces.len() != before
}

/// A fresh bridge-ingest profile (FirstParty, no scope but the one namespace we add
/// next). Built by deserializing the minimal `[info]` so all other sections take
/// their serde defaults.
fn fresh_bridge_ingest_profile() -> arlen_permissions::PermissionProfile {
    toml::from_str("[info]\napp_id = \"bridge-ingest\"\ntier = \"first-party\"\n")
        .expect("the minimal bridge-ingest profile is valid")
}

/// Provision the shared bridge-ingest permission profile with a bridge's delegated
/// namespace: load the existing `bridge-ingest.toml` (or start a fresh FirstParty
/// one), add `namespace` to `graph.delegated_namespaces` idempotently, and write it
/// back atomically. The knowledge write path re-validates each namespace through
/// `NamespaceGrant::new`, and the permission watcher reloads on the file change.
/// Returns `true` if the profile changed (a new namespace was granted).
///
/// Writes the USER-tier profile (`~/.config/permissions/bridge-ingest.toml`, or
/// `$ARLEN_PERMISSIONS_DIR`), which is what a user-installed bridge grants: the user
/// authorising a bridge to write their own graph, revocable via the same file.
pub fn provision_bridge_namespace(namespace: &str) -> Result<bool, ProvisionError> {
    if !is_valid_delegated_namespace(namespace) {
        return Err(ProvisionError::InvalidNamespace(namespace.to_string()));
    }
    let path = arlen_permissions::profile_path(BRIDGE_INGEST_APP_ID)
        .map_err(|e| ProvisionError::Path(e.to_string()))?;

    let mut profile = match std::fs::read_to_string(&path) {
        Ok(text) => toml::from_str(&text).map_err(|e| ProvisionError::Parse(e.to_string()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => fresh_bridge_ingest_profile(),
        Err(source) => {
            return Err(ProvisionError::Io { path: path.display().to_string(), source })
        }
    };

    if !add_delegated_namespace(&mut profile, namespace) {
        return Ok(false); // already granted; nothing to write
    }

    let text = toml::to_string(&profile).expect("a PermissionProfile serializes to TOML");
    write_atomic(&path, text.as_bytes())?;
    Ok(true)
}

/// Remove a bridge's delegated namespace from the shared bridge-ingest profile: the
/// rollback complement of [`provision_bridge_namespace`]. Load the profile, drop
/// `namespace` idempotently, and write it back atomically. Returns `true` if the
/// profile changed (the namespace was present and removed), `false` if it was absent
/// (or there is no profile). Used when a batch install unwinds a bridge whose grant
/// this same transaction just added, so a failed install never leaves a live grant.
/// A missing profile is a no-op (nothing to revoke), not an error.
pub fn deprovision_bridge_namespace(namespace: &str) -> Result<bool, ProvisionError> {
    let path = arlen_permissions::profile_path(BRIDGE_INGEST_APP_ID)
        .map_err(|e| ProvisionError::Path(e.to_string()))?;

    let mut profile: arlen_permissions::PermissionProfile = match std::fs::read_to_string(&path) {
        Ok(text) => toml::from_str(&text).map_err(|e| ProvisionError::Parse(e.to_string()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(source) => {
            return Err(ProvisionError::Io { path: path.display().to_string(), source })
        }
    };

    if !remove_delegated_namespace(&mut profile, namespace) {
        return Ok(false); // already absent; nothing to write
    }

    let text = toml::to_string(&profile).expect("a PermissionProfile serializes to TOML");
    write_atomic(&path, text.as_bytes())?;
    Ok(true)
}

/// Write `bytes` to `path` atomically (sibling temp + rename), creating the parent
/// directory. Atomic so a concurrent daemon read never sees a half-written profile.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ProvisionError> {
    let io = |p: &Path, source: std::io::Error| ProvisionError::Io {
        path: p.display().to_string(),
        source,
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, bytes).map_err(|e| io(&tmp, e))?;
    std::fs::rename(&tmp, path).map_err(|e| io(path, e))
}

/// Whether `ns` is a well-formed, non-reserved delegated namespace.
fn is_valid_delegated_namespace(ns: &str) -> bool {
    if ns.is_empty() {
        return false;
    }
    // Charset: lowercase reverse-DNS-ish, no path or wildcard characters.
    if !ns
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
    {
        return false;
    }
    // No empty dot-segments (e.g. `md..obsidian`, a leading/trailing dot).
    if ns.split('.').any(|seg| seg.is_empty()) {
        return false;
    }
    // Reserved: the exact root or any sub-namespace of it (`system`, `system.x`).
    let first = ns.split('.').next().unwrap_or(ns);
    !RESERVED_NAMESPACES.contains(&first)
}

/// A failure resolving a `foreign_side.into` template.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TemplateError {
    /// The template does not start with a `$TOKEN` anchor.
    #[error("template must start with a $TOKEN anchor: {0}")]
    NoAnchor(String),
    /// The anchor token has no value in the resolved token set.
    #[error("unknown template token: ${0}")]
    UnknownToken(String),
    /// The anchor token resolved to a relative path (the anchor must be absolute so
    /// the foreign side lands somewhere the app owns).
    #[error("template anchor ${0} is not an absolute path")]
    AnchorNotAbsolute(String),
    /// A segment after the anchor is unsafe (empty, `.`, `..` or contains `$`).
    #[error("unsafe template segment: {0}")]
    UnsafeSegment(String),
}

/// Resolve a `foreign_side.into` template (e.g. `"$VAULT/.obsidian/plugins/x/"`) to
/// a concrete absolute destination, given the token values discovered for the
/// foreign app (e.g. `VAULT` -> the user's Obsidian vault path). The template MUST
/// begin with a `$TOKEN` anchor that resolves to an absolute path; every remaining
/// segment must be safe (non-empty, not `.`/`..`, no further `$`). The result is
/// therefore always UNDER the anchor - the confinement that keeps a bridge's
/// foreign side inside a directory the app itself owns, never an arbitrary path.
pub fn resolve_foreign_dest(
    template: &str,
    tokens: &HashMap<String, PathBuf>,
) -> Result<PathBuf, TemplateError> {
    let trimmed = template.trim().trim_end_matches('/');
    let mut segments = trimmed.split('/');
    let anchor_seg = segments.next().unwrap_or("");
    let name = anchor_seg
        .strip_prefix('$')
        .ok_or_else(|| TemplateError::NoAnchor(template.to_string()))?;
    if name.is_empty() {
        return Err(TemplateError::NoAnchor(template.to_string()));
    }
    let anchor = tokens
        .get(name)
        .ok_or_else(|| TemplateError::UnknownToken(name.to_string()))?;
    if !anchor.is_absolute() {
        return Err(TemplateError::AnchorNotAbsolute(name.to_string()));
    }
    let mut out = anchor.clone();
    for seg in segments {
        if seg.is_empty() || seg == "." || seg == ".." || seg.contains('$') {
            return Err(TemplateError::UnsafeSegment(seg.to_string()));
        }
        out.push(seg);
    }
    Ok(out)
}

/// Whether `p` is a safe relative path: non-empty, not absolute, no `..` component.
fn is_safe_relative(p: &Path) -> bool {
    let mut any = false;
    for c in p.components() {
        any = true;
        match c {
            Component::Normal(_) | Component::CurDir => {}
            _ => return false,
        }
    }
    any
}

/// Verify each declared file is safe-relative and its source is a real regular file
/// (not a symlink), before any write happens.
fn preflight(source_dir: &Path, files: &[PathBuf]) -> Result<(), BridgeInstallError> {
    for f in files {
        if !is_safe_relative(f) {
            return Err(BridgeInstallError::UnsafePath(f.display().to_string()));
        }
        let src = source_dir.join(f);
        // symlink_metadata does NOT follow the final symlink, so a symlink source
        // reports its own (symlink) type and is refused by the is_file() check.
        match std::fs::symlink_metadata(&src) {
            Ok(m) if m.file_type().is_file() => {}
            _ => return Err(BridgeInstallError::BadSource(src.display().to_string())),
        }
    }
    Ok(())
}

/// Copy `files` from `source_dir` to `dest_dir`, recording each written path into
/// `into`. Paths were pre-flighted, so a failure here is a real I/O error.
fn copy_into(
    source_dir: &Path,
    dest_dir: &Path,
    files: &[PathBuf],
    record: &mut dyn FnMut(&mut InstalledBridge, PathBuf),
    installed: &mut InstalledBridge,
) -> Result<(), (String, std::io::Error)> {
    for f in files {
        let src = source_dir.join(f);
        let dst = dest_dir.join(f);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| (parent.display().to_string(), e))?;
        }
        let bytes = std::fs::read(&src).map_err(|e| (src.display().to_string(), e))?;
        std::fs::write(&dst, &bytes).map_err(|e| (dst.display().to_string(), e))?;
        record(installed, dst);
    }
    Ok(())
}

/// The outcome of a single bridge install: the files placed and whether the shared
/// bridge-ingest profile gained the namespace (false if it already had it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeInstallResult {
    /// The Arlen-side + foreign-side files written.
    pub installed: InstalledBridge,
    /// Whether the bridge-ingest profile changed (a new delegated namespace).
    pub namespace_granted: bool,
}

/// A one-bridge install failure. Both variants name what was already placed so the
/// caller can roll the whole bridge (and any sibling bridges in the same
/// transaction) back.
#[derive(Debug, thiserror::Error)]
pub enum InstallBridgeError {
    /// The two-halves copy failed; nothing that matters was placed (the copy
    /// self-reports partial writes inside its own error) and no namespace was added.
    #[error(transparent)]
    Copy(#[from] BridgeInstallError),
    /// The copy succeeded but provisioning the profile failed; `installed` is the
    /// placed files, to be rolled back (the namespace was NOT added).
    #[error("provisioned failed after copy: {source}")]
    Provision {
        /// The provisioning error.
        source: ProvisionError,
        /// The files placed by the (successful) copy, for rollback.
        installed: InstalledBridge,
    },
}

/// Install ONE bridge as a unit: copy both halves, then grant the namespace in the
/// bridge-ingest profile. The order matters - files first, profile second - so a
/// profile failure leaves only files (reported for rollback), never a namespace
/// grant with no bridge behind it. The caller (the forage CLI) owns the ONE user
/// consent (gated upstream) and the source fetch + dest/namespace resolution (via
/// [`resolve_foreign_dest`] / [`bridge_namespace`]); this is the transactional
/// mechanism, not the gate.
pub fn install_bridge(
    source_dir: &Path,
    install: &Install,
    arlen_bridge_dir: &Path,
    foreign_dest: &Path,
    namespace: &str,
) -> Result<BridgeInstallResult, InstallBridgeError> {
    let installed = install_bridge_halves(source_dir, install, arlen_bridge_dir, foreign_dest)?;
    match provision_bridge_namespace(namespace) {
        Ok(namespace_granted) => Ok(BridgeInstallResult { installed, namespace_granted }),
        Err(source) => Err(InstallBridgeError::Provision { source, installed }),
    }
}

/// Roll back an installed bridge's placed files (best-effort: a file already gone is
/// not an error). Used by the caller when a later step - a sibling bridge, the
/// profile, the daemon launch - fails and the transaction must unwind. The namespace
/// grant is a separate concern (the profile is only touched on the happy path).
pub fn rollback_bridge(installed: &InstalledBridge) {
    for f in installed.arlen_files.iter().chain(installed.foreign_files.iter()) {
        let _ = std::fs::remove_file(f);
    }
}

/// Install both halves of a bridge from a fetched recipe source.
///
/// `source_dir` is the verified recipe checkout, `install` its `[install]` manifest,
/// `arlen_bridge_dir` the destination for the Arlen-side registration files, and
/// `foreign_dest` the ALREADY-RESOLVED foreign plugin directory (template
/// resolution of `foreign_side.into`, e.g. `$VAULT`, is the caller's job; this
/// function receives a concrete path). Returns what was placed.
pub fn install_bridge_halves(
    source_dir: &Path,
    install: &Install,
    arlen_bridge_dir: &Path,
    foreign_dest: &Path,
) -> Result<InstalledBridge, BridgeInstallError> {
    // Pre-flight BOTH halves before writing anything, so a bad manifest cannot leave
    // a half-installed bridge.
    preflight(source_dir, &install.arlen_side)?;
    preflight(source_dir, &install.foreign_side.files)?;

    let mut installed = InstalledBridge::default();
    if let Err((path, source)) = copy_into(
        source_dir,
        arlen_bridge_dir,
        &install.arlen_side,
        &mut |i, p| i.arlen_files.push(p),
        &mut installed,
    ) {
        return Err(BridgeInstallError::Io { path, source, wrote: installed });
    }
    if let Err((path, source)) = copy_into(
        source_dir,
        foreign_dest,
        &install.foreign_side.files,
        &mut |i, p| i.foreign_files.push(p),
        &mut installed,
    ) {
        return Err(BridgeInstallError::Io { path, source, wrote: installed });
    }
    Ok(installed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_forage_recipe::{ForeignSide, Install};
    use std::fs;
    use std::sync::Mutex;

    /// Serializes the tests that mutate PROCESS env (`ARLEN_PERMISSIONS_DIR` /
    /// `XDG_DATA_HOME`): those vars are global, so a parallel run would let one
    /// test's value leak into another. Each such test holds this for its duration.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn install_manifest(arlen: &[&str], foreign: &[&str]) -> Install {
        Install {
            arlen_side: arlen.iter().map(PathBuf::from).collect(),
            foreign_side: ForeignSide {
                into: "$VAULT/.obsidian/plugins/x/".to_string(),
                files: foreign.iter().map(PathBuf::from).collect(),
            },
        }
    }

    fn seed(dir: &Path, rel: &str, contents: &str) {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, contents).unwrap();
    }

    #[test]
    fn installs_both_halves() {
        let src = tempfile::tempdir().unwrap();
        seed(src.path(), "entities.toml", "e");
        seed(src.path(), "bridge.toml", "b");
        seed(src.path(), "main.js", "js");
        seed(src.path(), "manifest.json", "{}");

        let arlen = tempfile::tempdir().unwrap();
        let foreign = tempfile::tempdir().unwrap();
        let manifest = install_manifest(&["entities.toml", "bridge.toml"], &["main.js", "manifest.json"]);

        let got = install_bridge_halves(src.path(), &manifest, arlen.path(), foreign.path()).unwrap();
        assert_eq!(got.arlen_files.len(), 2);
        assert_eq!(got.foreign_files.len(), 2);
        assert_eq!(fs::read_to_string(arlen.path().join("bridge.toml")).unwrap(), "b");
        assert_eq!(fs::read_to_string(foreign.path().join("main.js")).unwrap(), "js");
    }

    #[test]
    fn a_missing_source_file_fails_before_writing() {
        let src = tempfile::tempdir().unwrap();
        seed(src.path(), "entities.toml", "e");
        // bridge.toml is NOT seeded.
        let arlen = tempfile::tempdir().unwrap();
        let foreign = tempfile::tempdir().unwrap();
        let manifest = install_manifest(&["entities.toml", "bridge.toml"], &["main.js"]);

        let err = install_bridge_halves(src.path(), &manifest, arlen.path(), foreign.path()).unwrap_err();
        assert!(matches!(err, BridgeInstallError::BadSource(_)));
        // Pre-flight failed, so nothing was written (not even the present entities.toml).
        assert!(!arlen.path().join("entities.toml").exists());
    }

    #[test]
    fn a_symlink_source_is_refused() {
        let src = tempfile::tempdir().unwrap();
        // A recipe naming a symlink that points outside its own tree must not be
        // able to copy the target's contents out.
        let secret = src.path().join("secret.txt");
        fs::write(&secret, "top secret").unwrap();
        std::os::unix::fs::symlink(&secret, src.path().join("entities.toml")).unwrap();
        seed(src.path(), "bridge.toml", "b");
        seed(src.path(), "main.js", "js");

        let arlen = tempfile::tempdir().unwrap();
        let foreign = tempfile::tempdir().unwrap();
        let manifest = install_manifest(&["entities.toml", "bridge.toml"], &["main.js"]);

        let err = install_bridge_halves(src.path(), &manifest, arlen.path(), foreign.path()).unwrap_err();
        assert!(matches!(err, BridgeInstallError::BadSource(_)));
    }

    #[test]
    fn a_nested_relative_file_is_placed_under_its_dir() {
        let src = tempfile::tempdir().unwrap();
        seed(src.path(), "dist/main.js", "nested");
        let arlen = tempfile::tempdir().unwrap();
        let foreign = tempfile::tempdir().unwrap();
        let manifest = install_manifest(&[], &["dist/main.js"]);

        install_bridge_halves(src.path(), &manifest, arlen.path(), foreign.path()).unwrap();
        assert_eq!(fs::read_to_string(foreign.path().join("dist/main.js")).unwrap(), "nested");
    }

    #[test]
    fn install_bridge_copies_then_provisions_then_rolls_back() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let src = tempfile::tempdir().unwrap();
        seed(src.path(), "entities.toml", "namespace = \"md.obsidian\"\n");
        seed(src.path(), "bridge.toml", "b");
        seed(src.path(), "main.js", "js");
        let arlen = tempfile::tempdir().unwrap();
        let foreign = tempfile::tempdir().unwrap();
        let perms = tempfile::tempdir().unwrap();
        std::env::set_var("ARLEN_PERMISSIONS_DIR", perms.path());
        let manifest = install_manifest(&["entities.toml", "bridge.toml"], &["main.js"]);

        let res = install_bridge(src.path(), &manifest, arlen.path(), foreign.path(), "md.obsidian").unwrap();
        assert!(res.namespace_granted);
        assert_eq!(res.installed.arlen_files.len(), 2);
        assert_eq!(res.installed.foreign_files.len(), 1);
        assert!(arlen.path().join("bridge.toml").exists());
        assert!(foreign.path().join("main.js").exists());
        assert!(perms.path().join("bridge-ingest.toml").exists());

        // Rollback removes the placed files (the profile grant is left; a real
        // transaction would revoke it separately, but the files unwind cleanly).
        rollback_bridge(&res.installed);
        assert!(!arlen.path().join("bridge.toml").exists());
        assert!(!foreign.path().join("main.js").exists());
        std::env::remove_var("ARLEN_PERMISSIONS_DIR");
    }

    #[test]
    fn add_delegated_namespace_is_idempotent() {
        let mut profile: arlen_permissions::PermissionProfile =
            toml::from_str("[info]\napp_id = \"bridge-ingest\"\ntier = \"first-party\"\n").unwrap();
        assert!(add_delegated_namespace(&mut profile, "md.obsidian"));
        assert!(!add_delegated_namespace(&mut profile, "md.obsidian")); // already present
        assert!(add_delegated_namespace(&mut profile, "com.zotero"));
        assert_eq!(profile.graph.delegated_namespaces, vec!["md.obsidian", "com.zotero"]);
    }

    #[test]
    fn provision_creates_then_accumulates_the_profile() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("ARLEN_PERMISSIONS_DIR", dir.path());

        // First bridge: creates the profile.
        assert!(provision_bridge_namespace("md.obsidian").unwrap());
        let path = dir.path().join("bridge-ingest.toml");
        let p1: arlen_permissions::PermissionProfile =
            toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(p1.info.app_id, "bridge-ingest");
        assert_eq!(p1.graph.delegated_namespaces, vec!["md.obsidian"]);

        // Second bridge: accumulates (shared identity).
        assert!(provision_bridge_namespace("com.zotero").unwrap());
        let p2: arlen_permissions::PermissionProfile =
            toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(p2.graph.delegated_namespaces, vec!["md.obsidian", "com.zotero"]);

        // Re-provisioning the same namespace is a no-op (no change).
        assert!(!provision_bridge_namespace("md.obsidian").unwrap());

        // A reserved namespace is refused before any write.
        assert!(matches!(
            provision_bridge_namespace("system.core"),
            Err(ProvisionError::InvalidNamespace(_))
        ));
        std::env::remove_var("ARLEN_PERMISSIONS_DIR");
    }

    #[test]
    fn deprovision_removes_only_the_named_namespace() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("ARLEN_PERMISSIONS_DIR", dir.path());

        provision_bridge_namespace("md.obsidian").unwrap();
        provision_bridge_namespace("com.zotero").unwrap();

        // Removing one leaves the other (a shared-identity partial rollback).
        assert!(deprovision_bridge_namespace("md.obsidian").unwrap());
        let path = dir.path().join("bridge-ingest.toml");
        let p: arlen_permissions::PermissionProfile =
            toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(p.graph.delegated_namespaces, vec!["com.zotero"]);

        // Removing an absent namespace is a no-op, not an error.
        assert!(!deprovision_bridge_namespace("md.obsidian").unwrap());
        // No profile at all: also a clean no-op.
        std::fs::remove_file(&path).unwrap();
        assert!(!deprovision_bridge_namespace("com.zotero").unwrap());
        std::env::remove_var("ARLEN_PERMISSIONS_DIR");
    }

    #[test]
    fn reads_and_validates_the_bridge_namespace() {
        let entities = r#"
namespace = "md.obsidian"

[[entity]]
type = "Note"
"#;
        assert_eq!(bridge_namespace(entities).unwrap(), "md.obsidian");
    }

    #[test]
    fn a_missing_or_reserved_or_malformed_namespace_is_refused() {
        assert!(matches!(bridge_namespace("[[entity]]\ntype='x'").unwrap_err(), NamespaceError::Missing));
        // Reserved root and any sub-namespace of it.
        for ns in ["system", "system.core", "shared", "shared.person"] {
            let toml = format!("namespace = \"{ns}\"");
            assert!(
                matches!(bridge_namespace(&toml).unwrap_err(), NamespaceError::Invalid(_)),
                "{ns} must be reserved"
            );
        }
        // Malformed: wildcard, uppercase, empty segment.
        for ns in ["md.obsidian.*", "MD.Obsidian", "md..obsidian", ".md", ""] {
            let toml = format!("namespace = \"{ns}\"");
            assert!(
                matches!(bridge_namespace(&toml).unwrap_err(), NamespaceError::Invalid(_)),
                "{ns:?} must be invalid"
            );
        }
    }

    #[test]
    fn resolves_a_vault_anchored_template_under_the_anchor() {
        let mut tokens = HashMap::new();
        tokens.insert("VAULT".to_string(), PathBuf::from("/home/u/MyVault"));
        let got = resolve_foreign_dest("$VAULT/.obsidian/plugins/md-obsidian-bridge/", &tokens).unwrap();
        assert_eq!(got, PathBuf::from("/home/u/MyVault/.obsidian/plugins/md-obsidian-bridge"));
        // The result is always under the anchor.
        assert!(got.starts_with("/home/u/MyVault"));
    }

    #[test]
    fn template_resolution_is_confined_and_fails_closed() {
        let mut tokens = HashMap::new();
        tokens.insert("VAULT".to_string(), PathBuf::from("/home/u/MyVault"));
        // No anchor.
        assert_eq!(
            resolve_foreign_dest("/etc/passwd", &tokens),
            Err(TemplateError::NoAnchor("/etc/passwd".to_string()))
        );
        // Unknown token.
        assert!(matches!(
            resolve_foreign_dest("$HOME/x", &tokens),
            Err(TemplateError::UnknownToken(_))
        ));
        // Traversal after the anchor cannot escape.
        assert!(matches!(
            resolve_foreign_dest("$VAULT/../../etc/x", &tokens),
            Err(TemplateError::UnsafeSegment(_))
        ));
        // A relative anchor value is refused.
        let mut rel = HashMap::new();
        rel.insert("VAULT".to_string(), PathBuf::from("relative/vault"));
        assert!(matches!(
            resolve_foreign_dest("$VAULT/x", &rel),
            Err(TemplateError::AnchorNotAbsolute(_))
        ));
    }

    #[test]
    fn the_arlen_bridge_dir_is_recipe_scoped_under_xdg() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // With XDG_DATA_HOME set, the dir is <xdg>/arlen/bridges/<id>.
        std::env::set_var("XDG_DATA_HOME", "/tmp/xdg-test-data");
        let dir = arlen_bridge_dir("md.obsidian.bridge").unwrap();
        assert_eq!(dir, PathBuf::from("/tmp/xdg-test-data/arlen/bridges/md.obsidian.bridge"));
        std::env::remove_var("XDG_DATA_HOME");
    }
}
