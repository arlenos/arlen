//! A path + mtime keyed, on-disk thumbnail cache (file-manager-plan.md FM-R4).
//!
//! The file manager must never decode untrusted image bytes in its own process:
//! a decoder exploit would run with the FM's privileges. Generation is the
//! sandboxed `arlen-ai-sandbox` thumbnail worker, kept OFF the directory-listing
//! path so a slow decode never stalls the listing. This cache sits in front of
//! that worker: it keys a thumbnail by the source's canonical path and mtime (so
//! a changed file misses and is regenerated), stores the PNG under a host-supplied
//! cache directory, and only invokes the generator on a miss.
//!
//! Generation is the [`ThumbnailGenerator`] seam, so this core carries no
//! image-decode or sandbox dependency: the Tauri host wires the real sandboxed
//! worker (`arlen_ai_sandbox::thumbnail`) and resolves the cache directory (e.g.
//! `$XDG_CACHE_HOME/arlen/thumbnails`); tests inject a counting mock.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Produces thumbnail bytes (a PNG) for a source file.
///
/// The real implementation runs the sandboxed `arlen-ai-sandbox` thumbnail
/// worker. This seam keeps the cache core free of the decode/sandbox dependency
/// and testable with a mock.
pub trait ThumbnailGenerator {
    /// Generate a thumbnail PNG for `source`, or an error if none can be made.
    fn generate(&self, source: &Path) -> Result<Vec<u8>, ThumbnailError>;
}

/// Why a thumbnail could not be obtained.
#[derive(Debug, thiserror::Error)]
pub enum ThumbnailError {
    /// The source file's metadata (needed for the mtime cache key) was unreadable.
    #[error("source metadata: {0}")]
    Metadata(String),
    /// The cache directory could not be created, read, or written.
    #[error("cache io: {0}")]
    Cache(String),
    /// The generator failed to produce a thumbnail.
    #[error("generate: {0}")]
    Generate(String),
}

/// An on-disk thumbnail cache rooted at a host-supplied directory.
pub struct ThumbnailCache {
    dir: PathBuf,
}

impl ThumbnailCache {
    /// A cache storing thumbnails under `dir` (e.g.
    /// `$XDG_CACHE_HOME/arlen/thumbnails`, resolved by the host). The directory
    /// is created on the first write.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The cached thumbnail path for `source` if present and current, else `None`
    /// (without ever generating). Safe to call on the listing path: it only stats
    /// the source and the cache file.
    pub fn lookup(&self, source: &Path) -> Result<Option<PathBuf>, ThumbnailError> {
        let path = self.cached_path(source)?;
        Ok(path.is_file().then_some(path))
    }

