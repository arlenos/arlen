//! Reclaiming the space a bottle's shader caches take, without reaching past it.
//!
//! WHAT IS ACTUALLY CACHE, and it is a shorter list than the panel's wording
//! suggests. A prefix accumulates three kinds of regenerable file:
//!
//! - `*.dxvk-cache`, DXVK's pipeline state cache, written beside the program that
//!   produced it. Regenerated on the next run, at the cost of some stutter.
//! - `D3DSCache`, the Direct3D shader cache Windows itself keeps under a user's
//!   `AppData/Local`. Same bargain.
//! - `drive_c/windows/temp`, which is where installers leave what they unpacked.
//!   Not a cache in the shader sense, but nothing may depend on it surviving a
//!   reboot, which is the property that makes it safe to remove.
//!
//! FONT CACHES ARE NOT HERE, and their absence is the honest part. Wine's font
//! handling lives in the registry rather than in files under the prefix, so there
//! is nothing on disk to clear; a sweep that claimed to clear them would be
//! reporting work it did not do. The panel's wording is older than this module.
//!
//! SYMLINKS ARE NEVER FOLLOWED, and that is the load-bearing rule rather than a
//! precaution. A Wine prefix is FULL of links that leave it - `crate::sever` exists
//! because `wineboot` writes eight of them into every new prefix, pointing at the
//! person's real Documents, Downloads and Desktop. A recursive delete that followed
//! a link would walk straight out of the bottle and into the home directory it was
//! built to keep programs away from. Every step of the walk reads
//! `symlink_metadata` and steps over anything that is not a real directory.

use std::path::{Path, PathBuf};

/// What a sweep removed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cleared {
    /// How many bytes the removed files held.
    pub bytes: u64,
    /// How many files were removed.
    pub files: usize,
}

/// Directories under the prefix that hold nothing but regenerable data, relative
/// to the prefix root. Each is removed whole.
///
/// `D3DSCache` sits under a user directory whose name is the Wine user, so it is
/// found by walking rather than named here.
const CACHE_DIRS: &[&str] = &["drive_c/windows/temp"];

/// Files whose extension says they are a regenerable cache, found anywhere under
/// the prefix. DXVK writes its cache next to the program, so there is no one place
/// to look.
const CACHE_EXTENSIONS: &[&str] = &["dxvk-cache"];

/// Directory names that are caches wherever they appear under a user's data.
const CACHE_DIR_NAMES: &[&str] = &["D3DSCache"];

/// Remove the regenerable caches inside one prefix.
///
/// Errors are not returned per file: a cache file that will not delete is one the
/// person keeps, not a reason to abandon a sweep that has already freed the rest.
/// What comes back is what was actually removed, which is the number a surface may
/// state.
pub fn clear_caches(prefix_root: &Path) -> Cleared {
    let mut cleared = Cleared::default();
    if !prefix_root.is_dir() {
        return cleared;
    }
    for rel in CACHE_DIRS {
        let dir = prefix_root.join(rel);
        // The contents rather than the directory: `windows/temp` is a directory
        // Windows programs expect to exist, and removing it makes an installer
        // fail rather than run faster.
        empty_directory(&dir, &mut cleared);
    }
    walk(prefix_root, &mut cleared);
    cleared
}

/// Walk one directory, removing cache files and cache directories under it.
///
/// Never follows a symlink, in either the recursion or the removal: see the module
/// header for why that is the whole design and not a detail.
fn walk(dir: &Path, cleared: &mut Cleared) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_symlink() {
            continue;
        }
        if meta.is_dir() {
            if is_cache_dir(&path) {
                remove_directory(&path, cleared);
            } else {
                walk(&path, cleared);
            }
        } else if meta.is_file() && is_cache_file(&path) && std::fs::remove_file(&path).is_ok() {
            cleared.bytes += meta.len();
            cleared.files += 1;
        }
    }
}

/// Whether a directory name marks it as a cache.
fn is_cache_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| CACHE_DIR_NAMES.contains(&n))
}

/// Whether a file's name marks it as a regenerable cache.
///
/// Matched on the whole suffix rather than through `Path::extension`, which reads
/// `game.dxvk-cache` as the extension `dxvk-cache` only because the name has no
/// other dot - `Half-Life.dxvk-cache` would answer the same, but a name like
/// `v1.2.dxvk-cache` is exactly the shape that makes extension parsing a guess.
fn is_cache_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| CACHE_EXTENSIONS.iter().any(|e| n.ends_with(&format!(".{e}"))))
}

/// Remove a directory and everything real under it, counting as it goes.
fn remove_directory(dir: &Path, cleared: &mut Cleared) {
    empty_directory(dir, cleared);
    let _ = std::fs::remove_dir(dir);
}

/// Remove everything inside a directory, leaving the directory itself.
fn empty_directory(dir: &Path, cleared: &mut Cleared) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_symlink() {
            // Removed as a link, never followed: deleting the link frees the
            // prefix of it without touching whatever it pointed at.
            if std::fs::remove_file(&path).is_ok() {
                cleared.files += 1;
            }
            continue;
        }
        if meta.is_dir() {
            remove_directory(&path, cleared);
        } else if std::fs::remove_file(&path).is_ok() {
            cleared.bytes += meta.len();
            cleared.files += 1;
        }
    }
}

