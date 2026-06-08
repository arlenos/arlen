//! Fixed-output source fetch for forage.
//!
//! Fetch is the one phase with network access (forage-recipes.md sections 9,
//! 10a): the source is downloaded only from the hosts declared in the recipe's
//! `[source]`, SSRF-guarded and DNS-pinned via [`arlen_net_guard`], then stored
//! and rooted into the content-addressed store **only if it matches the
//! recipe's pinned `sha256`/commit**. A mismatch stores nothing. Because the
//! output is verified against a pre-pinned hash, allowing network here is safe
//! (a fixed-output derivation); after this phase the build runs with no network.
//!
//! All declared source kinds are handled: `tarball` (HTTP GET + sha256), `git`
//! (clone pinned to a commit, deterministic `git archive`), `github-release`
//! (tag-resolve via the API then download + lock the asset), `crate` (the
//! direct crates.io download URL + sha256) and `local` (a deterministic archive
//! of an on-disk path).
//!
//! ## Git SSRF model
//!
//! Unlike reqwest, `git` resolves DNS itself, so we cannot pin its resolver to
//! a verified address the way [`HttpDownloader`] pins reqwest with `.resolve()`.
//! The guard is therefore best-effort: before running `git` we parse the url's
//! host and call [`resolve_and_pin`], rejecting the fetch ([`FetchError::Blocked`])
//! if the host resolves into a blocked (loopback/RFC1918/link-local/...) range.
//! `git` then re-resolves the host when it connects, leaving a DNS-rebinding
//! window: a hostile resolver could return a public address to our check and a
//! private one to git's connect. This is the same class of limitation as
//! modulesd's redirect re-validation. It is acceptable here because the pinned
//! **commit hash is the real content-integrity guarantee**: `ProcessGitFetcher`
//! verifies `git rev-parse HEAD` equals the declared commit and archives that
//! exact tree, so a rebind can change *which server* is contacted but cannot
//! inject content that differs from the pinned commit. The pre-check still
//! closes the common SSRF case (a recipe naming an internal host directly).

use std::path::Path;
use std::process::Command;

use arlen_forage_recipe::{Source, SourceType};
use arlen_forage_store::{ContentHash, Store, StoreError};
use arlen_net_guard::{resolve_and_pin, GuardError};
use async_trait::async_trait;
use serde::Deserialize;

/// Default cap on a single downloaded source artifact (1 GiB). Source tarballs
/// are larger than module fetches; a recipe-specific override can come later.
pub const DEFAULT_MAX_BYTES: u64 = 1024 * 1024 * 1024;

