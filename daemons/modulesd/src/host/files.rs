//! Which file a module may read, and the deny list that makes the answer safe.
//!
//! `files.read` is the capability `module-system.md` has listed since the system
//! was designed and the WIT never grew, so a module that wants to read a man page
//! or a dictionary has had no way to. This module is the decision half: given a
//! path a guest asked for, the prefixes its manifest declared, and the paths no
//! manifest may name, it says allow or refuse and why.
//!
//! **THE DENY LIST IS THE LOAD-BEARING HALF, not a belt over the allowlist.**
//! Without it `files.read` is a way around `graph.read`: a module denied graph
//! access declares `files.read = ["/home/me"]`, opens the knowledge graph's
//! SQLite file and reads the whole graph. The standing invariant is that the KG
//! is never silently bulk-readable, and a prefix allowlist cannot give you that
//! because the person granting it is agreeing to a directory, not auditing what
//! lives under it.
//!
//! **Order is part of the rule.** Canonicalise first, with symlinks resolved, so
//! a link inside an allowed root cannot point out of it and a link outside cannot
//! point in. Then deny. Then allow. Any other order is a hole: checking the
//! allowlist against the path as written lets `/usr/share/man/../../../etc/shadow`
//! through, and checking deny before canonicalising lets a symlink walk around it.

use std::path::{Component, Path, PathBuf};

/// What the host may do with a path a guest asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Read it, at this canonical path rather than the one asked for.
    Allow(PathBuf),
    /// Refused by the deny list, which no manifest can widen.
    Denied {
        /// The deny root it fell under, for the log rather than the guest.
        under: PathBuf,
    },
    /// Not under any prefix the module declared.
    Undeclared,
    /// The path does not resolve: absent, or a dangling link.
    Unresolvable,
}

/// Whether `path` sits at or under `root`, by path component.
///
/// Not a string prefix: `/usr/share/manx` starts with `/usr/share/man` as text
/// and is a different directory. Every check here goes through this.
pub fn under(path: &Path, root: &Path) -> bool {
    let mut p = path.components();
    for c in root.components() {
        match p.next() {
            Some(x) if x == c => {}
            _ => return false,
        }
    }
    true
}

/// Decide what may be read, canonicalising through `resolve`.
///
/// `resolve` is injected so the decision is testable without a filesystem: the
/// host passes `std::fs::canonicalize`, which resolves symlinks and `..` and
/// fails for a path that does not exist.
pub fn decide<F>(
    requested: &Path,
    declared: &[PathBuf],
    deny: &[PathBuf],
    resolve: F,
) -> Verdict
where
    F: Fn(&Path) -> std::io::Result<PathBuf>,
{
    // A relative path has no meaning here: the guest has no working directory of
    // its own, so accepting one would resolve against the daemon's.
    if requested.components().next() != Some(Component::RootDir) {
        return Verdict::Undeclared;
    }
    let Ok(canonical) = resolve(requested) else {
        return Verdict::Unresolvable;
    };
    if let Some(root) = deny.iter().find(|d| under(&canonical, d)) {
        return Verdict::Denied { under: root.clone() };
    }
    if declared.iter().any(|d| under(&canonical, d)) {
        return Verdict::Allow(canonical);
    }
    Verdict::Undeclared
}

/// The paths no module may read, whatever its manifest says.
///
/// Built from the same environment the daemons use, so a test home or a real one
/// both produce the right answer. Each entry is here for a reason a reader can
/// check:
///
///   * the knowledge graph's store and its socket directory - the invariant this
///     whole list exists for;
///   * `~/.config/arlen` and `~/.local/share/arlen` - every daemon's config and
///     state, including permission profiles, which is where a module would look
///     to find out what it could get away with;
///   * `~/.timeline` - the FUSE view of the graph, which is the graph;
///   * the keyring and secrets directory;
///   * the modules directory itself - a module may not read its neighbours, or
///     its own manifest, to discover what else is installed.
pub fn deny_roots() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let dir = |var: &str, fallback: &str| {
        std::env::var_os(var)
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|h| h.join(fallback)))
    };
    deny_roots_from(
        home.clone(),
        dir("XDG_CONFIG_HOME", ".config"),
        dir("XDG_DATA_HOME", ".local/share"),
        dir("XDG_STATE_HOME", ".local/state"),
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
    )
}

