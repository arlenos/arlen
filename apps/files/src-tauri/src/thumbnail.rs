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

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

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
    /// A video file whose first frame is extracted by `ffmpeg` inside a bwrap
    /// jail (ffmpeg is a separate binary, so it cannot run in the in-process
    /// Landlock+seccomp workers the image/music kinds use).
    Video,
}

/// Whether `path` is a video file the [`VideoThumbnailGenerator`] handles (the
/// gate that spawns the jail, mirroring [`is_thumbnailable`]/[`is_audio`]).
fn is_video(path: &Path) -> bool {
    thumb_kind(path) == Some(ThumbKind::Video)
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
        // Video containers ffmpeg extracts a first frame from.
        Some("mp4" | "m4v" | "mkv" | "webm" | "mov" | "avi" | "wmv" | "flv" | "mpg" | "mpeg") => {
            Some(ThumbKind::Video)
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

/// Longest edge of a video thumbnail, in px.
const VIDEO_THUMB_MAX_PX: u32 = 320;
/// Hard cap on the PNG the video jail may emit. A thumbnail is tiny; this bounds
/// a decompression bomb or a hostile ffmpeg writing without end.
const VIDEO_OUTPUT_CAP: usize = 4 * 1024 * 1024;
/// Wall-clock budget for one video decode; a hung or heavy decode is killed.
const VIDEO_DECODE_TIMEOUT: Duration = Duration::from_secs(10);
/// The in-jail path the video is bound at (fixed, extension-free).
const VIDEO_INPUT_MOUNT: &str = "/in";
/// Largest single ffmpeg allocation (256 MiB): a ~8K RGBA frame fits, an absurd
/// resolution does not - the decode-memory bound of the bomb posture.
const VIDEO_MAX_ALLOC: usize = 256 * 1024 * 1024;
/// The 8-byte PNG signature; the jail's output must start with it or it is not a
/// frame (an ffmpeg error / empty output), and the tile shows its icon.
const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
/// Size cap on the jail's scratch tmpfs (64 MiB), so `/tmp` cannot grow into a
/// RAM-exhaustion vector even before the scope cgroup's aggregate ceiling.
const VIDEO_TMPFS_SIZE: usize = 64 * 1024 * 1024;
/// The transient-scope resource ceiling for one decode: aggregate memory, no
/// swap, a bounded task count (a fork-bomb ceiling the per-process jail cannot
/// give), two cores, and a hard 15s scope runtime backstopping the wall timeout.
const SCOPE_PROPS: [&str; 5] = [
    "MemoryMax=512M",
    "MemorySwapMax=0",
    "TasksMax=64",
    "CPUQuota=200%",
    "RuntimeMaxSec=15",
];

/// The `bwrap` binary (`ARLEN_BWRAP_BIN` override for tests/dev, else the standard
/// absolute path; pinned so `PATH` cannot redirect the sandbox launcher).
fn bwrap_bin() -> PathBuf {
    std::env::var_os("ARLEN_BWRAP_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/usr/bin/bwrap"))
}

/// Whether to wrap the jail in a `systemd-run --user --scope` cgroup. True only
/// when a user manager is reachable (a `systemd-run` binary and an
/// `XDG_RUNTIME_DIR`) and not explicitly disabled (`ARLEN_THUMBNAIL_NO_SCOPE`,
/// which the integration test sets so it runs without a user session). When
/// false the bare jail still holds (`-max_alloc`, the tmpfs `--size`, the wall
/// timeout); the scope only adds the aggregate ceiling.
fn use_scope() -> bool {
    std::env::var_os("ARLEN_THUMBNAIL_NO_SCOPE").is_none()
        && std::env::var_os("XDG_RUNTIME_DIR").is_some()
        && Path::new("/usr/bin/systemd-run").exists()
}

/// Build the `Command` that runs the jail: in production a transient
/// `systemd-run --user --scope` whose cgroup caps aggregate memory/tasks/CPU
/// (the review's HIGH: a hostile decode auto-triggered by merely viewing a
/// folder cannot OOM the host), else the bare pinned `bwrap` (dev/no-session,
/// the in-process floor still applies). `bwrap_argv` is everything after
/// `bwrap`.
fn jailed_command(bwrap_argv: &[String]) -> Command {
    let bwrap = bwrap_bin();
    if use_scope() {
        let mut c = Command::new("/usr/bin/systemd-run");
        c.args(["--user", "--scope", "-q", "--collect"]);
        for prop in SCOPE_PROPS {
            c.args(["-p", prop]);
        }
        c.arg("--");
        c.arg(&bwrap);
        c.args(bwrap_argv);
        c
    } else {
        let mut c = Command::new(&bwrap);
        c.args(bwrap_argv);
        c
    }
}

/// The host ffmpeg binary (`ARLEN_FFMPEG_BIN` override for tests/dev, else the
/// standard path). It lives inside the read-only `/usr` the jail binds, so the
/// host path is also the in-jail path.
fn ffmpeg_bin() -> PathBuf {
    std::env::var_os("ARLEN_FFMPEG_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/usr/bin/ffmpeg"))
}

