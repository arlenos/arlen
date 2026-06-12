//! The `files_thumbnail` command: image thumbnails for the grid/tile view
//! (file-manager-plan.md FM-R4).
//!
//! The file manager never decodes untrusted image bytes in its own process. This
//! host wires the two finished backend halves: the path+mtime
//! [`ThumbnailCache`](arlen_file_browser_core::thumbnail_cache::ThumbnailCache)
//! (in the core, decode-free) and the sandboxed `arlen-ai-sandbox` thumbnail
//! worker (decode in a Landlock + seccomp subprocess). The command reads the file
//! bytes (reading is safe; only decoding is dangerous, and that happens in the
//! sandbox), generates-and-caches on a miss, and returns the PNG as a data-URL
//! the tile's `<img>` loads. It must be called OFF the directory-listing path
//! (lazily, per visible tile), so a slow decode never stalls the listing; a
//! non-image or a failure returns `None` and the tile falls back to its icon.

use std::path::{Path, PathBuf};

use arlen_file_browser_core::thumbnail_cache::{
    ThumbnailCache, ThumbnailError, ThumbnailGenerator,
};

/// Image extensions worth thumbnailing. Non-image files return `None` without
/// ever spawning the sandbox worker (it would only fail), keeping the command
/// cheap for the common non-image case.
fn is_thumbnailable(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp")
    )
}

/// The path to the sandboxed thumbnail worker: the `ARLEN_THUMBNAIL_SANDBOX_BIN`
/// override (dev / test) else the installed libexec path. If the binary is
/// absent the generator simply fails and the tile falls back to its icon.
fn thumbnail_sandbox_bin() -> PathBuf {
    std::env::var_os("ARLEN_THUMBNAIL_SANDBOX_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/usr/lib/arlen/libexec/arlen-thumbnail-sandbox"))
}

/// The thumbnail cache directory: `$XDG_CACHE_HOME/arlen/thumbnails`.
fn thumbnail_cache_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("arlen").join("thumbnails"))
}

/// A generator that reads the source bytes and decodes them in the sandboxed
/// worker. Reading the file is safe; the dangerous decode runs behind the
/// worker's lockdown.
struct SandboxedThumbnailGenerator {
    sandbox_bin: PathBuf,
}

impl ThumbnailGenerator for SandboxedThumbnailGenerator {
    fn generate(&self, source: &Path) -> Result<Vec<u8>, ThumbnailError> {
        let bytes = read_capped(source)?;
        arlen_ai_sandbox::thumbnail(&self.sandbox_bin, &bytes)
            .map_err(|e| ThumbnailError::Generate(e.to_string()))
    }
}

/// Read up to the sandbox's byte cap; a larger file is refused (it would be
/// rejected by the worker anyway) without loading it whole.
fn read_capped(source: &Path) -> Result<Vec<u8>, ThumbnailError> {
    use std::io::Read;
    let file = std::fs::File::open(source).map_err(|e| ThumbnailError::Metadata(e.to_string()))?;
    let cap = arlen_ai_sandbox::MAX_BYTES;
    let mut buf = Vec::new();
    file.take(cap as u64 + 1)
        .read_to_end(&mut buf)
        .map_err(|e| ThumbnailError::Generate(e.to_string()))?;
    if buf.len() > cap {
        return Err(ThumbnailError::Generate("source too large".to_string()));
    }
    Ok(buf)
}

/// The cached PNG as the data-URL the tile's `<img>` loads.
fn encode(cached: &Path) -> Result<String, String> {
    let png = std::fs::read(cached).map_err(|e| e.to_string())?;
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
    Ok(format!("data:image/png;base64,{b64}"))
}

/// The pure command logic: a data-URL of `path`'s thumbnail, or `None` when it is
/// not a thumbnailable image or generation failed (the tile then shows its icon).
/// Generation/cache go through `cache` + `gen`, so this is testable with a mock.
fn thumbnail_data_url(
    cache: &ThumbnailCache,
    gen: &dyn ThumbnailGenerator,
    path: &Path,
) -> Result<Option<String>, String> {
    if !is_thumbnailable(path) {
        return Ok(None);
    }
    let cached = match cache.get_or_generate(path, gen) {
        Ok(p) => p,
        // A non-image or a decode failure: no thumbnail, fall back to the icon.
        Err(_) => return Ok(None),
    };
    encode(&cached).map(Some)
}

/// Bounds concurrent sandboxed decodes: every cache miss is a worker
/// subprocess plus an up-to-16MiB read, and the windowed grid can ask for a
/// whole viewport of tiles at once. Cache hits bypass the limit entirely.
pub struct ThumbnailLimiter(tokio::sync::Semaphore);

impl ThumbnailLimiter {
    /// Four concurrent decodes: enough to keep a viewport's worth of misses
    /// flowing without forking a process storm.
    pub fn new() -> Self {
        Self(tokio::sync::Semaphore::new(4))
    }
}

