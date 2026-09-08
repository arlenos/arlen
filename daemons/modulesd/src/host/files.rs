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