/// A failure fetching or storing a source.
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    /// A field required for this source kind is missing.
    #[error("source is missing required field: {0}")]
    MissingField(&'static str),
    /// The declared sha256 is malformed.
    #[error("invalid sha256: {0}")]
    InvalidHash(String),
    /// A network/transport failure.
    #[error("network: {0}")]
    Network(String),
    /// The destination resolved into a blocked range (SSRF guard).
    #[error("blocked destination: {0}")]
    Blocked(String),
    /// A git operation failed (clone/fetch/checkout/archive) or the checkout
    /// did not match the declared commit.
    #[error("git: {0}")]
    Git(String),
    /// The download exceeded the size cap.
    #[error("download exceeded {limit} bytes")]
    TooLarge {
        /// The cap that was exceeded.
        limit: u64,
    },
    /// Storing or verifying the fetched bytes failed (includes a hash mismatch).
    #[error("store: {0}")]
    Store(#[from] StoreError),
    /// A `local` source could not be read or archived.
    #[error("local source: {0}")]
    Local(String),
    /// A `crate` source had a name or version outside the crates.io character
    /// set (rejected before it is interpolated into the download URL).
    #[error("invalid crate source: {0}")]
    InvalidCrate(String),
}

/// Fetches the bytes at a URL, capped at `max_bytes`. Behind a trait so the
/// fetch logic can be tested without real network.
#[async_trait]
pub trait Downloader: Send + Sync {
    /// GET `url`, returning at most `max_bytes` of body or [`FetchError::TooLarge`].
    async fn get(&self, url: &str, max_bytes: u64) -> Result<Vec<u8>, FetchError>;
}

/// Clones a git repository pinned to a commit and returns a deterministic
/// archive of that commit's tree. Behind a trait so the fetch logic can be
/// tested without real git or network.
///
/// Implementations must verify the checkout actually equals `commit` and must
/// produce a reproducible archive (e.g. `git archive`, which excludes `.git`
/// and emits a deterministic tar for a fixed commit).
pub trait GitFetcher: Send + Sync {
    /// Clone `url`, check out `commit`, verify it, and return a deterministic
    /// tar archive of that commit's tree. `dest` is a caller-provided empty
    /// working directory the implementation may use as scratch.
    ///
    /// Returns [`FetchError::Git`] on any git failure or a commit mismatch,
    /// [`FetchError::TooLarge`] if the archive exceeds `max_bytes`, and must not
    /// return partial/unverified content.
    fn fetch_commit(
        &self,
        url: &str,
        commit: &str,
        dest: &Path,
        max_bytes: u64,
    ) -> Result<Vec<u8>, FetchError>;
}

/// Fetch `source` and, if it matches its pinned hash, store and root it to
/// `owner` in the content-addressed store, returning its address. Nothing is
/// stored on a mismatch or any failure.
///
/// A [`SourceType::Git`] source is handled via `git_fetcher`: the host is
/// SSRF-pre-checked with [`resolve_and_pin`], the repo is cloned and verified
/// against the pinned `commit`, then the deterministic archive bytes are
/// stored and rooted with [`Store::put_referenced`] (the commit is the
/// integrity pin, so there is no sha256 to verify the archive against).
#[allow(clippy::too_many_arguments)]
pub async fn fetch_source(
    source: &Source,
    owner: &str,
    store: &Store,
    downloader: &dyn Downloader,
    git_fetcher: &dyn GitFetcher,
    release_resolver: &dyn ReleaseResolver,
    max_bytes: u64,
) -> Result<ContentHash, FetchError> {
    match source.source_type {
        SourceType::Git => {
            let url = source
                .url
                .as_deref()
                .filter(|u| !u.is_empty())
                .ok_or(FetchError::MissingField("url"))?;
            let commit = source
                .commit
                .as_deref()
                .filter(|c| !c.is_empty())
                .ok_or(FetchError::MissingField("commit"))?;

            // SSRF pre-check: reject a host that resolves into a blocked range
            // before any git process is spawned. Best-effort against rebinding;
            // the commit pin is the content guarantee (see module docs).
            let parsed = reqwest::Url::parse(url)
                .map_err(|e| FetchError::Network(format!("parse git url: {e}")))?;
            // Restrict to https: other transports (git://, ssh://, file://) would
            // not go through the resolved/pinned host check and could read local
            // or internal endpoints, defeating the SSRF guard entirely.
            if parsed.scheme() != "https" {
                return Err(FetchError::Network(format!("non-https git url: {url}")));
            }
            let host = parsed
                .host_str()
                .ok_or_else(|| FetchError::Network(format!("git url has no host: {url}")))?;
            let port = parsed.port_or_known_default().unwrap_or(443);
            match resolve_and_pin(host, port).await {
                Ok(_addr) => {}
                Err(blocked @ GuardError::Blocked { .. }) => {
                    return Err(FetchError::Blocked(blocked.to_string()))
                }
                Err(e) => return Err(FetchError::Network(e.to_string())),
            }

            // Scratch working directory for the clone; dropped (removed) when
            // this returns, success or failure.
            let scratch = tempfile::tempdir()
                .map_err(|e| FetchError::Git(format!("scratch dir: {e}")))?;
            let bytes = git_fetcher.fetch_commit(url, commit, scratch.path(), max_bytes)?;

            // The commit is the integrity pin, not a sha256, so the verified
            // archive is stored and rooted without a content-hash check.
            let hash = store.put_referenced(&bytes, owner)?;
            Ok(hash)
        }
        SourceType::Tarball => {
            let url = source
                .url
                .as_deref()
                .filter(|u| !u.is_empty())
                .ok_or(FetchError::MissingField("url"))?;
            let sha = source
                .sha256
                .as_deref()
                .filter(|s| !s.is_empty())
                .ok_or(FetchError::MissingField("sha256"))?;
            let expected =
                ContentHash::parse(sha).map_err(|_| FetchError::InvalidHash(sha.to_string()))?;
            let bytes = downloader.get(url, max_bytes).await?;
            // The store verifies the bytes against `expected` before writing,
            // so a mismatch stores nothing, and roots atomically on a match.
            let hash = store.put_verified_referenced(&bytes, &expected, owner)?;
            Ok(hash)
        }
        SourceType::GithubRelease => {
            // Follows tags: resolve the release (latest or the pinned tag) and
            // the asset url via the GitHub API, then download it through the
            // redirect-following downloader (the asset 302s to a CDN). A recipe
            // that pins a sha256 (the cookbook path) is verified; without one
            // (a direct install) the resolved content address is the value to
            // lock (TOFU-then-locked, forage-recipes.md §7a/§17a, decision D3).
            let url = source
                .url
                .as_deref()
                .filter(|u| !u.is_empty())
                .ok_or(FetchError::MissingField("url"))?;
            let asset_template = source
                .asset
                .as_deref()
                .filter(|a| !a.is_empty())
                .ok_or(FetchError::MissingField("asset"))?;
            let resolved = release_resolver
                .resolve(url, source.tag.as_deref(), asset_template)
                .await?;
            let bytes = downloader.get(&resolved.asset_url, max_bytes).await?;
            match source.sha256.as_deref().filter(|s| !s.is_empty()) {
                Some(sha) => {
                    let expected = ContentHash::parse(sha)
                        .map_err(|_| FetchError::InvalidHash(sha.to_string()))?;
                    Ok(store.put_verified_referenced(&bytes, &expected, owner)?)
                }
                None => Ok(store.put_referenced(&bytes, owner)?),
            }
        }
        SourceType::Local => {
            // A local path (development source): not content-addressable across
            // machines, so it is build-locally-only (forage-recipes.md §17a).
            // Archive it deterministically and root it; no hash pin.
            let path = source
                .url
                .as_deref()
                .filter(|u| !u.is_empty())
                .ok_or(FetchError::MissingField("url"))?;
            let root = Path::new(path);
            if !root.is_absolute() {
                return Err(FetchError::Local(format!("path must be absolute: {path}")));
            }
            if !root.exists() {
                return Err(FetchError::Local(format!("path not found: {path}")));
            }
            let bytes = archive_local_path(root)?;
            let hash = store.put_referenced(&bytes, owner)?;
            Ok(hash)
        }
        SourceType::Crate => {
            // `url` is the bare crate name; `version` is the source-level crate
            // version (the pipeline defaults it to `recipe.version`); `sha256`
            // pins the content (decision D6).
            let name = source
                .url
                .as_deref()
                .filter(|u| !u.is_empty())
                .ok_or(FetchError::MissingField("url"))?;
            let version = source
                .version
                .as_deref()
                .filter(|v| !v.is_empty())
                .ok_or(FetchError::MissingField("version"))?;
            let sha = source
                .sha256
                .as_deref()
                .filter(|s| !s.is_empty())
                .ok_or(FetchError::MissingField("sha256"))?;
            let expected =
                ContentHash::parse(sha).map_err(|_| FetchError::InvalidHash(sha.to_string()))?;
            // Name and version are interpolated into the download URL path, so
            // restrict them to the crates.io character set: a crafted value
            // (`/`, `?`, `..`, whitespace) could otherwise inject extra path or
            // query segments into the request.
            if !is_crate_name(name) {
                return Err(FetchError::InvalidCrate(format!("name {name:?}")));
            }
            if !is_crate_version(version) {
                return Err(FetchError::InvalidCrate(format!("version {version:?}")));
            }
            // The direct, redirect-free crates.io download URL; the downloader
            // pins the resolved host against SSRF and the store verifies the
            // bytes against the pinned sha256 before rooting them.
            let url = format!("https://static.crates.io/crates/{name}/{name}-{version}.crate");
            let bytes = downloader.get(&url, max_bytes).await?;
            let hash = store.put_verified_referenced(&bytes, &expected, owner)?;
            Ok(hash)
        }
    }
}

/// Deterministically archive a local path (directory or file) into a tar the
/// store can hold and [`arlen_forage_extract`] can later unpack. Entries are
/// walked in sorted order with zeroed mtime and normalised modes, so the same
/// tree yields the same bytes. Symlinks and special files are rejected (they
/// would not survive the safe extraction and could point outside the tree).
fn archive_local_path(root: &Path) -> Result<Vec<u8>, FetchError> {
    let mut entries: Vec<(String, std::path::PathBuf, bool)> = Vec::new();
    collect_sorted(root, root, &mut entries)?;

    let mut builder = tar::Builder::new(Vec::new());
    builder.mode(tar::HeaderMode::Deterministic);
    for (rel, abs, is_dir) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_mtime(0);
        header.set_uid(0);
        header.set_gid(0);
        if is_dir {
            header.set_entry_type(tar::EntryType::Directory);
            header.set_mode(0o755);
            header.set_size(0);
            builder
                .append_data(&mut header, &rel, std::io::empty())
                .map_err(|e| FetchError::Local(e.to_string()))?;
        } else {
            let data = std::fs::read(&abs).map_err(|e| FetchError::Local(e.to_string()))?;
            header.set_entry_type(tar::EntryType::Regular);
            header.set_mode(0o644);
            header.set_size(data.len() as u64);
            builder
                .append_data(&mut header, &rel, &data[..])
                .map_err(|e| FetchError::Local(e.to_string()))?;
        }
    }
    builder.into_inner().map_err(|e| FetchError::Local(e.to_string()))
}

/// Recurse `dir`, collecting `(relative_path, absolute_path, is_dir)` entries
/// sorted by name at each level. Rejects symlinks and special files.
fn collect_sorted(
    root: &Path,
    dir: &Path,
    out: &mut Vec<(String, std::path::PathBuf, bool)>,
) -> Result<(), FetchError> {
    let mut names: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| FetchError::Local(e.to_string()))?
        .map(|e| e.map(|e| e.file_name()))
        .collect::<Result<_, _>>()
        .map_err(|e| FetchError::Local(e.to_string()))?;
    names.sort();
    for name in names {
        let abs = dir.join(&name);
        let meta = std::fs::symlink_metadata(&abs).map_err(|e| FetchError::Local(e.to_string()))?;
        let ft = meta.file_type();
        let rel = abs
            .strip_prefix(root)
            .map_err(|_| FetchError::Local("path escaped local root".into()))?
            .to_str()
            .ok_or_else(|| FetchError::Local("non-UTF8 path in local source".into()))?
            .to_string();
        if ft.is_symlink() {
            return Err(FetchError::Local(format!("symlink not supported: {rel}")));
        }
        if ft.is_dir() {
            out.push((rel, abs.clone(), true));
            collect_sorted(root, &abs, out)?;
        } else if ft.is_file() {
            out.push((rel, abs, false));
        } else {
            return Err(FetchError::Local(format!("unsupported file type: {rel}")));
        }
    }
    Ok(())
}

