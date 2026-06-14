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
    thumb_kind(path) == Some(ThumbKind::Image)
}

/// Whether the path is an audio file whose embedded cover art can be thumbnailed.
fn is_audio(path: &Path) -> bool {
    thumb_kind(path) == Some(ThumbKind::Music)
}

/// What kind of thumbnail a path is eligible for, decided cheaply by extension
/// so a non-thumbnailable file never spawns a worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThumbKind {
    /// A raster image, decoded directly by the image worker.
    Image,
    /// An audio file whose embedded cover art is extracted and decoded by the
    /// music worker.
    Music,
}

/// The thumbnail kind for a path, or `None` when it is neither a supported image
/// nor a supported audio file (the tile then shows its icon, no worker spawned).
fn thumb_kind(path: &Path) -> Option<ThumbKind> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp") => Some(ThumbKind::Image),
        // Audio containers lofty reads embedded cover art from.
        Some("mp3" | "flac" | "m4a" | "m4b" | "ogg" | "oga" | "opus" | "aiff" | "aif" | "wav") => {
            Some(ThumbKind::Music)
        }
        _ => None,
    }
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

/// The path to the sandboxed music cover-art worker: the
/// `ARLEN_MUSIC_THUMBNAIL_SANDBOX_BIN` override (dev / test) else the installed
/// libexec path. If the binary is absent the generator fails and the tile falls
/// back to its music icon.
fn music_thumbnail_sandbox_bin() -> PathBuf {
    std::env::var_os("ARLEN_MUSIC_THUMBNAIL_SANDBOX_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/usr/lib/arlen/libexec/arlen-music-thumbnail-sandbox"))
}

/// A generator that reads an audio file's bytes and extracts+decodes its
/// embedded cover art in the sandboxed music worker. Reading the file is safe;
/// the untrusted metadata parse and image decode both run behind the worker's
/// lockdown. An audio file with no usable embedded art is a generation "failure"
/// (so the tile shows its music icon), distinct from a real error but mapped the
/// same way: no thumbnail is cached.
struct MusicThumbnailGenerator {
    sandbox_bin: PathBuf,
}

impl ThumbnailGenerator for MusicThumbnailGenerator {
    fn generate(&self, source: &Path) -> Result<Vec<u8>, ThumbnailError> {
        let bytes = read_capped(source)?;
        match arlen_ai_sandbox::album_art_thumbnail(&self.sandbox_bin, &bytes) {
            Ok(Some(png)) => Ok(png),
            // No embedded art: fall back to the icon without caching an empty
            // thumbnail (a re-view re-probes, matching the image decode-failure
            // path; negative caching is not worth a stale-on-retag risk).
            Ok(None) => Err(ThumbnailError::Generate("no embedded cover art".to_string())),
            Err(e) => Err(ThumbnailError::Generate(e.to_string())),
        }
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

/// The pure command logic: a data-URL of `path`'s thumbnail, or `None` when the
/// path is not `supported`, has no thumbnail, or generation failed (the tile
/// then shows its icon). `supported` gates the spawn so an unsupported path
/// never reaches the worker; generation/cache go through `cache` + `gen`, so
/// this is testable with a mock.
fn thumbnail_data_url(
    cache: &ThumbnailCache,
    gen: &dyn ThumbnailGenerator,
    path: &Path,
    supported: fn(&Path) -> bool,
) -> Result<Option<String>, String> {
    if !supported(path) {
        return Ok(None);
    }
    let cached = match cache.get_or_generate(path, gen) {
        Ok(p) => p,
        // Unsupported, no embedded art, or a decode failure: no thumbnail, fall
        // back to the icon.
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
    // Decide the kind once: a non-thumbnailable path returns early, never
    // spawning a worker.
    let Some(kind) = thumb_kind(Path::new(&path)) else {
        return Ok(None);
    };

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
    // Off the async runtime: a miss spawns the matching worker and blocks on its
    // output. The image and music workers share the cache (path+mtime keyed) and
    // the limiter; only the generator and its support gate differ by kind.
    tauri::async_runtime::spawn_blocking(move || {
        let cache = ThumbnailCache::new(dir);
        match kind {
            ThumbKind::Image => {
                let gen = SandboxedThumbnailGenerator {
                    sandbox_bin: thumbnail_sandbox_bin(),
                };
                thumbnail_data_url(&cache, &gen, Path::new(&path), is_thumbnailable)
            }
            ThumbKind::Music => {
                let gen = MusicThumbnailGenerator {
                    sandbox_bin: music_thumbnail_sandbox_bin(),
                };
                thumbnail_data_url(&cache, &gen, Path::new(&path), is_audio)
            }
        }
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
    fn thumb_kind_classifies_image_audio_and_other() {
        assert_eq!(thumb_kind(Path::new("/x/a.png")), Some(ThumbKind::Image));
        assert_eq!(thumb_kind(Path::new("/x/song.MP3")), Some(ThumbKind::Music));
        assert_eq!(thumb_kind(Path::new("/x/song.flac")), Some(ThumbKind::Music));
        assert_eq!(thumb_kind(Path::new("/x/song.opus")), Some(ThumbKind::Music));
        assert!(is_audio(Path::new("/x/track.m4a")));
        assert!(!is_audio(Path::new("/x/a.png")));
        assert_eq!(thumb_kind(Path::new("/x/notes.txt")), None);
        assert_eq!(thumb_kind(Path::new("/x/noext")), None);
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
        let url = thumbnail_data_url(&cache, &gen, &src, is_thumbnailable)
            .unwrap()
            .unwrap();
        let prefix = "data:image/png;base64,";
        assert!(url.starts_with(prefix), "got {url}");
        use base64::Engine;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&url[prefix.len()..])
            .unwrap();
        assert_eq!(decoded, b"FAKE-PNG-BYTES");
    }

    #[test]
    fn data_url_wraps_extracted_cover_art_for_audio() {
        // The music path shares the same cache+encode; only the support gate and
        // the generator (which would extract+decode the embedded art) differ.
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("track.flac");
        std::fs::write(&src, b"pretend-audio").unwrap();
        let cache = ThumbnailCache::new(tmp.path().join("cache"));
        let gen = MockGen {
            calls: Cell::new(0),
            bytes: b"COVER-PNG".to_vec(),
        };
        let url = thumbnail_data_url(&cache, &gen, &src, is_audio)
            .unwrap()
            .unwrap();
        use base64::Engine;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&url["data:image/png;base64,".len()..])
            .unwrap();
        assert_eq!(decoded, b"COVER-PNG");
    }

    #[test]
    fn an_unsupported_path_returns_none_without_generating() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("notes.txt");
        std::fs::write(&src, b"text").unwrap();
        let cache = ThumbnailCache::new(tmp.path().join("cache"));
        let gen = MockGen {
            calls: Cell::new(0),
            bytes: vec![],
        };
        // Neither the image nor the audio gate admits a .txt, so no worker spawns.
        assert_eq!(
            thumbnail_data_url(&cache, &gen, &src, is_thumbnailable).unwrap(),
            None
        );
        assert_eq!(
            thumbnail_data_url(&cache, &gen, &src, is_audio).unwrap(),
            None
        );
        assert_eq!(gen.calls.get(), 0, "an unsupported path never spawns the generator");
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
        assert_eq!(
            thumbnail_data_url(&cache, &FailGen, &src, is_thumbnailable).unwrap(),
            None
        );
    }
}
