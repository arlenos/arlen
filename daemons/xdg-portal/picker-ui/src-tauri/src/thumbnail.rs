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

    /// The real thing, end to end: a real PNG through the real sandboxed worker
    /// into a real cache, out as the data-URL a tile loads.
    ///
    /// `#[ignore]`d because it needs that worker built
    /// (`cargo build -p arlen-ai-sandbox --features thumbnail`), which a plain
    /// `cargo test` does not do. It takes the generator directly rather than
    /// through the command so it can name the binary without setting an
    /// environment variable its neighbours are reading at the same time.
    ///
    /// Run: `cargo test -- --ignored thumbnails_a_real_png`
    #[test]
    #[ignore = "needs the sandboxed worker built"]
    fn thumbnails_a_real_png_through_the_sandboxed_worker() {
        let bin = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../target/debug/arlen-thumbnail-sandbox");
        assert!(bin.is_file(), "build it first: {}", bin.display());

        // A 4x4 red PNG, written here so the check carries its own fixture.
        let png: Vec<u8> = {
            let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
            let chunk = |kind: &[u8], data: &[u8]| {
                let mut c = kind.to_vec();
                c.extend_from_slice(data);
                let mut o = (data.len() as u32).to_be_bytes().to_vec();
                o.extend_from_slice(&c);
                o.extend_from_slice(&crc32(&c).to_be_bytes());
                o
            };
            let mut ihdr = 4u32.to_be_bytes().to_vec();
            ihdr.extend_from_slice(&4u32.to_be_bytes());
            ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
            out.extend_from_slice(&chunk(b"IHDR", &ihdr));
            let raw: Vec<u8> = (0..4)
                .flat_map(|_| {
                    let mut row = vec![0u8];
                    row.extend((0..4).flat_map(|_| [255u8, 0, 0]));
                    row
                })
                .collect();
            out.extend_from_slice(&chunk(b"IDAT", &deflate_stored(&raw)));
            out.extend_from_slice(&chunk(b"IEND", b""));
            out
        };

        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("red.png");
        std::fs::write(&source, &png).unwrap();

        let cache = ThumbnailCache::new(dir.path().join("cache"));
        let generator = Sandboxed { bin };
        let url = thumbnail_data_url(&cache, &generator, &source, is_thumbnailable)
            .expect("the worker answered")
            .expect("a thumbnail for a real png");
        assert!(url.starts_with("data:image/png;base64,"), "{}", &url[..40.min(url.len())]);

        // And the second ask is a cache hit rather than a second subprocess: the
        // generator is swapped for one that would fail if it ran.
        struct Never;
        impl ThumbnailGenerator for Never {
            fn generate(&self, _: &Path) -> Result<Vec<u8>, ThumbnailError> {
                panic!("a cached thumbnail was regenerated");
            }
        }
        let again = thumbnail_data_url(&cache, &Never, &source, is_thumbnailable)
            .unwrap()
            .unwrap();
        assert_eq!(again, url);
    }

    /// A zlib stream with one stored (uncompressed) block, enough for a fixture.
    fn deflate_stored(data: &[u8]) -> Vec<u8> {
        let mut out = vec![0x78, 0x01];
        out.push(0x01);
        out.extend_from_slice(&(data.len() as u16).to_le_bytes());
        out.extend_from_slice(&(!(data.len() as u16)).to_le_bytes());
        out.extend_from_slice(data);
        let mut a: u32 = 1;
        let mut b: u32 = 0;
        for byte in data {
            a = (a + u32::from(*byte)) % 65521;
            b = (b + a) % 65521;
        }
        out.extend_from_slice(&((b << 16) | a).to_be_bytes());
        out
    }

    /// CRC-32 as PNG chunks carry it.
    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for byte in data {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
            }
        }
        !crc
    }

    #[tokio::test]
    async fn an_unthumbnailable_path_never_reaches_a_worker() {
        // No sandbox binary exists in a test run, so reaching one would fail
        // rather than answer `None`.
        assert_eq!(picker_thumbnail("/tmp/notes.txt".to_string()).await.unwrap(), None);
    }
}