/// The production [`Downloader`]: reqwest over rustls, HTTPS only, no redirects,
/// the host SSRF-guarded and DNS-pinned (resolve ourselves, pin reqwest to the
/// verified address so rebinding cannot swap the target), with a streamed body
/// capped at `max_bytes`.
#[derive(Debug, Default)]
pub struct HttpDownloader;

#[async_trait]
impl Downloader for HttpDownloader {
    async fn get(&self, url: &str, max_bytes: u64) -> Result<Vec<u8>, FetchError> {
        let parsed =
            reqwest::Url::parse(url).map_err(|e| FetchError::Network(format!("parse url: {e}")))?;
        if parsed.scheme() != "https" {
            return Err(FetchError::Network(format!(
                "non-https source url: {url}"
            )));
        }
        let host = parsed
            .host_str()
            .ok_or_else(|| FetchError::Network(format!("url has no host: {url}")))?;
        let port = parsed.port_or_known_default().unwrap_or(443);

        // SSRF guard + DNS-rebinding pin: resolve ourselves, reject blocked
        // ranges, pin reqwest to the verified address.
        let addr = match resolve_and_pin(host, port).await {
            Ok(addr) => addr,
            Err(blocked @ GuardError::Blocked { .. }) => {
                return Err(FetchError::Blocked(blocked.to_string()))
            }
            Err(e) => return Err(FetchError::Network(e.to_string())),
        };

        let client = reqwest::Client::builder()
            .https_only(true)
            // Ignore any HTTPS_PROXY/ALL_PROXY in the environment: a proxy would
            // be contacted without running through resolve_and_pin, bypassing
            // the SSRF/DNS-rebinding guard. We resolve and pin the origin
            // ourselves, so no implicit proxy may sit in front of it.
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(120))
            .user_agent(concat!("Arlen-forage/", env!("CARGO_PKG_VERSION")))
            .resolve(host, addr)
            .build()
            .map_err(|e| FetchError::Network(format!("client: {e}")))?;

        let resp = client
            .get(url)
            .send()
            .await
            .map_err(|e| FetchError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(FetchError::Network(format!("status {}", resp.status())));
        }

        // Stream with a hard cap so a hostile server cannot exhaust memory.
        use futures_util::StreamExt;
        let mut buf: Vec<u8> = Vec::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| FetchError::Network(format!("body: {e}")))?;
            if buf.len() as u64 + chunk.len() as u64 > max_bytes {
                return Err(FetchError::TooLarge { limit: max_bytes });
            }
            buf.extend_from_slice(&chunk);
        }
        Ok(buf)
    }
}

/// Wall-clock bound on any single git invocation. A hostile or degraded remote
/// (a hanging fetch) is killed rather than wedging the fetch worker.
const GIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// The production [`GitFetcher`]: drives the `git` CLI via [`Command`] with
/// explicit arguments (never a shell) under a **sanitized environment**, a
/// **wall-clock timeout**, and a **bounded** archive read.
///
/// The environment is cleared and only a minimal allowlist is set, so no
/// inherited git config or environment can redirect the fetch: `~/.gitconfig`
/// (`url.*.insteadOf`, proxies), `GIT_CONFIG_GLOBAL`/`GIT_CONFIG_*`,
/// `GIT_SSH`/`GIT_SSH_COMMAND`, `http(s)_proxy`/`ALL_PROXY` and friends are all
/// dropped; global and system config are pointed at `/dev/null`. This closes
/// the SSRF gap where `GIT_CONFIG_NOSYSTEM` alone left global config and proxy
/// variables able to send git to a private endpoint after the pre-check.
#[derive(Debug, Default)]
pub struct ProcessGitFetcher;

impl ProcessGitFetcher {
    /// Run `git` with explicit args in `cwd`, a sanitized env, a timeout, and an
    /// optional stdout byte cap. Returns stdout on success (capped if `cap` is
    /// set), or an error on non-zero exit, timeout, or cap overflow.
    fn git(&self, cwd: &Path, args: &[&str], cap: Option<u64>) -> Result<Vec<u8>, FetchError> {
        use std::io::Read;
        use std::process::Stdio;
        use wait_timeout::ChildExt;

        let arg0 = args.first().copied().unwrap_or("");
        let mut child = Command::new("git")
            .args(args)
            .current_dir(cwd)
            // Sanitized environment: clear everything, then set only the minimum.
            // GIT_CONFIG_GLOBAL/SYSTEM=/dev/null disable user/system config
            // (insteadOf, proxies); env_clear drops proxy and GIT_SSH* vars.
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", cwd)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| FetchError::Git(format!("spawn git {arg0}: {e}")))?;

        // Drain stdout (capped) and stderr concurrently so a full pipe never
        // blocks the child before it can exit.
        let mut out = child.stdout.take().expect("stdout piped");
        let mut err = child.stderr.take().expect("stderr piped");
        let out_handle = std::thread::spawn(move || read_capped(&mut out, cap));
        let err_handle = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = err.read_to_end(&mut buf);
            buf
        });

        let status = match child.wait_timeout(GIT_TIMEOUT) {
            Ok(Some(status)) => status,
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = out_handle.join();
                let _ = err_handle.join();
                return Err(FetchError::Git(format!("git {arg0} timed out")));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(FetchError::Git(format!("git {arg0} wait: {e}")));
            }
        };

        let stdout = out_handle
            .join()
            .map_err(|_| FetchError::Git(format!("git {arg0} stdout reader panicked")))??;
        let stderr = err_handle.join().unwrap_or_default();

        if !status.success() {
            return Err(FetchError::Git(format!(
                "git {arg0} failed ({status}): {}",
                String::from_utf8_lossy(&stderr).trim()
            )));
        }
        Ok(stdout)
    }
}

impl GitFetcher for ProcessGitFetcher {
    fn fetch_commit(
        &self,
        url: &str,
        commit: &str,
        dest: &Path,
        max_bytes: u64,
    ) -> Result<Vec<u8>, FetchError> {
        // init an empty repo, fetch only the pinned commit shallowly, check it
        // out detached. Fetching the commit directly (rather than cloning a
        // branch) keeps the transfer minimal and never depends on a default
        // branch. `--` terminates option parsing so a url/commit can never be
        // read as a flag.
        self.git(dest, &["init", "--quiet"], None)?;
        self.git(
            dest,
            &["fetch", "--depth", "1", "--no-tags", "--", url, commit],
            None,
        )?;
        self.git(dest, &["checkout", "--quiet", "FETCH_HEAD"], None)?;

        // Verify the checkout is exactly the declared commit. git resolves
        // FETCH_HEAD, so a server that served a different object than asked
        // (or an abbreviated/ambiguous ref) is caught here.
        let head = self.git(dest, &["rev-parse", "HEAD"], None)?;
        let head = String::from_utf8_lossy(&head);
        let head = head.trim();
        if !head.eq_ignore_ascii_case(commit) {
            return Err(FetchError::Git(format!(
                "checkout is {head}, expected pinned commit {commit}"
            )));
        }

        // Deterministic archive of the pinned tree, bounded by the size cap so a
        // pinned-but-huge tree cannot exhaust memory.
        self.git(dest, &["archive", "--format=tar", commit], Some(max_bytes))
    }
}

