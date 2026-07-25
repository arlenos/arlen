//! The trash-execution core: move each operand into the freedesktop home trash and
//! capture the restorable inverse, honoring the `rm` directory semantics.
//!
//! Sync and self-contained so it is tested against a tempdir trash; the binary owns
//! the (async, best-effort) journaling of each captured inverse to the undo-signer.
//! A directory is trashed as a single subtree move, so `-r`/`-d` here gate WHETHER a
//! directory may be trashed (mirroring the unlink path), not how deep the walk goes.

use arlen_ai_undo_core::effect_model::{CanonicalPath, InverseReceipt};
use arlen_trash_rm_core::parse::RmInvocation;
use std::path::{Path, PathBuf};

/// The outcome of a trash run: the captured inverses (to journal) and per-operand
/// errors.
#[derive(Debug, Default)]
pub struct TrashReport {
    /// Each successfully trashed operand (as the user gave it) with its restorable
    /// inverse.
    pub trashed: Vec<(String, InverseReceipt)>,
    /// Operands that could not be trashed, paired with the reason.
    pub errors: Vec<(String, String)>,
}

impl TrashReport {
    /// The process exit code: 0 when every operand was handled, 1 on any error.
    pub fn exit_code(&self) -> i32 {
        if self.errors.is_empty() {
            0
        } else {
            1
        }
    }
}

/// Resolve an operand to the canonical original path undo restores to, WITHOUT
/// following a final symlink (so a symlink is trashed as itself). The parent is
/// canonicalized (resolving `..` and any symlinked ancestor); the final component
/// is joined verbatim. `None` for a path with no final component (`/`, `..`).
pub fn resolve_original(operand: &str) -> Option<CanonicalPath> {
    let p = Path::new(operand);
    let file_name = p.file_name()?;
    let parent = match p.parent() {
        Some(par) if !par.as_os_str().is_empty() => par.to_path_buf(),
        // No parent component (e.g. "foo"): resolve against the current directory.
        _ => PathBuf::from("."),
    };
    let parent_canon = std::fs::canonicalize(&parent).ok()?;
    parent_canon.join(file_name).to_str().and_then(CanonicalPath::new)
}

/// Trash every operand in `inv` into the freedesktop home trash, capturing a
/// [`InverseReceipt::RestoreFromTrash`] per success. A missing operand errors unless
/// `-f`; a directory needs `-r` or `-d` (else it errors, like the unlink path). A
/// cross-filesystem operand errors (the home trash cannot atomically move across a
/// mount boundary), never a silent fall-through to a copy.
pub fn execute_trash(inv: &RmInvocation) -> TrashReport {
    let mut report = TrashReport::default();
    let Some(trash) = arlen_freedesktop_trash::home_trash_dir() else {
        for path in &inv.paths {
            report.errors.push((path.clone(), "no home trash directory".to_string()));
        }
        return report;
    };
    let files = trash.join("files");
    let info = trash.join("info");
    for path in &inv.paths {
        // Existence and directory gating, matching the unlink path's semantics.
        match std::fs::symlink_metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if !inv.force {
                    report.errors.push((path.clone(), "no such file or directory".to_string()));
                }
                continue;
            }
            Err(e) => {
                report.errors.push((path.clone(), e.to_string()));
                continue;
            }
            Ok(meta) => {
                if meta.file_type().is_dir() && !inv.recursive && !inv.dir {
                    report.errors.push((path.clone(), "is a directory".to_string()));
                    continue;
                }
            }
        }
        let Some(original) = resolve_original(path) else {
            report.errors.push((path.clone(), "cannot resolve a canonical path".to_string()));
            continue;
        };
        if let Err(e) =
            std::fs::create_dir_all(&files).and_then(|()| std::fs::create_dir_all(&info))
        {
            report.errors.push((path.clone(), format!("trash unavailable: {e}")));
            continue;
        }
        let base = Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file");
        match arlen_freedesktop_trash::trash_into(&files, &info, base, original.as_str()) {
            Ok(slot) => {
                let (trashed, trash_info) = slot.into_parts();
                let inverse = InverseReceipt::RestoreFromTrash { original, trashed, trash_info };
                report.trashed.push((path.clone(), inverse));
            }
            Err(e) => report.errors.push((path.clone(), format!("{e:?}"))),
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_trash_rm_core::parse::parse_rm_args;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    // `execute_trash` reads XDG_DATA_HOME (process-global), so trash tests serialize.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn tmp() -> std::path::PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("trash-rm-cli-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }

    fn inv(v: &[&str]) -> RmInvocation {
        parse_rm_args(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
    }

    #[test]
    fn trashes_a_file_and_captures_a_restore_inverse() {
        let _g = ENV_LOCK.lock().unwrap();
        let root = tmp();
        let data_home = root.join("data");
        std::env::set_var("XDG_DATA_HOME", &data_home);
        let f = root.join("notes.txt");
        std::fs::write(&f, b"hi").unwrap();

        let report = execute_trash(&inv(&[f.to_str().unwrap()]));
        std::env::remove_var("XDG_DATA_HOME");

        assert_eq!(report.exit_code(), 0);
        assert!(!f.exists(), "the source moved out");
        assert_eq!(report.trashed.len(), 1);
        assert!(data_home.join("Trash/files/notes.txt").exists());
        assert!(data_home.join("Trash/info/notes.txt.trashinfo").exists());
        match &report.trashed[0].1 {
            InverseReceipt::RestoreFromTrash { original, trashed, .. } => {
                assert!(original.as_str().ends_with("/notes.txt"));
                assert!(trashed.as_str().ends_with("/Trash/files/notes.txt"));
            }
            other => panic!("expected RestoreFromTrash, got {other:?}"),
        }
    }

    #[test]
    fn a_directory_is_trashed_only_with_recursive_or_dir() {
        let _g = ENV_LOCK.lock().unwrap();
        let root = tmp();
        std::env::set_var("XDG_DATA_HOME", root.join("data"));
        let sub = root.join("proj");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("f"), b"x").unwrap();

        // Without -r/-d: an error, directory left in place.
        let report = execute_trash(&inv(&[sub.to_str().unwrap()]));
        assert_eq!(report.exit_code(), 1);
        assert!(sub.exists());
        // With -r: the whole subtree is trashed in one move.
        let report = execute_trash(&inv(&["-r", sub.to_str().unwrap()]));
        std::env::remove_var("XDG_DATA_HOME");
        assert_eq!(report.exit_code(), 0);
        assert!(!sub.exists());
        assert_eq!(report.trashed.len(), 1);
    }

    #[test]
    fn a_missing_operand_errors_unless_force() {
        let _g = ENV_LOCK.lock().unwrap();
        let root = tmp();
        std::env::set_var("XDG_DATA_HOME", root.join("data"));
        let gone = root.join("gone");
        assert_eq!(execute_trash(&inv(&[gone.to_str().unwrap()])).exit_code(), 1);
        assert_eq!(execute_trash(&inv(&["-f", gone.to_str().unwrap()])).exit_code(), 0);
        std::env::remove_var("XDG_DATA_HOME");
    }

    #[test]
    fn resolve_original_refuses_a_rootless_path() {
        assert!(resolve_original("/").is_none());
    }
}
