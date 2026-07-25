//! Execute the hard-unlink path (the scripted/`--purge`/non-interactive delete).
//!
//! POSIX `rm` semantics over the parsed operands: a file (or symlink) is unlinked;
//! a directory needs `-r` (remove the tree) or `-d` (remove an empty directory),
//! else it is an error; a missing operand is an error unless `-f`. A symlink is
//! removed via `lstat` so the LINK is unlinked, never its target followed. Per-
//! operand failures are collected (like `rm`, which continues) and set the exit
//! code. Real filesystem I/O, tested against tempdirs.

use crate::parse::RmInvocation;
use std::path::Path;

/// The result of a hard-unlink run: what was removed and the per-operand errors.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct UnlinkReport {
    /// Operands successfully unlinked, in order.
    pub removed: Vec<String>,
    /// Operands that could not be removed, paired with the reason.
    pub errors: Vec<(String, String)>,
}

impl UnlinkReport {
    /// The process exit code: 0 when every operand was handled, 1 on any error
    /// (matching `rm`).
    pub fn exit_code(&self) -> i32 {
        if self.errors.is_empty() {
            0
        } else {
            1
        }
    }
}

/// Hard-unlink every operand in `inv` per its flags. Does NOT prompt for `-i` (the
/// binary resolves interactive confirmation before calling); it applies the
/// remove semantics for `-r`/`-d`/`-f`.
pub fn execute_unlink(inv: &RmInvocation) -> UnlinkReport {
    let mut report = UnlinkReport::default();
    for path in &inv.paths {
        let p = Path::new(path);
        // lstat, so a symlink is a leaf we unlink, not a directory we recurse into.
        match std::fs::symlink_metadata(p) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if !inv.force {
                    report
                        .errors
                        .push((path.clone(), "no such file or directory".to_string()));
                }
            }
            Err(e) => report.errors.push((path.clone(), e.to_string())),
            Ok(meta) => {
                let result = if meta.file_type().is_dir() {
                    if inv.recursive {
                        std::fs::remove_dir_all(p)
                    } else if inv.dir {
                        // Empty-only; a non-empty directory errors here.
                        std::fs::remove_dir(p)
                    } else {
                        Err(std::io::Error::other("is a directory"))
                    }
                } else {
                    // A regular file or a symlink: unlink the entry itself.
                    std::fs::remove_file(p)
                };
                match result {
                    Ok(()) => report.removed.push(path.clone()),
                    Err(e) => report.errors.push((path.clone(), e.to_string())),
                }
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_rm_args;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn tmp() -> std::path::PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("trash-rm-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn inv(v: &[&str]) -> RmInvocation {
        parse_rm_args(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
    }

    #[test]
    fn removes_a_file() {
        let d = tmp();
        let f = d.join("x.txt");
        std::fs::write(&f, b"hi").unwrap();
        let r = execute_unlink(&inv(&[f.to_str().unwrap()]));
        assert_eq!(r.exit_code(), 0);
        assert!(!f.exists());
        assert_eq!(r.removed.len(), 1);
    }

    #[test]
    fn a_directory_needs_recursive_or_dir() {
        let d = tmp();
        let sub = d.join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("inner"), b"x").unwrap();
        // Without -r/-d: an error, directory left in place.
        let r = execute_unlink(&inv(&[sub.to_str().unwrap()]));
        assert_eq!(r.exit_code(), 1);
        assert!(sub.exists());
        // With -r: the whole tree goes.
        let r = execute_unlink(&inv(&["-r", sub.to_str().unwrap()]));
        assert_eq!(r.exit_code(), 0);
        assert!(!sub.exists());
    }

    #[test]
    fn dir_flag_removes_only_an_empty_directory() {
        let d = tmp();
        let empty = d.join("empty");
        std::fs::create_dir(&empty).unwrap();
        assert_eq!(execute_unlink(&inv(&["-d", empty.to_str().unwrap()])).exit_code(), 0);
        assert!(!empty.exists());
        // A non-empty dir with -d (not -r) errors.
        let full = d.join("full");
        std::fs::create_dir(&full).unwrap();
        std::fs::write(full.join("f"), b"x").unwrap();
        assert_eq!(execute_unlink(&inv(&["-d", full.to_str().unwrap()])).exit_code(), 1);
        assert!(full.exists());
    }

    #[test]
    fn missing_operand_errors_unless_force() {
        let d = tmp();
        let gone = d.join("gone");
        assert_eq!(execute_unlink(&inv(&[gone.to_str().unwrap()])).exit_code(), 1);
        // -f: a missing operand is a silent success.
        assert_eq!(execute_unlink(&inv(&["-f", gone.to_str().unwrap()])).exit_code(), 0);
    }

    #[test]
    fn a_symlink_is_unlinked_not_followed() {
        let d = tmp();
        let target = d.join("target.txt");
        std::fs::write(&target, b"keep me").unwrap();
        let link = d.join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let r = execute_unlink(&inv(&[link.to_str().unwrap()]));
        assert_eq!(r.exit_code(), 0);
        assert!(!link.exists(), "the link is gone");
        assert!(target.exists(), "the target is untouched");
    }
}