/// Whether `name` is a valid crates.io crate name: non-empty, at most 64 chars
/// (the registry limit), and only `[A-Za-z0-9_-]`, so it is a safe single URL
/// path segment.
fn is_crate_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Whether `version` is a safe crate version segment: non-empty, bounded, and
/// only semver characters `[A-Za-z0-9.+-]` (no `/`, `..`, whitespace, or
/// URL-special characters that could warp the download URL).
fn is_crate_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 64
        && version != ".."
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-'))
}

/// The default cap on a cloned recipe repository's working tree, so a hostile
/// `git+URL` cannot fill the disk. Recipe repos are small (a `recipe.toml` and a
/// handful of patches); 64 MiB is generous.
pub const DEFAULT_RECIPE_REPO_BYTES: u64 = 64 * 1024 * 1024;

/// Shallow-clone a recipe repository's working tree into `dest` (an existing,
/// empty directory) for `forage install git+URL`. Unlike
/// [`ProcessGitFetcher::fetch_commit`], which archives a *pinned source* commit,
/// this checks out the repo's files (the recipe and its patches) at an optional
/// branch or tag, or the default branch.
///
/// Hardened like the source git fetch: https-only, an SSRF pre-check on the
/// resolved host, the sanitized environment and timeout of [`ProcessGitFetcher`],
/// and `--depth 1`. After the clone, the working tree is size-checked against
/// `max_bytes`. Residual: the cap is enforced only after the clone has written
/// to disk, so the transient peak is bounded by `--depth 1` rather than the cap;
/// recipe repos are small and `dest` is a caller-owned temporary removed on
/// failure. The SSRF pin is best-effort (git re-resolves DNS), matching the
/// pinned-source caveat in the module docs.
pub async fn clone_recipe_repo(
    url: &str,
    git_ref: Option<&str>,
    dest: &Path,
    max_bytes: u64,
) -> Result<(), FetchError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|e| FetchError::Network(format!("parse git url: {e}")))?;
    if parsed.scheme() != "https" {
        return Err(FetchError::Network(format!("non-https git url: {url}")));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| FetchError::Network(format!("git url has no host: {url}")))?;
    let port = parsed.port_or_known_default().unwrap_or(443);
    match resolve_and_pin(host, port).await {
        Ok(_addr) => {}
        Err(blocked @ GuardError::Blocked { .. }) => {
            return Err(FetchError::Blocked(blocked.to_string()))
        }
        Err(e) => return Err(FetchError::Network(e.to_string())),
    }

    let cwd = dest.parent().unwrap_or_else(|| Path::new("."));
    let dest_str = dest
        .to_str()
        .ok_or_else(|| FetchError::Git("clone destination is not valid UTF-8".into()))?;

    // `--` terminates option parsing so the url or dest can never be read as a
    // flag. The ref is passed as a single `--branch=<ref>` token (not a separate
    // value) so a ref beginning with `-` can never be parsed as its own option,
    // independent of git's positional-consumption behaviour. `--branch` accepts
    // a branch or a tag. No `--recurse-submodules`: submodules are neither
    // fetched nor executed, and the cloned tree's hooks/filters never run
    // (a fresh clone uses local hook samples, and config is neutralised).
    let branch_arg: String;
    let mut args: Vec<&str> = vec!["clone", "--depth", "1", "--no-tags", "--quiet"];
    if let Some(r) = git_ref {
        branch_arg = format!("--branch={r}");
        args.push(&branch_arg);
    }
    args.push("--");
    args.push(url);
    args.push(dest_str);
    ProcessGitFetcher.git(cwd, &args, None)?;

    let size = dir_size(dest)?;
    if size > max_bytes {
        return Err(FetchError::TooLarge { limit: max_bytes });
    }
    Ok(())
}

/// Sum the byte size of every regular file under `root`, not following
/// symlinks, for the post-clone size guard.
fn dir_size(root: &Path) -> Result<u64, FetchError> {
    let mut total = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries =
            std::fs::read_dir(&dir).map_err(|e| FetchError::Git(format!("read clone dir: {e}")))?;
        for entry in entries {
            let entry = entry.map_err(|e| FetchError::Git(format!("clone dir entry: {e}")))?;
            let meta = entry
                .path()
                .symlink_metadata()
                .map_err(|e| FetchError::Git(format!("clone stat: {e}")))?;
            let ft = meta.file_type();
            if ft.is_dir() {
                stack.push(entry.path());
            } else if ft.is_file() {
                total = total.saturating_add(meta.len());
            }
            // Symlinks contribute nothing and are not followed.
        }
    }
    Ok(total)
}

/// Read `src` to EOF, returning its bytes, but fail with [`FetchError::TooLarge`]
/// if it exceeds `cap`. When the cap is hit, the rest is drained and discarded
/// (so the producer is not left blocked on a full pipe) and `TooLarge` is
/// returned. `cap = None` reads without a limit.
fn read_capped<R: std::io::Read>(src: &mut R, cap: Option<u64>) -> Result<Vec<u8>, FetchError> {
    let mut buf = Vec::new();
    let mut scratch = [0u8; 64 * 1024];
    let mut total: u64 = 0;
    let mut exceeded = false;
    loop {
        let n = src
            .read(&mut scratch)
            .map_err(|e| FetchError::Git(format!("read git output: {e}")))?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if let Some(cap) = cap {
            if total > cap {
                exceeded = true; // keep draining to unblock the child, discard
                continue;
            }
        }
        if !exceeded {
            buf.extend_from_slice(&scratch[..n]);
        }
    }
    if exceeded {
        return Err(FetchError::TooLarge {
            limit: cap.unwrap_or(0),
        });
    }
    Ok(buf)
}

/// Maximum redirect hops a [`RedirectingHttpDownloader`] follows before
/// failing, matching modulesd's stricter-than-default cap.
const MAX_REDIRECT_HOPS: usize = 5;

/// One HTTPS response with redirects NOT followed: the status, the `Location`
/// header (for a 3xx), and the body (for a 2xx, already size-capped).
#[derive(Debug)]
pub struct OneResponse {
    /// HTTP status code.
    pub status: u16,
    /// The `Location` header value, if present.
    pub location: Option<String>,
    /// The response body (only populated/capped for a success).
    pub body: Vec<u8>,
}

/// A single SSRF-pinned, no-redirect HTTPS GET. Behind a trait so the
/// redirect-following loop can be tested without network.
#[async_trait]
pub trait SingleRequest: Send + Sync {
    /// Perform one GET of `url` (which the caller has already checked is https),
    /// resolving and pinning the host, returning the status + `Location` + a
    /// body capped at `max_bytes`.
    async fn get_once(&self, url: &str, max_bytes: u64) -> Result<OneResponse, FetchError>;
}

/// Follow redirects manually so each hop is SSRF-checked: every hop must be
/// https and is resolved-and-pinned by the [`SingleRequest`] before connecting;
/// the chain is capped at [`MAX_REDIRECT_HOPS`] and the final body at
/// `max_bytes`. The content is still verified by hash downstream, so a redirect
/// can change *which server* is reached but not what content is accepted.
pub async fn follow_redirects(
    req: &dyn SingleRequest,
    url: &str,
    max_bytes: u64,
) -> Result<Vec<u8>, FetchError> {
    let mut current = url.to_string();
    for _ in 0..=MAX_REDIRECT_HOPS {
        let parsed = reqwest::Url::parse(&current)
            .map_err(|e| FetchError::Network(format!("parse url: {e}")))?;
        if parsed.scheme() != "https" {
            return Err(FetchError::Network(format!("non-https hop: {current}")));
        }
        let resp = req.get_once(&current, max_bytes).await?;
        if (300..400).contains(&resp.status) {
            let loc = resp
                .location
                .ok_or_else(|| FetchError::Network(format!("redirect {} without Location", resp.status)))?;
            // Resolve a possibly-relative Location against the current URL.
            current = parsed
                .join(&loc)
                .map_err(|e| FetchError::Network(format!("bad redirect target: {e}")))?
                .to_string();
            continue;
        }
        if (200..300).contains(&resp.status) {
            return Ok(resp.body);
        }
        return Err(FetchError::Network(format!("status {}", resp.status)));
    }
    Err(FetchError::Network(format!(
        "more than {MAX_REDIRECT_HOPS} redirects"
    )))
}