/// Build the `bwrap` argument vector (everything after the literal `bwrap`) that
/// runs `ffmpeg` jailed over `input`. Pure, so the confinement is unit-tested.
///
/// The jail: every namespace unshared INCLUDING the network; a read-only `/usr`
/// (ffmpeg + its shared libraries) with the merged-usr symlinks so the ELF loader
/// resolves; the one untrusted video bound READ-ONLY at [`VIDEO_INPUT_MOUNT`]; a
/// private proc/dev and a scratch tmpfs; NO writable bind anywhere; a fixed PATH.
/// ffmpeg then reads the input and writes a single PNG frame to stdout. A
/// malicious-video ffmpeg RCE is contained: no network, no filesystem write, no
/// home - only the read-only `/usr` and the one input are reachable. `None` if a
/// path is not UTF-8 (it would be mangled into the argv).
fn video_bwrap_argv(ffmpeg: &Path, input: &Path) -> Option<Vec<String>> {
    let ffmpeg = ffmpeg.to_str()?.to_string();
    let input = input.to_str()?.to_string();
    let scale = format!("scale='min({VIDEO_THUMB_MAX_PX},iw)':-2");
    let mut a: Vec<String> = Vec::new();
    for f in [
        "--unshare-user",
        "--unshare-pid",
        "--unshare-ipc",
        "--unshare-uts",
        "--unshare-cgroup-try",
        "--unshare-net",
        "--new-session",
        "--die-with-parent",
        "--clearenv",
    ] {
        a.push(f.into());
    }
    // Read-only system + the merged-usr symlinks the dynamic loader needs.
    a.extend(["--ro-bind".into(), "/usr".into(), "/usr".into()]);
    for (target, link) in [
        ("usr/lib", "/lib"),
        ("usr/lib64", "/lib64"),
        ("usr/bin", "/bin"),
        ("usr/sbin", "/sbin"),
    ] {
        a.extend(["--symlink".into(), target.into(), link.into()]);
    }
    // The one untrusted input, read-only.
    a.extend(["--ro-bind".into(), input, VIDEO_INPUT_MOUNT.into()]);
    // Private kernel surfaces + a size-bounded scratch tmpfs; no writable host
    // bind. The `--size` caps `/tmp` so a runaway/hostile write cannot fill RAM
    // through the tmpfs (the in-process floor; the scope cgroup below is the
    // aggregate ceiling in production).
    a.extend(["--proc".into(), "/proc".into()]);
    a.extend(["--dev".into(), "/dev".into()]);
    a.extend(["--size".into(), VIDEO_TMPFS_SIZE.to_string(), "--tmpfs".into(), "/tmp".into()]);
    a.extend(["--setenv".into(), "PATH".into(), "/usr/bin:/bin".into()]);
    // The decoder: one frame, scaled, PNG to stdout.
    a.push("--".into());
    a.push(ffmpeg);
    let max_alloc = VIDEO_MAX_ALLOC.to_string();
    // `-max_alloc` caps a single ffmpeg allocation: a video declaring an absurd
    // resolution (a decode-memory bomb) is refused before it can exhaust host RAM
    // (the jail caps escape, the output cap caps stdout, the timeout caps time;
    // this caps decode memory). It is a global option, so it precedes `-i`.
    for arg in [
        "-hide_banner",
        "-loglevel",
        "error",
        "-max_alloc",
        &max_alloc,
        "-nostdin",
        "-i",
        VIDEO_INPUT_MOUNT,
        "-frames:v",
        "1",
        "-vf",
    ] {
        a.push(arg.into());
    }
    a.push(scale);
    for arg in ["-f", "image2", "-c:v", "png", "-"] {
        a.push(arg.into());
    }
    Some(a)
}