/// The same list over bases handed in rather than read.
///
/// Split out because the alternative is a test that calls `set_var`, and this
/// crate's own suite already records what that costs: the variable is
/// process-wide, so one test setting `HOME` sends every other test in the binary
/// to a directory it cannot write. Two manager tests failed that way within a
/// minute of the first version of this file.
pub fn deny_roots_from(
    home: Option<PathBuf>,
    config: Option<PathBuf>,
    data: Option<PathBuf>,
    state: Option<PathBuf>,
    runtime: Option<PathBuf>,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for (base, leaf) in [
        (config.clone(), "arlen"),
        (data.clone(), "arlen"),
        (state, "arlen"),
        (runtime, "arlen"),
        (config.clone(), "permissions"),
        (data, "keyrings"),
        (config, "gnupg"),
    ] {
        if let Some(b) = base {
            roots.push(b.join(leaf));
        }
    }
    if let Some(h) = home {
        roots.push(h.join(".timeline"));
        roots.push(h.join(".ssh"));
        roots.push(h.join(".gnupg"));
    }
    // The system-wide halves of the same things.
    roots.push(PathBuf::from("/var/lib/arlen"));
    roots.push(PathBuf::from("/run/arlen"));
    roots.push(PathBuf::from("/usr/share/arlen/modules"));
    roots
}

/// The most a guest gets from one `read`.
///
/// A module asked for a file, not for the host's memory: the store is capped at
/// 64 MB and a guest that asks for something larger would take the daemon's
/// allocation with it. Eight megabytes covers what this capability is for - a
/// man page is kilobytes, `/usr/share/dict/words` is about one megabyte - and
/// anything past it is told `too-large` rather than being cut off silently.
pub const MAX_READ_BYTES: u64 = 8 * 1024 * 1024;

/// What went wrong, in the vocabulary the WIT enum uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Under something no manifest may name.
    Denied,
    /// Outside every prefix this module declared.
    NotAllowed,
    /// Absent, or a link that goes nowhere.
    NotFound,
    /// There and unreadable.
    Unreadable,
    /// Larger than [`MAX_READ_BYTES`].
    TooLarge,
}

/// One directory entry, as the guest sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub directory: bool,
    pub size: u64,
}

fn declared(ctx: &CapabilityContext) -> Vec<PathBuf> {
    ctx.capabilities
        .files
        .as_ref()
        .map(|f| f.read.iter().map(PathBuf::from).collect())
        .unwrap_or_default()
}

fn resolved(ctx: &CapabilityContext, path: &str) -> Result<PathBuf, Refusal> {
    match decide(
        Path::new(path),
        &declared(ctx),
        &deny_roots(),
        |p| std::fs::canonicalize(p),
    ) {
        Verdict::Allow(p) => Ok(p),
        Verdict::Denied { under } => {
            tracing::warn!(
                module = %ctx.module_id,
                denied_under = %under.display(),
                "files.read refused: the path is on the deny list"
            );
            Err(Refusal::Denied)
        }
        Verdict::Undeclared => Err(Refusal::NotAllowed),
        Verdict::Unresolvable => Err(Refusal::NotFound),
    }
}

/// Read a whole file, if this module may.
pub fn read(ctx: &CapabilityContext, path: &str) -> Result<Vec<u8>, Refusal> {
    let path = resolved(ctx, path)?;
    let meta = std::fs::metadata(&path).map_err(|_| Refusal::Unreadable)?;
    if meta.is_dir() {
        // A directory is not a file, and saying `unreadable` here is more honest
        // than handing back whatever the platform does with `read_to_end` on one.
        return Err(Refusal::Unreadable);
    }
    if meta.len() > MAX_READ_BYTES {
        return Err(Refusal::TooLarge);
    }
    std::fs::read(&path).map_err(|_| Refusal::Unreadable)
}