/// The production [`SingleRequest`]: one reqwest GET over rustls, https-only,
/// no redirects, no proxy, the host SSRF-guarded and DNS-pinned, body streamed
/// and capped. Used by [`RedirectingHttpDownloader`] for each hop.
#[derive(Debug, Default)]
pub struct PinnedSingleRequest;

#[async_trait]
impl SingleRequest for PinnedSingleRequest {
    async fn get_once(&self, url: &str, max_bytes: u64) -> Result<OneResponse, FetchError> {
        let parsed =
            reqwest::Url::parse(url).map_err(|e| FetchError::Network(format!("parse url: {e}")))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| FetchError::Network(format!("url has no host: {url}")))?;
        let port = parsed.port_or_known_default().unwrap_or(443);
        let addr = match resolve_and_pin(host, port).await {
            Ok(addr) => addr,
            Err(blocked @ GuardError::Blocked { .. }) => {
                return Err(FetchError::Blocked(blocked.to_string()))
            }
            Err(e) => return Err(FetchError::Network(e.to_string())),
        };
        let client = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(120))
            .user_agent(concat!("Arlen-forage/", env!("CARGO_PKG_VERSION")))
            .resolve(host, addr)
            .build()
            .map_err(|e| FetchError::Network(format!("client: {e}")))?;
        let resp = client
            .get(url)
            .send()
            .await
            .map_err(|e| FetchError::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        let location = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        // Only read a body for a success; a 3xx body is irrelevant.
        let body = if (200..300).contains(&status) {
            use futures_util::StreamExt;
            let mut buf = Vec::new();
            let mut stream = resp.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| FetchError::Network(format!("body: {e}")))?;
                if buf.len() as u64 + chunk.len() as u64 > max_bytes {
                    return Err(FetchError::TooLarge { limit: max_bytes });
                }
                buf.extend_from_slice(&chunk);
            }
            buf
        } else {
            Vec::new()
        };
        Ok(OneResponse {
            status,
            location,
            body,
        })
    }
}

/// A [`Downloader`] that follows redirects with a per-hop SSRF check (for hosts
/// like GitHub release / crates.io that 3xx to a CDN). Each hop is https-only
/// and DNS-pinned; the chain is hop-capped and the body size-capped.
#[derive(Debug, Default)]
pub struct RedirectingHttpDownloader;

#[async_trait]
impl Downloader for RedirectingHttpDownloader {
    async fn get(&self, url: &str, max_bytes: u64) -> Result<Vec<u8>, FetchError> {
        follow_redirects(&PinnedSingleRequest, url, max_bytes).await
    }
}

/// A resolved GitHub release: the concrete version (the tag) and the download
/// URL of the matched asset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRelease {
    /// The release tag (substituted for `{version}`).
    pub version: String,
    /// The asset download URL (which typically 3xx-redirects to a CDN).
    pub asset_url: String,
}

/// Resolves a `github-release` source to a concrete asset URL. Behind a trait
/// so the fetch logic is tested without hitting the GitHub API.
#[async_trait]
pub trait ReleaseResolver: Send + Sync {
    /// Resolve the repo at `repo_url` (a `github.com/{owner}/{repo}` URL): the
    /// release is the pinned `tag` or the latest, and the asset is the one whose
    /// name matches `asset_template` (with `{version}`/`{target}` substituted).
    async fn resolve(
        &self,
        repo_url: &str,
        tag: Option<&str>,
        asset_template: &str,
    ) -> Result<ResolvedRelease, FetchError>;
}

/// The production [`ReleaseResolver`]: queries the GitHub releases API over the
/// SSRF-pinned, redirect-following path for a fixed platform `target`.
#[derive(Debug, Clone)]
pub struct GitHubReleaseResolver {
    target: String,
}

impl GitHubReleaseResolver {
    /// Resolve for the given platform target (used to substitute `{target}`).
    pub fn new(target: impl Into<String>) -> Self {
        Self {
            target: target.into(),
        }
    }

    /// The running platform's target string, e.g. `x86_64-linux`.
    pub fn host_target() -> String {
        format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS)
    }
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

#[async_trait]
impl ReleaseResolver for GitHubReleaseResolver {
    async fn resolve(
        &self,
        repo_url: &str,
        tag: Option<&str>,
        asset_template: &str,
    ) -> Result<ResolvedRelease, FetchError> {
        let (owner, repo) = parse_github_repo(repo_url)?;
        let api = match tag {
            Some(t) => format!("https://api.github.com/repos/{owner}/{repo}/releases/tags/{t}"),
            None => format!("https://api.github.com/repos/{owner}/{repo}/releases/latest"),
        };
        // The API response is small JSON; cap it well below a source artifact.
        let body = follow_redirects(&PinnedSingleRequest, &api, 4 * 1024 * 1024).await?;
        let release: GhRelease = serde_json::from_slice(&body)
            .map_err(|e| FetchError::Network(format!("github api json: {e}")))?;
        let want = substitute_asset(asset_template, &release.tag_name, &self.target);
        let asset = release
            .assets
            .iter()
            .find(|a| a.name == want)
            .ok_or_else(|| {
                FetchError::Network(format!("no asset `{want}` in release {}", release.tag_name))
            })?;
        Ok(ResolvedRelease {
            version: release.tag_name,
            asset_url: asset.browser_download_url.clone(),
        })
    }
}

/// Parse a `github.com/{owner}/{repo}` URL into its owner and repo. Only the
/// `github.com` host is accepted (the API is GitHub-specific).
fn parse_github_repo(url: &str) -> Result<(String, String), FetchError> {
    let u = reqwest::Url::parse(url)
        .map_err(|e| FetchError::Network(format!("parse repo url: {e}")))?;
    // Accept the absolute-FQDN spelling `github.com.` (a single trailing dot)
    // by normalising it away before the exact host comparison.
    let host = u.host_str().map(|h| h.strip_suffix('.').unwrap_or(h));
    if host != Some("github.com") {
        return Err(FetchError::Network(format!(
            "github-release url must be on github.com: {url}"
        )));
    }
    let mut segs = u.path_segments().into_iter().flatten();
    let owner = segs
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| FetchError::Network(format!("github url missing owner: {url}")))?;
    let repo = segs
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| FetchError::Network(format!("github url missing repo: {url}")))?
        .trim_end_matches(".git");
    Ok((owner.to_string(), repo.to_string()))
}