/// Run the jailed ffmpeg over `source` and return the first-frame PNG, or a
/// [`ThumbnailError`] (the tile then shows its icon). stdout is read on a thread
/// (so a full pipe never deadlocks the wait) capped at [`VIDEO_OUTPUT_CAP`]; the
/// child is killed after [`VIDEO_DECODE_TIMEOUT`]; the output must be a real PNG.
fn generate_video_thumbnail(source: &Path) -> Result<Vec<u8>, ThumbnailError> {
    let argv = video_bwrap_argv(&ffmpeg_bin(), source)
        .ok_or_else(|| ThumbnailError::Generate("non-utf8 path".to_string()))?;
    let mut child = jailed_command(&argv)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| ThumbnailError::Generate(format!("spawn jail: {e}")))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| ThumbnailError::Generate("no stdout".to_string()))?;
    let reader = thread::spawn(move || {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 16384];
        let mut capped = false;
        loop {
            match stdout.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    let room = VIDEO_OUTPUT_CAP.saturating_sub(buf.len());
                    if n >= room {
                        buf.extend_from_slice(&chunk[..room]);
                        capped = true;
                        break; // dropping `stdout` after this closes the pipe
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                Err(_) => break,
            }
        }
        (buf, capped)
    });
    let deadline = Instant::now() + VIDEO_DECODE_TIMEOUT;
    let timed_out = loop {
        match child.try_wait() {
            Ok(Some(_)) => break false,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break true;
                }
                thread::sleep(Duration::from_millis(40));
            }
            Err(e) => return Err(ThumbnailError::Generate(format!("wait: {e}"))),
        }
    };
    // The reader thread always terminates: it stops on EOF (the killed/exited
    // child closes the pipe) or on the cap (it drops `stdout`), so the join is
    // bounded by the wall timeout above, not by a cooperative peer.
    let (bytes, capped) = reader
        .join()
        .map_err(|_| ThumbnailError::Generate("reader panicked".to_string()))?;
    if timed_out {
        return Err(ThumbnailError::Generate("decode timed out".to_string()));
    }
    if capped {
        // A truncated stream is not a frame; never cache a partial PNG.
        return Err(ThumbnailError::Generate("output exceeded the cap".to_string()));
    }
    if bytes.len() >= 8 && bytes[..8] == PNG_MAGIC {
        Ok(bytes)
    } else {
        Err(ThumbnailError::Generate("ffmpeg produced no frame".to_string()))
    }
}

/// A generator that extracts a video's first frame in the bwrap-ffmpeg jail.
/// Unlike the image/music generators it passes the file path (ffmpeg must seek to
/// decode a first frame, which a stdin pipe cannot do), bound read-only into the
/// jail; no file bytes are read into this process.
struct VideoThumbnailGenerator;