    /// The cached thumbnail path for `source`, generating and caching it on a miss
    /// via `gen`.
    ///
    /// On a miss the generated PNG is written atomically (a unique temp file then
    /// a rename over the final name), so a concurrent reader never observes a
    /// partial thumbnail. This should run OFF the listing path (e.g. lazily per
    /// visible tile), since the generator decodes in a subprocess.
    pub fn get_or_generate(
        &self,
        source: &Path,
        gen: &dyn ThumbnailGenerator,
    ) -> Result<PathBuf, ThumbnailError> {
        let path = self.cached_path(source)?;
        if path.is_file() {
            return Ok(path);
        }
        let bytes = gen.generate(source)?;
        std::fs::create_dir_all(&self.dir).map_err(|e| ThumbnailError::Cache(e.to_string()))?;
        // Atomic publish: a unique temp (pid-tagged) then a rename over the final
        // name. A failed rename removes the temp so the cache dir stays clean.
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| ThumbnailError::Cache("bad cache file name".to_string()))?;
        let tmp = self.dir.join(format!(".{}.{}.tmp", file_name, std::process::id()));
        std::fs::write(&tmp, &bytes).map_err(|e| ThumbnailError::Cache(e.to_string()))?;
        std::fs::rename(&tmp, &path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            ThumbnailError::Cache(e.to_string())
        })?;
        Ok(path)
    }

    /// The cache-file path for `source`: `<dir>/<key>.png`, where the key is a hex
    /// SHA-256 of the source's canonical path and mtime. Including the mtime
    /// invalidates the entry whenever the file changes.
    fn cached_path(&self, source: &Path) -> Result<PathBuf, ThumbnailError> {
        Ok(self.dir.join(format!("{}.png", self.key_for(source)?)))
    }

    fn key_for(&self, source: &Path) -> Result<String, ThumbnailError> {
        let meta =
            std::fs::metadata(source).map_err(|e| ThumbnailError::Metadata(e.to_string()))?;
        let mtime_nanos = meta
            .modified()
            .map_err(|e| ThumbnailError::Metadata(e.to_string()))?
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        // Canonicalise so two paths to the same file share one cache entry; fall
        // back to the given path if canonicalisation fails (permissions, etc.).
        let canonical = std::fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());

        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(canonical.to_string_lossy().as_bytes());
        hasher.update(b"\0");
        hasher.update(mtime_nanos.to_le_bytes());
        Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::ffi::CString;

    /// A generator that records how many times it ran.
    struct CountingGen {
        calls: Cell<u32>,
        bytes: Vec<u8>,
    }
    impl ThumbnailGenerator for CountingGen {
        fn generate(&self, _source: &Path) -> Result<Vec<u8>, ThumbnailError> {
            self.calls.set(self.calls.get() + 1);
            Ok(self.bytes.clone())
        }
    }
    fn counting(bytes: &[u8]) -> CountingGen {
        CountingGen {
            calls: Cell::new(0),
            bytes: bytes.to_vec(),
        }
    }

    fn write(dir: &Path, name: &str, contents: &[u8]) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, contents).unwrap();
        p
    }

    /// Set an explicit mtime so a key change is deterministic (no sleep race).
    fn set_mtime(path: &Path, secs: i64) {
        let t = libc::timeval {
            tv_sec: secs as libc::time_t,
            tv_usec: 0,
        };
        let times = [t, t];
        let c = CString::new(path.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::utimes(c.as_ptr(), times.as_ptr()) }, 0);
    }

    #[test]
    fn generates_on_miss_then_serves_from_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = ThumbnailCache::new(tmp.path().join("cache"));
        let src = write(tmp.path(), "a.png", b"original");
        let gen = counting(b"THUMB-BYTES");

        let first = cache.get_or_generate(&src, &gen).unwrap();
        assert_eq!(std::fs::read(&first).unwrap(), b"THUMB-BYTES");
        assert_eq!(gen.calls.get(), 1, "a miss generates");

        let second = cache.get_or_generate(&src, &gen).unwrap();
        assert_eq!(first, second);
        assert_eq!(gen.calls.get(), 1, "a hit does not regenerate");
    }

    #[test]
    fn lookup_never_generates() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = ThumbnailCache::new(tmp.path().join("cache"));
        let src = write(tmp.path(), "a.png", b"x");
        assert!(cache.lookup(&src).unwrap().is_none(), "miss is None, no gen");

        let gen = counting(b"T");
        cache.get_or_generate(&src, &gen).unwrap();
        assert!(cache.lookup(&src).unwrap().is_some(), "now cached");
        assert_eq!(gen.calls.get(), 1);
    }

    #[test]
    fn a_changed_file_misses_and_regenerates() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = ThumbnailCache::new(tmp.path().join("cache"));
        let src = write(tmp.path(), "a.png", b"v1");
        set_mtime(&src, 1_000_000_000);
        let gen = counting(b"T");

        cache.get_or_generate(&src, &gen).unwrap();
        assert_eq!(gen.calls.get(), 1);

        // Same content, an explicitly newer mtime -> different key -> miss.
        set_mtime(&src, 2_000_000_000);
        cache.get_or_generate(&src, &gen).unwrap();
        assert_eq!(gen.calls.get(), 2, "a changed mtime regenerates");
    }

    #[test]
    fn a_generator_error_yields_no_cache_file() {
        struct FailGen;
        impl ThumbnailGenerator for FailGen {
            fn generate(&self, _: &Path) -> Result<Vec<u8>, ThumbnailError> {
                Err(ThumbnailError::Generate("boom".to_string()))
            }
        }
        let tmp = tempfile::tempdir().unwrap();
        let cache = ThumbnailCache::new(tmp.path().join("cache"));
        let src = write(tmp.path(), "a.png", b"x");
        assert!(cache.get_or_generate(&src, &FailGen).is_err());
        assert!(cache.lookup(&src).unwrap().is_none(), "no partial cache file");
    }
}
