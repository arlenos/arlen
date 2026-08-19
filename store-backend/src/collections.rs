// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Editorial collections: the hand-picked groupings the store's landing view
//! shows before anyone types.
//!
//! `store-app.md` section 8.7 settles the discovery model as **editorial
//! collections, hand-picked and human-curated** rather than algorithmic. The
//! word that decides where they live is *curated*: they are DATA somebody
//! maintains, not a constant compiled into a page. Held as a constant they
//! cannot change without a rebuild, and - the failure this module was written
//! after - they can name apps that no catalog on any machine contains, which is
//! how the store came to render an empty landing view over a live catalog of 43
//! apps.
//!
//! TITLES ARE THE ONE EXCEPTION to "identifiers, never prose". Everything else
//! crossing this wire is an identifier the app renders in its own language,
//! because a Rust backend writing "Cannot reach the network" ships English into
//! a German build. A collection cannot work that way: its name is the curator's,
//! and a curator who cannot add one without an app release is not curating. So a
//! collection carries its titles PER LOCALE, supplied by whoever wrote the file,
//! and the app picks the one matching its own locale. This is what AppStream
//! itself does with `<name xml:lang="de">`, and it keeps the project's own words
//! out of Rust while leaving the shape open.
//!
//! The honesty rule is [`resolve`]: members are intersected with the catalog the
//! machine actually has, and a collection left with nothing is DROPPED rather
//! than rendered as a heading over empty space. A heading promising apps that
//! are not there is the same defect as the empty landing view, one size smaller.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::catalog::ComponentId;

/// The locale a collection must always carry, and the fallback when the app's
/// own locale is missing. Something has to be renderable or the collection is
/// unshowable, and the tree's source language is the only honest choice.
pub const FALLBACK_LOCALE: &str = "en";

/// One editorial collection: a curator's grouping of component-ids under a name
/// they wrote themselves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Collection {
    /// A stable identifier for the grouping. Never displayed: it is what a
    /// caller uses to remember a collection across a re-ordering of the file.
    pub id: String,
    /// The curator's own title per locale, always including [`FALLBACK_LOCALE`].
    pub titles: BTreeMap<String, String>,
    /// The component-ids in this collection, in the curator's order. Order is
    /// editorial too, so it is preserved rather than sorted.
    pub members: Vec<ComponentId>,
}

impl Collection {
    /// The title for a locale, falling back to [`FALLBACK_LOCALE`]. A locale
    /// like `de-AT` falls back to `de` before falling back to English, so a
    /// regional variant does not silently land in the wrong language.
    #[must_use]
    pub fn title(&self, locale: &str) -> &str {
        if let Some(t) = self.titles.get(locale) {
            return t;
        }
        if let Some((base, _)) = locale.split_once('-') {
            if let Some(t) = self.titles.get(base) {
                return t;
            }
        }
        self.titles
            .get(FALLBACK_LOCALE)
            .map_or("", String::as_str)
    }
}

/// Why a collections file was refused.
///
/// Refused, not repaired. A collections file is small and hand-written, so a
/// mistake in it is a mistake somebody can fix; guessing past one would ship a
/// landing view that quietly differs from what the curator wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectionError {
    /// The file is not valid TOML, or does not have the expected shape.
    Malformed(String),
    /// A collection has an empty id.
    EmptyId,
    /// Two collections share an id.
    DuplicateId(String),
    /// A collection carries no title in [`FALLBACK_LOCALE`], so there is no
    /// language it can always be rendered in.
    NoFallbackTitle(String),
    /// A collection names no members at all, which cannot be an editorial act.
    NoMembers(String),
}

impl std::fmt::Display for CollectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(e) => write!(f, "collections file is malformed: {e}"),
            Self::EmptyId => write!(f, "a collection has an empty id"),
            Self::DuplicateId(id) => write!(f, "two collections share the id {id}"),
            Self::NoFallbackTitle(id) => {
                write!(f, "collection {id} has no `{FALLBACK_LOCALE}` title")
            }
            Self::NoMembers(id) => write!(f, "collection {id} names no members"),
        }
    }
}

impl std::error::Error for CollectionError {}

/// The on-disk shape: a list of `[[collection]]` tables.
#[derive(Debug, Deserialize)]
struct CollectionsFile {
    #[serde(default)]
    collection: Vec<RawCollection>,
}

#[derive(Debug, Deserialize)]
struct RawCollection {
    id: String,
    #[serde(default)]
    titles: BTreeMap<String, String>,
    #[serde(default)]
    members: Vec<String>,
}

/// Parse a collections file, refusing anything that could not be rendered
/// honestly.
///
/// # Errors
///
/// Returns [`CollectionError`] for malformed TOML, an empty or duplicated id, a
/// collection with no fallback-locale title, or one naming no members.
pub fn parse_collections(text: &str) -> Result<Vec<Collection>, CollectionError> {
    let parsed: CollectionsFile =
        toml::from_str(text).map_err(|e| CollectionError::Malformed(e.to_string()))?;
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(parsed.collection.len());
    for raw in parsed.collection {
        if raw.id.trim().is_empty() {
            return Err(CollectionError::EmptyId);
        }
        if !seen.insert(raw.id.clone()) {
            return Err(CollectionError::DuplicateId(raw.id));
        }
        if !raw.titles.contains_key(FALLBACK_LOCALE) {
            return Err(CollectionError::NoFallbackTitle(raw.id));
        }
        if raw.members.is_empty() {
            return Err(CollectionError::NoMembers(raw.id));
        }
        out.push(Collection {
            id: raw.id,
            titles: raw.titles,
            members: raw.members.into_iter().map(ComponentId).collect(),
        });
    }
    Ok(out)
}