/// List a directory, if this module may.
pub fn list_dir(ctx: &CapabilityContext, path: &str) -> Result<Vec<Entry>, Refusal> {
    let path = resolved(ctx, path)?;
    let dir = std::fs::read_dir(&path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => Refusal::NotFound,
        _ => Refusal::Unreadable,
    })?;
    let mut out = Vec::new();
    for entry in dir.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        out.push(Entry {
            name: entry.file_name().to_string_lossy().into_owned(),
            directory: meta.is_dir(),
            size: if meta.is_dir() { 0 } else { meta.len() },
        });
    }
    // Sorted, because a directory's order is the filesystem's and a guest that
    // renders a listing should not change between calls for no reason.
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

use crate::host::context::CapabilityContext;

#[cfg(test)]
mod tests {
    use super::*;

    /// A resolver that pretends `link` points at `target` and everything else
    /// resolves to itself, so the ORDER of the checks can be tested without a
    /// filesystem.
    fn resolver(link: &'static str, target: &'static str) -> impl Fn(&Path) -> std::io::Result<PathBuf> {
        move |p: &Path| {
            if p == Path::new(link) {
                Ok(PathBuf::from(target))
            } else if p == Path::new("/nowhere") {
                Err(std::io::Error::from(std::io::ErrorKind::NotFound))
            } else {
                Ok(p.to_path_buf())
            }
        }
    }

    fn declared() -> Vec<PathBuf> {
        vec![PathBuf::from("/usr/share/man")]
    }

    fn deny() -> Vec<PathBuf> {
        vec![PathBuf::from("/home/u/.config/arlen")]
    }

    #[test]
    fn a_declared_prefix_is_read_at_its_canonical_path() {
        let v = decide(
            Path::new("/usr/share/man/man1/ls.1"),
            &declared(),
            &deny(),
            resolver("", ""),
        );
        assert_eq!(v, Verdict::Allow(PathBuf::from("/usr/share/man/man1/ls.1")));
    }

    #[test]
    fn a_neighbouring_directory_is_not_the_declared_one() {
        // `/usr/share/manx` starts with `/usr/share/man` as a string. This is why
        // the check is by component.
        let v = decide(
            Path::new("/usr/share/manx/secret"),
            &declared(),
            &deny(),
            resolver("", ""),
        );
        assert_eq!(v, Verdict::Undeclared);
    }

    #[test]
    fn a_link_inside_an_allowed_root_cannot_point_out_of_it() {
        let v = decide(
            Path::new("/usr/share/man/escape"),
            &declared(),
            &deny(),
            resolver("/usr/share/man/escape", "/etc/shadow"),
        );
        assert_eq!(v, Verdict::Undeclared, "resolved out of the root, so not declared");
    }

    #[test]
    fn a_link_inside_an_allowed_root_cannot_reach_the_deny_list() {
        // The case the order exists for: declared prefix, denied target.
        let v = decide(
            Path::new("/usr/share/man/graph"),
            &declared(),
            &deny(),
            resolver("/usr/share/man/graph", "/home/u/.config/arlen/graph.db"),
        );
        assert_eq!(v, Verdict::Denied { under: PathBuf::from("/home/u/.config/arlen") });
    }

    #[test]
    fn a_module_cannot_declare_its_way_into_the_deny_list() {
        // The deny list is checked BEFORE the allowlist, so declaring the parent
        // buys nothing.
        let v = decide(
            Path::new("/home/u/.config/arlen/graph.db"),
            &[PathBuf::from("/home/u")],
            &deny(),
            resolver("", ""),
        );
        assert!(matches!(v, Verdict::Denied { .. }));
    }

    #[test]
    fn a_relative_path_is_refused_rather_than_resolved() {
        let v = decide(Path::new("man1/ls.1"), &declared(), &deny(), resolver("", ""));
        assert_eq!(v, Verdict::Undeclared);
    }