impl ThumbnailGenerator for VideoThumbnailGenerator {
    fn generate(&self, source: &Path) -> Result<Vec<u8>, ThumbnailError> {
        generate_video_thumbnail(source)
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
            ThumbKind::Video => {
                thumbnail_data_url(&cache, &VideoThumbnailGenerator, Path::new(&path), is_video)
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
        assert_eq!(thumb_kind(Path::new("/x/clip.MP4")), Some(ThumbKind::Video));
        assert_eq!(thumb_kind(Path::new("/x/clip.mkv")), Some(ThumbKind::Video));
        assert_eq!(thumb_kind(Path::new("/x/clip.webm")), Some(ThumbKind::Video));
        assert!(is_video(Path::new("/x/clip.mov")));
        assert!(!is_video(Path::new("/x/a.png")));
        assert_eq!(thumb_kind(Path::new("/x/notes.txt")), None);
        assert_eq!(thumb_kind(Path::new("/x/noext")), None);
    }

    #[test]
    fn video_bwrap_argv_jails_no_net_no_write_and_ro_binds_the_input() {
        let argv = video_bwrap_argv(Path::new("/usr/bin/ffmpeg"), Path::new("/home/u/clip.mp4"))
            .expect("utf8 paths");
        // No network, and the file system is read-only: no `--bind` (writable)
        // anywhere - only `--ro-bind`, `--symlink`, `--tmpfs`, `--proc`, `--dev`.
        assert!(argv.iter().any(|a| a == "--unshare-net"));
        assert!(!argv.iter().any(|a| a == "--bind"), "no writable host bind: {argv:?}");
        // The system is bound read-only and the one input is bound read-only at /in.
        let ro: Vec<_> = argv.windows(3).filter(|w| w[0] == "--ro-bind").map(|w| (w[1].as_str(), w[2].as_str())).collect();
        assert!(ro.contains(&("/usr", "/usr")));
        assert!(ro.contains(&("/home/u/clip.mp4", VIDEO_INPUT_MOUNT)));
        // ffmpeg is invoked for exactly one frame, read from /in, PNG to stdout.
        assert!(argv.iter().any(|a| a == "/usr/bin/ffmpeg"));
        let frames = argv.windows(2).any(|w| w[0] == "-frames:v" && w[1] == "1");
        assert!(frames, "one frame: {argv:?}");
        assert_eq!(argv.last().map(String::as_str), Some("-"));
        // A non-UTF-8 path is refused, not mangled into the argv.
        use std::os::unix::ffi::OsStrExt;
        let bad = Path::new(std::ffi::OsStr::from_bytes(b"/x/\xff.mp4"));
        assert!(video_bwrap_argv(Path::new("/usr/bin/ffmpeg"), bad).is_none());
    }

    /// On-host (needs `ffmpeg` + `bwrap`): a generated clip yields a real PNG from
    /// the jail. Skips cleanly where either tool is absent (CI without ffmpeg).
    #[test]
    fn video_thumbnail_extracts_a_png_frame_in_the_jail() {
        fn have(bin: &str) -> bool {
            std::process::Command::new(bin)
                .arg("-version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        }
        if !have("ffmpeg") || !have("bwrap") {
            eprintln!("skipping: ffmpeg/bwrap not available");
            return;
        }
        // Drive the bare jail (no `systemd-run --user` scope), so the test does
        // not depend on a running user manager; the boundary is identical, the
        // scope only adds the aggregate cgroup ceiling in production.
        std::env::set_var("ARLEN_THUMBNAIL_NO_SCOPE", "1");
        let tmp = tempfile::tempdir().unwrap();
        let clip = tmp.path().join("clip.mp4");
        // A 1-second synthetic clip, faststart so the moov atom is up front.
        let made = std::process::Command::new("ffmpeg")
            .args([
                "-y", "-f", "lavfi", "-i", "testsrc=size=320x240:rate=15:duration=1",
                "-frames:v", "15", "-pix_fmt", "yuv420p", "-movflags", "+faststart",
            ])
            .arg(&clip)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        assert!(made && clip.exists(), "could not synthesise a test clip");
        let png = generate_video_thumbnail(&clip).expect("a frame from the jail");
        assert!(png.len() >= 8 && png[..8] == PNG_MAGIC, "real PNG output");
        assert!(png.len() < VIDEO_OUTPUT_CAP, "within the output cap");
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
