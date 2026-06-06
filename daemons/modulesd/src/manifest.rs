/// Module discovery and tier classification.
///
/// Reads `manifest.toml` files from the system and user module
/// directories using `arlen-modules`, then classifies each module by
/// tier so the runtime knows which sandbox to apply.

use std::path::{Path, PathBuf};

use arlen_modules::{load_manifest, ModuleManifest, ModuleType};

use crate::error::{DaemonError, Result};

/// Sandbox tier the module runs in. Foundation §07 splits modules into
/// data-only (Tier 1, WASM) and UI-rendering (Tier 2, iframe). System
/// modules are not handled by the daemon at all and never appear here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// WASM Component, hosted in-process by the daemon.
    Wasm,
    /// Iframe, rendered in the host webview. Daemon brokers the
    /// postMessage capability checks but holds no DOM.
    Iframe,
}

/// One module's discovery record.
#[derive(Debug, Clone)]
pub struct ModuleRecord {
    pub manifest: ModuleManifest,
    pub root: PathBuf,
    pub tier: Tier,
}

impl ModuleRecord {
    pub fn id(&self) -> &str {
        &self.manifest.module.id
    }

    pub fn wasm_path(&self) -> PathBuf {
        self.root.join("module.wasm")
    }

    pub fn dist_dir(&self) -> PathBuf {
        self.root.join("dist")
    }
}

/// System search path for first-party modules installed by the OS.
pub fn system_modules_dir() -> PathBuf {
    PathBuf::from("/usr/share/arlen/modules")
}

/// User search path; matches `installd` write location.
pub fn user_modules_dir() -> PathBuf {
    if let Ok(p) = std::env::var("LUNARIS_USER_MODULES_DIR") {
        return PathBuf::from(p);
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("arlen/modules")
}

/// Walk both module directories and load every valid manifest. Invalid
/// manifests are logged and skipped (a single broken module must not
/// break discovery for everything else).
pub fn discover_all() -> Vec<ModuleRecord> {
    let mut out = Vec::new();
    for dir in [system_modules_dir(), user_modules_dir()] {
        if !dir.exists() {
            continue;
        }
        match scan_dir(&dir) {
            Ok(records) => out.extend(records),
            Err(err) => tracing::warn!(
                "modulesd: scan failed for {}: {}",
                dir.display(),
                err
            ),
        }
    }
    out
}

fn scan_dir(dir: &Path) -> Result<Vec<ModuleRecord>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = match entry {
            Ok(e) => e,
            Err(err) => {
                tracing::warn!("modulesd: skipping unreadable entry: {err}");
                continue;
            }
        };
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest_path = path.join("manifest.toml");
        if !manifest_path.exists() {
            continue;
        }
        match load_record(&path, &manifest_path) {
            Ok(record) => out.push(record),
            Err(err) => tracing::warn!(
                "modulesd: skipping {}: {}",
                path.display(),
                err
            ),
        }
    }
    Ok(out)
}

fn load_record(root: &Path, manifest_path: &Path) -> Result<ModuleRecord> {
    let manifest = load_manifest(manifest_path).map_err(|e| {
        DaemonError::ManifestInvalid {
            module_id: root
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            reason: e.to_string(),
        }
    })?;

    // System modules never go through the daemon. Surface that as a
    // discovery error so the caller can route them to the in-process
    // PluginManager instead.
    if matches!(manifest.module.module_type, ModuleType::System) {
        return Err(DaemonError::ManifestInvalid {
            module_id: manifest.module.id.clone(),
            reason: "system modules are not hosted by modulesd".into(),
        });
    }

    let tier = classify_tier(&manifest, root);

    Ok(ModuleRecord {
        manifest,
        root: root.to_path_buf(),
        tier,
    })
}

/// Tier picks itself from what's on disk: a `module.wasm` means Tier 1,
/// a `dist/` means Tier 2. A module with both is allowed and gets two
/// runtime instances; we report Tier 1 here and the caller spawns the
/// iframe alongside on demand.
fn classify_tier(manifest: &ModuleManifest, root: &Path) -> Tier {
    let _ = manifest;
    if root.join("module.wasm").exists() {
        Tier::Wasm
    } else {
        Tier::Iframe
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Helper: write a minimal valid manifest TOML.
    fn write_manifest(dir: &Path, id: &str, module_type: &str) {
        fs::create_dir_all(dir).unwrap();
        let toml = format!(
            r#"
[module]
id = "{id}"
name = "Test"
version = "1.0.0"
type = "{module_type}"
entry = "module.wasm"
"#
        );
        fs::write(dir.join("manifest.toml"), toml).unwrap();
        fs::write(dir.join("module.wasm"), b"\0asm\x01\0\0\0").unwrap();
    }

    #[test]
    fn discover_skips_directories_without_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir(tmp.path().join("not-a-module")).unwrap();
        let records = scan_dir(tmp.path()).unwrap();
        assert!(records.is_empty());
    }

    #[test]
    fn discover_finds_valid_third_party() {
        let tmp = tempfile::tempdir().unwrap();
        let module_dir = tmp.path().join("com.example.test");
        write_manifest(&module_dir, "com.example.test", "third-party");

        let records = scan_dir(tmp.path()).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id(), "com.example.test");
        assert_eq!(records[0].tier, Tier::Wasm);
    }

    #[test]
    fn discover_skips_system_modules() {
        let tmp = tempfile::tempdir().unwrap();
        let module_dir = tmp.path().join("system.calc");
        write_manifest(&module_dir, "system.calc", "system");

        // System modules error out in load_record, so scan_dir returns
        // an empty list (errors are logged and skipped).
        let records = scan_dir(tmp.path()).unwrap();
        assert!(records.is_empty());
    }

    #[test]
    fn classify_iframe_when_no_wasm() {
        let tmp = tempfile::tempdir().unwrap();
        let module_dir = tmp.path().join("com.example.ui");
        fs::create_dir_all(module_dir.join("dist")).unwrap();
        fs::write(
            module_dir.join("manifest.toml"),
            r#"
[module]
id = "com.example.ui"
name = "UI Test"
version = "1.0.0"
type = "third-party"
entry = "dist/index.html"
"#,
        )
        .unwrap();
        fs::write(module_dir.join("dist/index.html"), "<html/>").unwrap();

        let records = scan_dir(tmp.path()).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].tier, Tier::Iframe);
    }
}
