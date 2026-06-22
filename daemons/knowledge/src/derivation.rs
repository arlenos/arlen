//! Strong-signal file-to-file derivation detection (KG-richness Thrust 3d,
//! PROV-O `wasDerivedFrom`).
//!
//! DECIDED noise posture (Tim, 22 June): emit a `DERIVED_FROM` edge ONLY on a
//! STRONG signal - same-process AND (a name/extension relation, OR a recognized
//! transform tool) read-then-write. A weak signal (an editor reading many files
//! and writing one) emits NOTHING; the already-built `CO_ACCESSED` / `MODIFIED_BY`
//! capture that weaker relation. A false `DERIVED_FROM` is worse than none.
//!
//! This module is the PURE name/extension-relation predicate - the noise-killing
//! disjunct: the written file is named after the read file (`foo.md` ->
//! `foo.pdf`, `main.rs` -> `main.o`, `foo.tar` -> `foo.tar.gz`, `doc` ->
//! `doc.bak`). It performs no I/O and only answers "is the NAME relation
//! strong". The eventual promotion wiring supplies the other half of the AND -
//! the same-process gate + a temporal window - and stamps the edge with its
//! derivation confidence. (True same-PROCESS needs a `cgroup_id` on the
//! `file.written` event, which the kernel-layer write probe does not emit today,
//! unlike `file.opened`; the wiring uses the writing `app_id` as the proxy until
//! that kernel follow-up lands.) Lives behind `allow(dead_code)` until the
//! wiring consumes it (mechanism before trigger).
#![allow(dead_code)]

use std::path::Path;

/// Whether `written` is, by NAME, a strong derivation of `read` (Thrust 3d's
/// name/extension disjunct). Pure (no filesystem access); the caller still gates
/// on same-process + temporal proximity before emitting an edge.
///
/// Strong iff (and the paths are not identical - a modify, not a derivation):
///  - `written` is `read` plus an appended extension (`foo.tar` -> `foo.tar.gz`,
///    `doc` -> `doc.bak`, `foo.md` -> `foo.md.sig`); OR
///  - they share a basename stem but differ in extension (`foo.md` -> `foo.pdf`,
///    `main.rs` -> `main.o`), directory-independent (`a/foo.md` -> `b/foo.pdf`).
///
/// Returns false for an unrelated name (`foo.md` -> `bar.pdf`, the editor-noise
/// case) and for a same-name copy with no extension change (`a/foo.md` ->
/// `b/foo.md`, ambiguous with a move, not a transform).
pub fn is_name_derivation(read: &str, written: &str) -> bool {
    if read == written {
        return false;
    }
    let (r, w) = (Path::new(read), Path::new(written));
    let (rname, wname) = match (
        r.file_name().and_then(|s| s.to_str()),
        w.file_name().and_then(|s| s.to_str()),
    ) {
        (Some(a), Some(b)) => (a, b),
        _ => return false,
    };

    // (1) The written name is the read name plus an appended `.ext`.
    if let Some(rest) = wname.strip_prefix(rname) {
        if let Some(ext) = rest.strip_prefix('.') {
            if !ext.is_empty() {
                return true;
            }
        }
    }

    // (2) Same basename stem, different extension (the classic transform shape).
    match (
        r.file_stem().and_then(|s| s.to_str()),
        w.file_stem().and_then(|s| s.to_str()),
    ) {
        (Some(rs), Some(ws)) if rs == ws => {
            r.extension().and_then(|s| s.to_str()) != w.extension().and_then(|s| s.to_str())
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_stem_different_extension_is_a_strong_derivation() {
        assert!(is_name_derivation("/p/foo.md", "/p/foo.pdf"));
        assert!(is_name_derivation("/p/main.rs", "/p/main.o"));
        // Directory-independent: a transform may write the output elsewhere.
        assert!(is_name_derivation("/src/foo.md", "/out/foo.pdf"));
    }

    #[test]
    fn an_appended_extension_is_a_strong_derivation() {
        assert!(is_name_derivation("/p/foo.tar", "/p/foo.tar.gz"));
        assert!(is_name_derivation("/p/doc", "/p/doc.bak"));
        assert!(is_name_derivation("/p/foo.md", "/p/foo.md.sig"));
    }

    #[test]
    fn an_unrelated_name_is_not_a_derivation() {
        // The editor-noise case: read one file, write an unrelated one.
        assert!(!is_name_derivation("/p/foo.md", "/p/bar.pdf"));
        assert!(!is_name_derivation("/p/notes.md", "/p/report.md"));
    }

    #[test]
    fn the_same_path_is_not_a_derivation() {
        assert!(!is_name_derivation("/p/foo.md", "/p/foo.md"));
    }

    #[test]
    fn a_same_name_copy_with_no_extension_change_is_not_a_derivation() {
        // Ambiguous with a move/copy, not a transform: conservatively excluded.
        assert!(!is_name_derivation("/a/foo.md", "/b/foo.md"));
    }

    #[test]
    fn an_extensionless_rename_relation_holds_by_stem() {
        // Stripping an extension keeps the stem relation (rare but a real shape).
        assert!(is_name_derivation("/p/foo.md", "/p/foo"));
    }
}