    #[test]
    fn a_path_that_does_not_resolve_says_so() {
        let v = decide(Path::new("/nowhere"), &declared(), &deny(), resolver("", ""));
        assert_eq!(v, Verdict::Unresolvable);
    }

    fn ctx_reading(prefixes: &[&std::path::Path]) -> CapabilityContext {
        let caps = arlen_modules::ModuleCapabilities {
            files: Some(arlen_modules::FilesCapability {
                read: prefixes.iter().map(|p| p.display().to_string()).collect(),
            }),
            ..Default::default()
        };
        CapabilityContext::new("com.example.reader", caps)
    }

    #[test]
    fn a_declared_file_reads_and_an_undeclared_sibling_does_not() {
        let tmp = tempfile::tempdir().unwrap();
        let allowed = tmp.path().join("allowed");
        let other = tmp.path().join("other");
        std::fs::create_dir_all(&allowed).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(allowed.join("page"), b"HEAVY BLACK HEART").unwrap();
        std::fs::write(other.join("secret"), b"no").unwrap();

        let ctx = ctx_reading(&[&std::fs::canonicalize(&allowed).unwrap()]);
        assert_eq!(
            read(&ctx, allowed.join("page").to_str().unwrap()),
            Ok(b"HEAVY BLACK HEART".to_vec())
        );
        assert_eq!(
            read(&ctx, other.join("secret").to_str().unwrap()),
            Err(Refusal::NotAllowed)
        );
    }

    #[test]
    fn a_module_that_declared_nothing_reads_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("f"), b"x").unwrap();
        let ctx = CapabilityContext::empty("com.example.quiet");
        assert_eq!(
            read(&ctx, tmp.path().join("f").to_str().unwrap()),
            Err(Refusal::NotAllowed)
        );
    }

    #[test]
    fn a_listing_names_entries_and_sorts_them() {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        std::fs::write(root.join("b"), b"22").unwrap();
        std::fs::write(root.join("a"), b"1").unwrap();
        std::fs::create_dir(root.join("c")).unwrap();

        let ctx = ctx_reading(&[&root]);
        let listed = list_dir(&ctx, root.to_str().unwrap()).expect("listed");
        assert_eq!(
            listed,
            vec![
                Entry { name: "a".into(), directory: false, size: 1 },
                Entry { name: "b".into(), directory: false, size: 2 },
                Entry { name: "c".into(), directory: true, size: 0 },
            ]
        );
    }

    #[test]
    fn a_directory_is_not_a_file() {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let ctx = ctx_reading(&[&root]);
        assert_eq!(read(&ctx, root.to_str().unwrap()), Err(Refusal::Unreadable));
    }

    #[test]
    fn a_link_out_of_the_declared_root_is_refused_by_the_real_resolver() {
        // The same claim as the injected-resolver test, but through
        // `std::fs::canonicalize`, because the guarantee rests on what the real
        // one does with a symlink.
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let inside = root.join("inside");
        let outside = root.join("outside");
        std::fs::create_dir_all(&inside).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret"), b"no").unwrap();
        std::os::unix::fs::symlink(outside.join("secret"), inside.join("escape")).unwrap();

        let ctx = ctx_reading(&[&inside]);
        assert_eq!(
            read(&ctx, inside.join("escape").to_str().unwrap()),
            Err(Refusal::NotAllowed),
            "the link resolves out of the declared root"
        );
    }

    #[test]
    fn the_deny_roots_cover_the_graph_and_the_modules_directory() {
        let roots = deny_roots_from(
            Some(PathBuf::from("/home/u")),
            Some(PathBuf::from("/home/u/.config")),
            Some(PathBuf::from("/home/u/.local/share")),
            Some(PathBuf::from("/home/u/.local/state")),
            None,
        );
        for expected in [
            "/home/u/.config/arlen",
            "/home/u/.local/share/arlen",
            "/home/u/.timeline",
            "/var/lib/arlen",
            "/usr/share/arlen/modules",
        ] {
            assert!(
                roots.iter().any(|r| r == Path::new(expected)),
                "{expected} is not denied: {roots:?}"
            );
        }
    }
}