/// Substitute `{version}` and `{target}` in an asset name template.
fn substitute_asset(template: &str, version: &str, target: &str) -> String {
    template
        .replace("{version}", version)
        .replace("{target}", target)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A downloader that returns canned bytes (or a canned error), so the fetch
    /// logic is tested without network.
    struct MockDownloader {
        result: std::sync::Mutex<Option<Result<Vec<u8>, FetchError>>>,
    }
    impl MockDownloader {
        fn ok(bytes: &[u8]) -> Self {
            Self {
                result: std::sync::Mutex::new(Some(Ok(bytes.to_vec()))),
            }
        }
        fn err(e: FetchError) -> Self {
            Self {
                result: std::sync::Mutex::new(Some(Err(e))),
            }
        }
    }
    #[async_trait]
    impl Downloader for MockDownloader {
        async fn get(&self, _url: &str, _max_bytes: u64) -> Result<Vec<u8>, FetchError> {
            self.result
                .lock()
                .unwrap()
                .take()
                .expect("mock downloader called once")
        }
    }

    /// A git fetcher that returns canned archive bytes (or a canned error)
    /// without touching real git or the network.
    struct MockGitFetcher {
        result: std::sync::Mutex<Option<Result<Vec<u8>, FetchError>>>,
    }
    impl MockGitFetcher {
        fn ok(bytes: &[u8]) -> Self {
            Self {
                result: std::sync::Mutex::new(Some(Ok(bytes.to_vec()))),
            }
        }
        fn err(e: FetchError) -> Self {
            Self {
                result: std::sync::Mutex::new(Some(Err(e))),
            }
        }
        /// A fetcher that must never be called (asserts if it is).
        fn never() -> Self {
            Self {
                result: std::sync::Mutex::new(None),
            }
        }
    }
    impl GitFetcher for MockGitFetcher {
        fn fetch_commit(
            &self,
            _url: &str,
            _commit: &str,
            _dest: &Path,
            _max_bytes: u64,
        ) -> Result<Vec<u8>, FetchError> {
            self.result
                .lock()
                .unwrap()
                .take()
                .expect("mock git fetcher called without a canned result")
        }
    }

    /// The HTTP downloader is never reached on the git path; use this to assert
    /// that.
    fn no_dl() -> MockDownloader {
        MockDownloader {
            result: std::sync::Mutex::new(None),
        }
    }

    /// A release resolver that must not be called (non-github-release paths).
    struct NoResolver;
    #[async_trait]
    impl ReleaseResolver for NoResolver {
        async fn resolve(&self, _: &str, _: Option<&str>, _: &str) -> Result<ResolvedRelease, FetchError> {
            panic!("release resolver must not be used for this source type")
        }
    }
    fn no_resolver() -> NoResolver {
        NoResolver
    }

    /// A resolver that returns a canned asset url.
    struct MockResolver {
        asset_url: String,
    }
    #[async_trait]
    impl ReleaseResolver for MockResolver {
        async fn resolve(&self, _: &str, _: Option<&str>, _: &str) -> Result<ResolvedRelease, FetchError> {
            Ok(ResolvedRelease {
                version: "v1.2.3".into(),
                asset_url: self.asset_url.clone(),
            })
        }
    }

    fn release(url: &str, asset: &str, sha256: Option<&str>) -> Source {
        Source {
            source_type: SourceType::GithubRelease,
            url: Some(url.into()),
            commit: None,
            sha256: sha256.map(|s| s.into()),
            asset: Some(asset.into()),
            tag: None,
            version: None,
            patches: Vec::new(),
        }
    }

    #[test]
    fn parse_github_repo_and_substitute() {
        assert_eq!(
            parse_github_repo("https://github.com/zed-industries/zed").unwrap(),
            ("zed-industries".into(), "zed".into())
        );
        assert_eq!(
            parse_github_repo("https://github.com/o/r.git").unwrap(),
            ("o".into(), "r".into())
        );
        // Only github.com (lookalikes rejected), but the trailing-dot FQDN ok.
        assert!(parse_github_repo("https://gitlab.com/o/r").is_err());
        assert!(parse_github_repo("https://github.com.evil/o/r").is_err());
        assert!(parse_github_repo("https://github.com/o").is_err());
        assert_eq!(
            parse_github_repo("https://github.com./o/r").unwrap(),
            ("o".into(), "r".into())
        );
        assert_eq!(
            substitute_asset("zed-{version}-{target}.tar.gz", "v0.1", "x86_64-linux"),
            "zed-v0.1-x86_64-linux.tar.gz"
        );
    }

    #[tokio::test]
    async fn github_release_pinned_sha_is_verified_and_stored() {
        let (_d, s) = store();
        let body = b"released asset bytes";
        let sha = ContentHash::of(body);
        let src = release("https://github.com/o/r", "app-{version}.tar.gz", Some(sha.as_str()));
        let resolver = MockResolver {
            asset_url: "https://example.org/asset".into(),
        };
        let h = fetch_source(&src, "org.example.app", &s, &MockDownloader::ok(body), &MockGitFetcher::never(), &resolver, DEFAULT_MAX_BYTES)
            .await
            .unwrap();
        assert_eq!(h, sha);
        assert_eq!(s.refcount(&h).unwrap(), 1);
    }

    #[tokio::test]
    async fn github_release_tofu_stores_and_returns_the_resolved_hash() {
        let (_d, s) = store();
        let body = b"asset without a pinned sha";
        let src = release("https://github.com/o/r", "app-{version}.tar.gz", None);
        let resolver = MockResolver {
            asset_url: "https://example.org/asset".into(),
        };
        let h = fetch_source(&src, "org.example.app", &s, &MockDownloader::ok(body), &MockGitFetcher::never(), &resolver, DEFAULT_MAX_BYTES)
            .await
            .unwrap();
        // TOFU: the returned content address is the value to lock.
        assert_eq!(h, ContentHash::of(body));
        assert_eq!(s.refcount(&h).unwrap(), 1);
    }

    #[tokio::test]
    async fn github_release_sha_mismatch_stores_nothing() {
        let (_d, s) = store();
        let declared = ContentHash::of(b"what the cookbook pinned");
        let src = release("https://github.com/o/r", "app-{version}.tar.gz", Some(declared.as_str()));
        let resolver = MockResolver {
            asset_url: "https://example.org/asset".into(),
        };
        let err = fetch_source(&src, "o", &s, &MockDownloader::ok(b"server served something else"), &MockGitFetcher::never(), &resolver, DEFAULT_MAX_BYTES)
            .await
            .unwrap_err();
        assert!(matches!(err, FetchError::Store(StoreError::Mismatch { .. })));
        assert!(!s.has(&declared));
    }

    #[tokio::test]
    async fn github_release_missing_asset_template_is_rejected() {
        let (_d, s) = store();
        let mut src = release("https://github.com/o/r", "x", None);
        src.asset = None;
        let err = fetch_source(&src, "o", &s, &no_dl(), &MockGitFetcher::never(), &no_resolver(), DEFAULT_MAX_BYTES)
            .await
            .unwrap_err();
        assert!(matches!(err, FetchError::MissingField("asset")));
    }

    #[test]
    fn read_capped_enforces_the_limit() {
        // Under the cap: returns the bytes.
        let mut under = std::io::Cursor::new(vec![1u8; 100]);
        assert_eq!(read_capped(&mut under, Some(200)).unwrap().len(), 100);
        // Over the cap: TooLarge, even though the reader has more to give.
        let mut over = std::io::Cursor::new(vec![1u8; 300]);
        assert!(matches!(
            read_capped(&mut over, Some(200)),
            Err(FetchError::TooLarge { limit: 200 })
        ));
        // No cap: reads everything.
        let mut all = std::io::Cursor::new(vec![1u8; 300]);
        assert_eq!(read_capped(&mut all, None).unwrap().len(), 300);
    }

    /// A SingleRequest that returns a queued sequence of responses (one per
    /// call), so the redirect-following loop is tested without network.
    struct MockSingle {
        responses: std::sync::Mutex<std::collections::VecDeque<OneResponse>>,
    }
    impl MockSingle {
        fn new(responses: Vec<OneResponse>) -> Self {
            Self {
                responses: std::sync::Mutex::new(responses.into()),
            }
        }
    }
    #[async_trait]
    impl SingleRequest for MockSingle {
        async fn get_once(&self, _url: &str, _max: u64) -> Result<OneResponse, FetchError> {
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| FetchError::Network("mock exhausted".into()))
        }
    }
    fn redirect_to(loc: &str) -> OneResponse {
        OneResponse {
            status: 302,
            location: Some(loc.into()),
            body: Vec::new(),
        }
    }
    fn ok_body(body: &[u8]) -> OneResponse {
        OneResponse {
            status: 200,
            location: None,
            body: body.to_vec(),
        }
    }

    #[tokio::test]
    async fn follow_redirects_chases_to_the_final_body() {
        let req = MockSingle::new(vec![
            redirect_to("https://cdn.example/a"),
            redirect_to("https://cdn.example/b"),
            ok_body(b"final content"),
        ]);
        let body = follow_redirects(&req, "https://example.org/start", DEFAULT_MAX_BYTES)
            .await
            .unwrap();
        assert_eq!(body, b"final content");
    }

    #[tokio::test]
    async fn follow_redirects_caps_hops() {
        // Always redirecting -> error after the hop cap.
        let loop_resps: Vec<OneResponse> =
            (0..20).map(|_| redirect_to("https://example.org/again")).collect();
        let req = MockSingle::new(loop_resps);
        assert!(matches!(
            follow_redirects(&req, "https://example.org/x", DEFAULT_MAX_BYTES).await,
            Err(FetchError::Network(_))
        ));
    }

    #[tokio::test]
    async fn follow_redirects_rejects_non_https_hop() {
        let req = MockSingle::new(vec![redirect_to("http://evil.internal/x")]);
        assert!(matches!(
            follow_redirects(&req, "https://example.org/x", DEFAULT_MAX_BYTES).await,
            Err(FetchError::Network(_))
        ));
    }

    #[tokio::test]
    async fn follow_redirects_surfaces_non_success_status() {
        let req = MockSingle::new(vec![OneResponse {
            status: 404,
            location: None,
            body: Vec::new(),
        }]);
        assert!(matches!(
            follow_redirects(&req, "https://example.org/x", DEFAULT_MAX_BYTES).await,
            Err(FetchError::Network(_))
        ));
    }

    fn local(path: &str) -> Source {
        Source {
            source_type: SourceType::Local,
            url: Some(path.into()),
            commit: None,
            sha256: None,
            asset: None,
            tag: None,
            version: None,
            patches: Vec::new(),
        }
    }

    #[tokio::test]
    async fn local_source_is_archived_and_rooted() {
        use std::fs;
        let (_d, s) = store();
        let src_dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(src_dir.path().join("src")).unwrap();
        fs::write(src_dir.path().join("src/main.rs"), b"fn main(){}").unwrap();
        fs::write(src_dir.path().join("Cargo.toml"), b"[package]").unwrap();

        let src = local(src_dir.path().to_str().unwrap());
        let h = fetch_source(&src, "org.example.app", &s, &no_dl(), &MockGitFetcher::never(), &no_resolver(), DEFAULT_MAX_BYTES)
            .await
            .unwrap();
        assert_eq!(s.refcount(&h).unwrap(), 1);

        // The stored archive is a deterministic tar the extractor accepts.
        let bytes = s.read(&h).unwrap();
        let out = tempfile::tempdir().unwrap();
        arlen_forage_extract::extract_tar(&bytes, out.path(), &Default::default()).unwrap();
        assert_eq!(fs::read(out.path().join("src/main.rs")).unwrap(), b"fn main(){}");

        // Determinism: archiving the same tree again yields the same address.
        let h2 = fetch_source(&src, "org.example.app", &s, &no_dl(), &MockGitFetcher::never(), &no_resolver(), DEFAULT_MAX_BYTES)
            .await
            .unwrap();
        assert_eq!(h, h2);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn local_source_rejects_symlinks() {
        use std::os::unix::fs::symlink;
        let (_d, s) = store();
        let src_dir = tempfile::tempdir().unwrap();
        std::fs::write(src_dir.path().join("real"), b"x").unwrap();
        symlink("/etc/passwd", src_dir.path().join("link")).unwrap();
        let src = local(src_dir.path().to_str().unwrap());
        assert!(matches!(
            fetch_source(&src, "o", &s, &no_dl(), &MockGitFetcher::never(), &no_resolver(), DEFAULT_MAX_BYTES).await,
            Err(FetchError::Local(_))
        ));
    }

    #[tokio::test]
    async fn local_source_missing_or_relative_path_rejected() {
        let (_d, s) = store();
        assert!(matches!(
            fetch_source(&local(""), "o", &s, &no_dl(), &MockGitFetcher::never(), &no_resolver(), DEFAULT_MAX_BYTES).await,
            Err(FetchError::MissingField("url"))
        ));
        assert!(matches!(
            fetch_source(&local("relative/path"), "o", &s, &no_dl(), &MockGitFetcher::never(), &no_resolver(), DEFAULT_MAX_BYTES).await,
            Err(FetchError::Local(_))
        ));
    }

    #[tokio::test]
    async fn non_https_git_url_is_rejected() {
        let (_d, s) = store();
        for url in ["git://evil/repo", "ssh://host/repo", "file:///etc"] {
            let src = git(Some(url), Some(&"a".repeat(40)));
            assert!(
                matches!(
                    fetch_source(&src, "o", &s, &no_dl(), &MockGitFetcher::never(), &no_resolver(), DEFAULT_MAX_BYTES)
                        .await,
                    Err(FetchError::Network(_))
                ),
                "non-https git url `{url}` must be rejected"
            );
        }
    }

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(dir.path()).unwrap();
        (dir, s)
    }

    fn tarball(url: &str, sha256: Option<&str>) -> Source {
        Source {
            source_type: SourceType::Tarball,
            url: Some(url.to_string()),
            commit: None,
            sha256: sha256.map(|s| s.to_string()),
            asset: None,
            tag: None,
            version: None,
            patches: Vec::new(),
        }
    }

    fn git(url: Option<&str>, commit: Option<&str>) -> Source {
        Source {
            source_type: SourceType::Git,
            url: url.map(|s| s.to_string()),
            commit: commit.map(|s| s.to_string()),
            sha256: None,
            asset: None,
            tag: None,
            version: None,
            patches: Vec::new(),
        }
    }

    fn crate_src(name: &str, version: Option<&str>, sha256: Option<&str>) -> Source {
        Source {
            source_type: SourceType::Crate,
            url: Some(name.to_string()),
            commit: None,
            sha256: sha256.map(|s| s.to_string()),
            asset: None,
            tag: None,
            version: version.map(|s| s.to_string()),
            patches: Vec::new(),
        }
    }

    #[tokio::test]
    async fn matching_crate_is_downloaded_and_rooted() {
        let (_d, s) = store();
        let body = b"the .crate tarball bytes";
        let sha = ContentHash::of(body);
        let src = crate_src("serde", Some("1.0.0"), Some(sha.as_str()));
        let dl = MockDownloader::ok(body);
        let h = fetch_source(
            &src,
            "org.example.app",
            &s,
            &dl,
            &MockGitFetcher::never(),
            &no_resolver(),
            DEFAULT_MAX_BYTES,
        )
        .await
        .unwrap();
        assert_eq!(h, sha);
        assert_eq!(s.read(&h).unwrap(), body);
    }

    #[tokio::test]
    async fn crate_without_a_version_is_rejected() {
        let (_d, s) = store();
        let src = crate_src("serde", None, Some(ContentHash::of(b"x").as_str()));
        let err = fetch_source(&src, "o", &s, &MockDownloader::ok(b"x"), &MockGitFetcher::never(), &no_resolver(), DEFAULT_MAX_BYTES)
            .await
            .unwrap_err();
        assert!(matches!(err, FetchError::MissingField("version")), "got {err:?}");
    }

    #[tokio::test]
    async fn crate_name_or_version_outside_the_charset_is_refused() {
        let (_d, s) = store();
        let sha = ContentHash::of(b"x");
        // A crafted name that would inject extra URL path segments.
        for src in [
            crate_src("../evil", Some("1.0.0"), Some(sha.as_str())),
            crate_src("serde", Some("1.0/../../etc"), Some(sha.as_str())),
        ] {
            let err = fetch_source(&src, "o", &s, &MockDownloader::ok(b"x"), &MockGitFetcher::never(), &no_resolver(), DEFAULT_MAX_BYTES)
                .await
                .unwrap_err();
            assert!(matches!(err, FetchError::InvalidCrate(_)), "got {err:?}");
        }
    }

    /// A full 40-hex git object id, used as the pinned commit in git tests.
    const COMMIT: &str = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

    #[tokio::test]
    async fn matching_tarball_is_stored_and_rooted() {
        let (_d, s) = store();
        let body = b"the source tree";
        let sha = ContentHash::of(body);
        let src = tarball("https://example.org/x.tar.gz", Some(sha.as_str()));
        let dl = MockDownloader::ok(body);
        let h = fetch_source(
            &src,
            "org.example.app",
            &s,
            &dl,
            &MockGitFetcher::never(), &no_resolver(), DEFAULT_MAX_BYTES,
        )
        .await
        .unwrap();
        assert_eq!(h, sha);
        assert_eq!(s.refcount(&h).unwrap(), 1);
        assert_eq!(s.read(&h).unwrap(), body);
    }

    #[tokio::test]
    async fn sha256_mismatch_stores_nothing() {
        let (_d, s) = store();
        let declared = ContentHash::of(b"what the recipe pinned");
        let src = tarball("https://example.org/x.tar.gz", Some(declared.as_str()));
        let dl = MockDownloader::ok(b"but the server served this");
        let err = fetch_source(
            &src,
            "org.example.app",
            &s,
            &dl,
            &MockGitFetcher::never(), &no_resolver(), DEFAULT_MAX_BYTES,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, FetchError::Store(StoreError::Mismatch { .. })));
        assert!(!s.has(&declared));
        assert!(!s.has(&ContentHash::of(b"but the server served this")));
    }

    #[tokio::test]
    async fn missing_sha256_is_rejected() {
        let (_d, s) = store();
        let src = tarball("https://example.org/x.tar.gz", None);
        let dl = MockDownloader::ok(b"x");
        assert!(matches!(
            fetch_source(
                &src,
                "o",
                &s,
                &dl,
                &MockGitFetcher::never(), &no_resolver(), DEFAULT_MAX_BYTES
            )
            .await,
            Err(FetchError::MissingField("sha256"))
        ));
    }


    #[tokio::test]
    async fn download_error_propagates_and_stores_nothing() {
        let (_d, s) = store();
        let sha = ContentHash::of(b"never arrives");
        let src = tarball("https://example.org/x.tar.gz", Some(sha.as_str()));
        let dl = MockDownloader::err(FetchError::TooLarge { limit: 10 });
        assert!(matches!(
            fetch_source(
                &src,
                "o",
                &s,
                &dl,
                &MockGitFetcher::never(), &no_resolver(), DEFAULT_MAX_BYTES
            )
            .await,
            Err(FetchError::TooLarge { limit: 10 })
        ));
        assert!(!s.has(&sha));
    }

    #[tokio::test]
    async fn git_source_is_stored_and_rooted() {
        let (_d, s) = store();
        let archive = b"deterministic git archive tar bytes";
        let src = git(Some("https://example.org/repo.git"), Some(COMMIT));
        let gf = MockGitFetcher::ok(archive);
        let h = fetch_source(
            &src,
            "org.example.app",
            &s,
            &no_dl(),
            &gf, &no_resolver(), DEFAULT_MAX_BYTES,
        )
        .await
        .unwrap();
        // The commit is the integrity pin: the stored object is addressed by
        // the archive bytes' own content hash.
        assert_eq!(h, ContentHash::of(archive));
        assert_eq!(s.refcount(&h).unwrap(), 1);
        assert_eq!(s.read(&h).unwrap(), archive);
        // Rooted atomically, so a gc cannot collect it.
        assert_eq!(s.gc().unwrap().removed, vec![]);
    }

    #[tokio::test]
    async fn git_source_missing_url_is_rejected() {
        let (_d, s) = store();
        let src = git(None, Some(COMMIT));
        assert!(matches!(
            fetch_source(
                &src,
                "o",
                &s,
                &no_dl(),
                &MockGitFetcher::never(), &no_resolver(), DEFAULT_MAX_BYTES
            )
            .await,
            Err(FetchError::MissingField("url"))
        ));
    }

    #[tokio::test]
    async fn git_source_missing_commit_is_rejected() {
        let (_d, s) = store();
        let src = git(Some("https://example.org/repo.git"), None);
        assert!(matches!(
            fetch_source(
                &src,
                "o",
                &s,
                &no_dl(),
                &MockGitFetcher::never(), &no_resolver(), DEFAULT_MAX_BYTES
            )
            .await,
            Err(FetchError::MissingField("commit"))
        ));
    }

    #[tokio::test]
    async fn git_fetcher_error_propagates_and_stores_nothing() {
        let (_d, s) = store();
        let src = git(Some("https://example.org/repo.git"), Some(COMMIT));
        let gf = MockGitFetcher::err(FetchError::Git("checkout mismatch".into()));
        let err = fetch_source(&src, "o", &s, &no_dl(), &gf, &no_resolver(), DEFAULT_MAX_BYTES)
            .await
            .unwrap_err();
        assert!(matches!(err, FetchError::Git(_)), "got {err:?}");
        // No object made it into the store.
        assert_eq!(s.gc().unwrap().removed, vec![]);
    }

    #[tokio::test]
    async fn clone_recipe_repo_rejects_non_https_transports() {
        let dest = tempfile::tempdir().unwrap();
        // Every non-https transport is refused before any git process spawns, so
        // the SSRF guard cannot be sidestepped via git://, ssh://, or file://.
        for url in [
            "http://example.org/r",
            "git://example.org/r",
            "ssh://example.org/r",
            "file:///etc",
        ] {
            let err = clone_recipe_repo(url, None, dest.path(), DEFAULT_RECIPE_REPO_BYTES)
                .await
                .unwrap_err();
            assert!(
                matches!(err, FetchError::Network(_)),
                "`{url}` must be refused as non-https, got {err:?}"
            );
        }
    }

    #[test]
    fn dir_size_sums_regular_files_only() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a"), b"12345").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/b"), b"678").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("a", dir.path().join("link")).unwrap();
        // 5 + 3 bytes; the symlink contributes nothing.
        assert_eq!(dir_size(dir.path()).unwrap(), 8);
    }
}