/// How many bytes a bottle's prefix holds.
///
/// MEASURED, and `None` when there is no prefix to measure - a bottle that was
/// made and never booted has no size, which is a different thing from being empty
/// and must not read as zero.
///
/// Never follows a symlink, for the reason the sweep gives at the top of this
/// file: a prefix is full of links into the person's home, and counting through
/// one would report the size of their Documents folder as the size of a Windows
/// app. Link sizes themselves are not counted either; what is being answered is
/// how much disk this bottle would give back.
pub fn prefix_bytes(prefix_root: &Path) -> Option<u64> {
    if !prefix_root.is_dir() {
        return None;
    }
    let mut total = 0u64;
    let mut stack = vec![prefix_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_symlink() {
                continue;
            }
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                total = total.saturating_add(meta.len());
            }
        }
    }
    Some(total)
}

/// The prefix directories a sweep would look at, for a caller that wants to say
/// what it is about to do. Absolute, under the given prefix.
pub fn cache_locations(prefix_root: &Path) -> Vec<PathBuf> {
    CACHE_DIRS.iter().map(|d| prefix_root.join(d)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("arlen-caches-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_sweep_removes_shader_caches_and_leaves_the_program_alone() {
        let prefix = scratch("shader");
        let game = prefix.join("drive_c/Program Files/Game");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("game.exe"), vec![0u8; 16]).unwrap();
        std::fs::write(game.join("game.dxvk-cache"), vec![0u8; 64]).unwrap();

        let cleared = clear_caches(&prefix);
        assert_eq!(cleared.files, 1);
        assert_eq!(cleared.bytes, 64);
        assert!(game.join("game.exe").is_file(), "the program is not a cache");
        assert!(!game.join("game.dxvk-cache").exists());
    }

    #[test]
    fn windows_temp_is_emptied_and_kept() {
        let prefix = scratch("temp");
        let temp = prefix.join("drive_c/windows/temp");
        std::fs::create_dir_all(temp.join("installer")).unwrap();
        std::fs::write(temp.join("installer/setup.dat"), vec![0u8; 32]).unwrap();

        let cleared = clear_caches(&prefix);
        assert_eq!(cleared.bytes, 32);
        assert!(temp.is_dir(), "a program that writes to TEMP expects it to be there");
        assert!(!temp.join("installer").exists());
    }

    #[test]
    fn a_link_out_of_the_prefix_is_cut_and_never_walked() {
        let outside = scratch("outside");
        // Named so the sweep WOULD take them if it ever walked in here: a bare
        // thesis.txt proves nothing, because nothing in the rules matches it and
        // the test would pass with the symlink guard deleted.
        std::fs::write(outside.join("thesis.dxvk-cache"), vec![0u8; 128]).unwrap();
        std::fs::create_dir_all(outside.join("D3DSCache")).unwrap();

        let prefix = scratch("links");
        let temp = prefix.join("drive_c/windows/temp");
        std::fs::create_dir_all(&temp).unwrap();
        std::os::unix::fs::symlink(&outside, temp.join("Documents")).unwrap();

        // And the same link where a walk would meet it rather than an empty-out.
        let user = prefix.join("drive_c/users/arlen");
        std::fs::create_dir_all(&user).unwrap();
        std::os::unix::fs::symlink(&outside, user.join("Documents")).unwrap();

        clear_caches(&prefix);
        assert!(
            outside.join("thesis.dxvk-cache").is_file() && outside.join("D3DSCache").is_dir(),
            "a sweep that followed a prefix link would delete the home directory it \
             points at - this is the whole reason the walk reads symlink_metadata"
        );
        assert!(!temp.join("Documents").exists(), "the link itself is cleared");
        assert!(
            user.join("Documents").exists(),
            "outside the emptied directories a link is stepped over, not removed"
        );
    }

    #[test]
    fn a_d3d_cache_directory_goes_wherever_it_sits() {
        let prefix = scratch("d3d");
        let cache = prefix.join("drive_c/users/arlen/AppData/Local/D3DSCache/abc");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("shaders.bin"), vec![0u8; 48]).unwrap();

        let cleared = clear_caches(&prefix);
        assert_eq!(cleared.bytes, 48);
        assert!(!prefix
            .join("drive_c/users/arlen/AppData/Local/D3DSCache")
            .exists());
    }

    #[test]
    fn a_prefix_reports_its_size_and_an_absent_one_reports_nothing() {
        let outside = scratch("size-outside");
        std::fs::write(outside.join("thesis.pdf"), vec![0u8; 5000]).unwrap();

        let prefix = scratch("size");
        let app = prefix.join("drive_c/Program Files/Game");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("game.exe"), vec![0u8; 300]).unwrap();
        std::fs::write(app.join("data.pak"), vec![0u8; 700]).unwrap();
        // The shell folder wineboot leaves pointing at the person's real files.
        std::os::unix::fs::symlink(&outside, app.join("Documents")).unwrap();

        assert_eq!(
            prefix_bytes(&prefix),
            Some(1000),
            "counting through the link would report the size of somebody's \
             Documents folder as the size of a Windows app"
        );
        assert_eq!(
            prefix_bytes(Path::new("/nonexistent/prefix")),
            None,
            "a bottle that was never booted has no size, which is not zero"
        );
    }

    #[test]
    fn a_prefix_that_is_not_there_clears_nothing_rather_than_failing() {
        let cleared = clear_caches(Path::new("/nonexistent/prefix"));
        assert_eq!(cleared, Cleared::default());
    }
}
