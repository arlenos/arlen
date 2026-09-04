//! Thumbnails for the picker's grid view.
//!
//! WHY THIS EXISTS NOW, and the reason it did not is worth keeping. The note on
//! the missing-command list said the picker "is confined to the daemon's cap-std
//! root, so it cannot read one itself". That is true of the DAEMON and not of
//! this process: `fs_commands` lists directories with plain `tokio::fs` over
//! absolute paths, which is the same reach a thumbnail needs. The reason
//! described a neighbour.
//!
//! Nothing here decodes an image. The bytes go to the sandboxed worker the file
//! manager already uses - a subprocess under Landlock and seccomp - because a
//! picker that decoded an untrusted image in its own address space would be the
//! widest attack surface in a dialog whose whole job is being shown files
//! somebody has not opened yet.
//!
//! Every piece is shared with the file manager rather than copied: one cache,
//! one supported-type test, one idea of what a failed decode looks like. Two
//! surfaces showing the same thumbnails had two chances to disagree about that
//! and now have none.

use std::path::{Path, PathBuf};

use arlen_file_browser_core::thumbnail_cache::{
    read_capped, thumbnail_data_url, ThumbnailCache, ThumbnailError, ThumbnailGenerator,
};

/// What the sandboxed image worker can decode.
///
/// Mirrors the picker frontend's own `THUMBNAILABLE`, and deliberately not the
/// file manager's wider set: no music cover art and no video frames here. The
/// picker shows a grid of files somebody is choosing between, and spawning a
/// video decoder per tile in a modal dialog is a cost with no matching value.
const EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "bmp", "webp"];

/// Whether a path is one the worker will decode.
#[must_use]
pub fn is_thumbnailable(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .is_some_and(|e| EXTENSIONS.contains(&e.as_str()))
}

/// The sandboxed worker: the `ARLEN_THUMBNAIL_SANDBOX_BIN` override for a dev
/// run, else the installed path. An absent binary makes generation fail, which
/// the tile renders as its icon.
fn sandbox_bin() -> PathBuf {
    std::env::var_os("ARLEN_THUMBNAIL_SANDBOX_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/usr/lib/arlen/libexec/arlen-thumbnail-sandbox"))
}

/// The shared cache: `$XDG_CACHE_HOME/arlen/thumbnails`.
///
/// The SAME directory the file manager writes, keyed by path and mtime, so a
/// picture already thumbnailed in one surface costs nothing in the other. That
/// sharing is the reason the key includes the mtime rather than the path alone.
fn cache_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("arlen").join("thumbnails"))
}

/// Reads the file and hands the bytes to the sandboxed decoder.
struct Sandboxed {
    bin: PathBuf,
}

impl ThumbnailGenerator for Sandboxed {
    fn generate(&self, source: &Path) -> Result<Vec<u8>, ThumbnailError> {
        let bytes = read_capped(source, arlen_ai_sandbox::MAX_BYTES)?;
        arlen_ai_sandbox::thumbnail(&self.bin, &bytes)
            .map_err(|e| ThumbnailError::Generate(e.to_string()))
    }
}

/// A data-URL thumbnail for `path`, or `None` when there is none to show.
///
/// `None` rather than an error for every failure: the grid falls back to the
/// file's icon, which is a complete tile rather than a broken one. A picker that
/// showed an error where a preview goes would make an unreadable image look like
/// a broken dialog.
#[tauri::command]
pub async fn picker_thumbnail(path: String) -> Result<Option<String>, String> {
    let Some(dir) = cache_dir() else {
        return Ok(None);
    };
    if !is_thumbnailable(Path::new(&path)) {
        return Ok(None);
    }
    // Off the async runtime: a miss spawns the worker and blocks on its output.
    tokio::task::spawn_blocking(move || {
        let cache = ThumbnailCache::new(dir);
        let generator = Sandboxed { bin: sandbox_bin() };
        thumbnail_data_url(&cache, &generator, Path::new(&path), is_thumbnailable)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_decodable_types_are_the_ones_the_grid_asks_about() {
        for name in ["a.png", "b.JPG", "c.jpeg", "d.gif", "e.bmp", "f.webp"] {
            assert!(is_thumbnailable(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn everything_else_keeps_its_icon() {
        // svg and ico are images the worker does not decode; a document is not
        // one at all. Each renders its icon rather than an empty tile.
        for name in ["a.svg", "b.ico", "c.avif", "d.pdf", "e", "f.png.txt"] {
            assert!(!is_thumbnailable(Path::new(name)), "{name}");
        }
    }

    #[tokio::test]
    async fn an_unthumbnailable_path_never_reaches_a_worker() {
        // No sandbox binary exists in a test run, so reaching one would fail
        // rather than answer `None`.
        assert_eq!(picker_thumbnail("/tmp/notes.txt".to_string()).await.unwrap(), None);
    }
}