impl Default for ThumbnailLimiter {
    fn default() -> Self {
        Self::new()
    }
}

/// Return a data-URL thumbnail for `path`, generating and caching it on a miss.
///
/// Call this OFF the listing path (lazily per visible tile): a miss spawns the
/// sandboxed decoder subprocess, bounded by the limiter. A cache hit is served
/// outside the limit (a stat + read + encode never queues behind decodes).
/// Returns `None` for a non-image or on any failure, so the tile falls back to
/// its icon.
#[tauri::command]
pub async fn files_thumbnail(
    path: String,
    limiter: tauri::State<'_, ThumbnailLimiter>,
) -> Result<Option<String>, String> {
    let Some(dir) = thumbnail_cache_dir() else {
        return Ok(None);
    };
    if !is_thumbnailable(Path::new(&path)) {
        return Ok(None);
    }

    let hit = {
        let dir = dir.clone();
        let p = path.clone();
        tauri::async_runtime::spawn_blocking(move || {
            ThumbnailCache::new(dir).lookup(Path::new(&p)).ok().flatten()
        })
        .await
        .map_err(|e| e.to_string())?
    };
    if let Some(cached) = hit {
        return encode(&cached).map(Some);
    }

    let _permit = limiter.0.acquire().await.map_err(|e| e.to_string())?;
    let bin = thumbnail_sandbox_bin();
    // Off the async runtime: a miss spawns the worker and blocks on its output.
    tauri::async_runtime::spawn_blocking(move || {
        let cache = ThumbnailCache::new(dir);
        let gen = SandboxedThumbnailGenerator { sandbox_bin: bin };
        thumbnail_data_url(&cache, &gen, Path::new(&path))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct MockGen {
        calls: Cell<u32>,
        bytes: Vec<u8>,
    }
    impl ThumbnailGenerator for MockGen {
        fn generate(&self, _: &Path) -> Result<Vec<u8>, ThumbnailError> {
            self.calls.set(self.calls.get() + 1);
            Ok(self.bytes.clone())
        }
    }

    #[test]
    fn is_thumbnailable_matches_image_extensions_case_insensitively() {
        assert!(is_thumbnailable(Path::new("/x/a.png")));
        assert!(is_thumbnailable(Path::new("/x/a.JPG")));
        assert!(is_thumbnailable(Path::new("/x/a.webp")));
        assert!(!is_thumbnailable(Path::new("/x/a.txt")));
        assert!(!is_thumbnailable(Path::new("/x/noext")));
    }

    #[test]
    fn data_url_wraps_the_cached_png_for_an_image() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("a.png");
        std::fs::write(&src, b"pretend-image").unwrap();
        let cache = ThumbnailCache::new(tmp.path().join("cache"));
        let gen = MockGen {
            calls: Cell::new(0),
            bytes: b"FAKE-PNG-BYTES".to_vec(),
        };
        let url = thumbnail_data_url(&cache, &gen, &src).unwrap().unwrap();
        let prefix = "data:image/png;base64,";
        assert!(url.starts_with(prefix), "got {url}");
        use base64::Engine;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&url[prefix.len()..])
            .unwrap();
        assert_eq!(decoded, b"FAKE-PNG-BYTES");
    }

    #[test]
    fn a_non_image_returns_none_without_generating() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("notes.txt");
        std::fs::write(&src, b"text").unwrap();
        let cache = ThumbnailCache::new(tmp.path().join("cache"));
        let gen = MockGen {
            calls: Cell::new(0),
            bytes: vec![],
        };
        assert_eq!(thumbnail_data_url(&cache, &gen, &src).unwrap(), None);
        assert_eq!(gen.calls.get(), 0, "a non-image never spawns the generator");
    }

    #[test]
    fn encode_round_trips_the_cached_file() {
        let tmp = tempfile::tempdir().unwrap();
        let cached = tmp.path().join("x.png");
        std::fs::write(&cached, b"PNGBYTES").unwrap();
        let url = encode(&cached).unwrap();
        let prefix = "data:image/png;base64,";
        use base64::Engine;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&url[prefix.len()..])
            .unwrap();
        assert_eq!(decoded, b"PNGBYTES");
    }

    #[test]
    fn a_generation_failure_falls_back_to_none() {
        struct FailGen;
        impl ThumbnailGenerator for FailGen {
            fn generate(&self, _: &Path) -> Result<Vec<u8>, ThumbnailError> {
                Err(ThumbnailError::Generate("decode failed".to_string()))
            }
        }
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("broken.png");
        std::fs::write(&src, b"not really a png").unwrap();
        let cache = ThumbnailCache::new(tmp.path().join("cache"));
        assert_eq!(thumbnail_data_url(&cache, &FailGen, &src).unwrap(), None);
    }
}