/// Narrow collections to what this machine can actually show.
///
/// Each collection keeps only the members `present` reports, in the curator's
/// order, and a collection left empty is dropped. This is the whole point of the
/// module: the store must never head a row with a name and then show nothing
/// under it, and it must never promise an app the catalog does not carry.
#[must_use]
pub fn resolve(collections: &[Collection], present: &dyn Fn(&ComponentId) -> bool) -> Vec<Collection> {
    collections
        .iter()
        .filter_map(|c| {
            let members: Vec<ComponentId> =
                c.members.iter().filter(|id| present(id)).cloned().collect();
            if members.is_empty() {
                return None;
            }
            Some(Collection { id: c.id.clone(), titles: c.titles.clone(), members })
        })
        .collect()
}

/// Where the shipped collections file lives.
///
/// `ARLEN_STORE_COLLECTIONS` overrides it, which is how the tests and the drive
/// script point at their own file without installing one.
#[must_use]
pub fn collections_path() -> PathBuf {
    std::env::var_os("ARLEN_STORE_COLLECTIONS")
        .map_or_else(|| PathBuf::from("/usr/share/arlen/store/collections.toml"), PathBuf::from)
}

/// Read the shipped collections.
///
/// An absent file is `Ok` with nothing in it: a machine that ships no curated
/// list is a real state, and the landing view then falls back to the catalog
/// itself rather than to a promise. A file that is PRESENT but broken is an
/// error the caller reports, because half a curated list is not a curated list
/// and swallowing the reason leaves an image bug with nowhere to surface.
///
/// # Errors
///
/// Returns [`CollectionError`] when the file exists and does not parse.
pub fn load_collections() -> Result<Vec<Collection>, CollectionError> {
    let Ok(text) = std::fs::read_to_string(collections_path()) else {
        return Ok(Vec::new());
    };
    parse_collections(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[[collection]]
id = "essentials"
titles.en = "Essentials"
titles.de = "Grundausstattung"
members = ["org.arlen.Files", "org.arlen.TextEditor"]

[[collection]]
id = "creative"
titles.en = "Make something"
members = ["org.inkscape.Inkscape"]
"#;

    #[test]
    fn parses_titles_members_and_order() {
        let c = parse_collections(SAMPLE).expect("sample parses");
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].id, "essentials");
        assert_eq!(c[0].title("de"), "Grundausstattung");
        assert_eq!(c[0].members[0].0, "org.arlen.Files");
        // Curator order, not alphabetical: the ordering is editorial.
        assert_eq!(c[1].id, "creative");
    }

    #[test]
    fn a_regional_locale_falls_back_to_its_base_before_english() {
        let c = parse_collections(SAMPLE).expect("sample parses");
        assert_eq!(c[0].title("de-AT"), "Grundausstattung");
        assert_eq!(c[0].title("fr"), "Essentials");
        // A collection with no German title says the English one rather than
        // nothing at all.
        assert_eq!(c[1].title("de"), "Make something");
    }

    #[test]
    fn refuses_a_file_that_could_not_be_rendered_honestly() {
        assert!(matches!(parse_collections("[[collection]]\nid = \"\"\ntitles.en = \"x\"\nmembers = [\"a\"]"),
            Err(CollectionError::EmptyId)));
        assert!(matches!(
            parse_collections("[[collection]]\nid = \"a\"\ntitles.de = \"x\"\nmembers = [\"m\"]"),
            Err(CollectionError::NoFallbackTitle(_))
        ));
        assert!(matches!(
            parse_collections("[[collection]]\nid = \"a\"\ntitles.en = \"x\"\nmembers = []"),
            Err(CollectionError::NoMembers(_))
        ));
        let dup = "[[collection]]\nid = \"a\"\ntitles.en = \"x\"\nmembers = [\"m\"]\n\
                   [[collection]]\nid = \"a\"\ntitles.en = \"y\"\nmembers = [\"n\"]";
        assert!(matches!(parse_collections(dup), Err(CollectionError::DuplicateId(_))));
        assert!(matches!(parse_collections("not toml ["), Err(CollectionError::Malformed(_))));
    }

    #[test]
    fn resolve_drops_members_and_collections_the_catalog_does_not_have() {
        let c = parse_collections(SAMPLE).expect("sample parses");
        // Only one of the two essentials is installed-able here, and nothing
        // from the creative collection is.
        let present = |id: &ComponentId| id.0 == "org.arlen.TextEditor";
        let got = resolve(&c, &present);
        assert_eq!(got.len(), 1, "a collection with no present member is dropped");
        assert_eq!(got[0].id, "essentials");
        assert_eq!(got[0].members.len(), 1);
        assert_eq!(got[0].members[0].0, "org.arlen.TextEditor");
    }

    #[test]
    fn resolve_against_an_empty_catalog_shows_nothing_rather_than_headings() {
        let c = parse_collections(SAMPLE).expect("sample parses");
        assert!(resolve(&c, &|_| false).is_empty());
    }
}
