// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Read a `.lunpkg`'s package id without installing it.
//!
//! BR-5 installs a foreign app's bridges alongside the app. For a Flatpak the
//! id is in the target the user typed; for a local package it is inside the
//! archive, which is why that half was left out. This reads it, so a local
//! install provisions its bridges like any other.
//!
//! Extraction goes through the hardened `arlen-forage-extract` rather than a
//! second, thinner tar walk: a package is untrusted input at this point (the
//! signature is verified by installd, after this), and having one archive
//! reader means one place where traversal, symlink and bomb handling live. The
//! limits here are far below its defaults because a manifest read has no reason
//! to unpack a large tree.

use std::path::Path;

/// Entry and byte ceilings for reading a manifest. Deliberately small: this is
/// not the install, it is a peek, and a package that needs more than this to
/// state its id is one we should not be reading unprompted.
const MAX_ENTRIES: usize = 10_000;
/// 64 MiB, enough for a real package's file table without unpacking a bomb.
const MAX_BYTES: u64 = 64 * 1024 * 1024;

/// The `[package] id` from a manifest's TOML text, or `None` when the document
/// does not parse or states no id.
///
/// Separate from the file handling so the parse is testable without an archive,
/// and total: a malformed manifest yields no id rather than an error, because
/// every caller's answer to "cannot tell" is the same as to "has none" - skip
/// the bridge step and let the install speak for itself.
pub fn package_id_from_manifest(text: &str) -> Option<String> {
    let doc: toml::Value = toml::from_str(text).ok()?;
    let id = doc.get("package")?.get("id")?.as_str()?.trim();
    (!id.is_empty()).then(|| id.to_string())
}

/// The package id inside a `.lunpkg`, or `None` if it cannot be read.
///
/// Never fatal by design. The bridge step it feeds is a convenience on top of
/// an install that has already succeeded, so a package this cannot read gets
/// installed and simply provisions no bridges.
pub fn package_id(archive: &Path) -> Option<String> {
    let bytes = std::fs::read(archive).ok()?;
    let temp = tempfile::TempDir::new().ok()?;
    let limits = arlen_forage_extract::ExtractLimits {
        max_entries: MAX_ENTRIES,
        max_total_bytes: MAX_BYTES,
    };
    arlen_forage_extract::extract_tar(&bytes, temp.path(), &limits).ok()?;
    let manifest = std::fs::read_to_string(temp.path().join("manifest.toml")).ok()?;
    package_id_from_manifest(&manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manifest_yields_its_package_id() {
        let id = package_id_from_manifest(
            r#"
[package]
id = "com.example.notes"
name = "Notes"
version = "1.2.0"
"#,
        );
        assert_eq!(id.as_deref(), Some("com.example.notes"));
    }

    /// Every "cannot tell" case answers the same way, because the caller does
    /// the same thing with all of them: skip the bridge step.
    #[test]
    fn anything_that_does_not_state_an_id_yields_none() {
        for text in [
            "",                                   // empty
            "this is not toml {{{",               // malformed
            "[package]\nname = \"Notes\"",        // no id
            "[package]\nid = \"\"",               // empty id
            "[package]\nid = \"   \"",            // blank id
            "[package]\nid = 7",                  // wrong type
            "[other]\nid = \"com.example.notes\"", // wrong table
        ] {
            assert_eq!(package_id_from_manifest(text), None, "for {text:?}");
        }
    }

    #[test]
    fn an_id_is_trimmed_rather_than_carried_with_whitespace() {
        // It becomes a lookup key for the bridge search, so a stray space would
        // silently match nothing.
        let id = package_id_from_manifest("[package]\nid = \"  com.example.notes  \"");
        assert_eq!(id.as_deref(), Some("com.example.notes"));
    }

    #[test]
    fn a_missing_archive_reads_as_no_id_rather_than_panicking() {
        assert_eq!(package_id(Path::new("/nonexistent-package.lunpkg")), None);
    }
}
