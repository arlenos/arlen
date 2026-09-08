//! Find a manual page by name, using the capability that did not exist.
//!
//! **This module is the test of a ruling.** On 8 September the first module's
//! report said `man` and `dict` could not be modules at all: the host imports
//! were graph, network, events and log, with no filesystem read, so a module
//! whose whole job is reading `/usr/share/man` had nothing to read it with. The
//! answer was to build the capability rather than to narrow the idea, and this
//! is what checks that the answer works - a module that does nothing but read
//! files, declares exactly the directory it reads, and is refused everything
//! else by a deny list it cannot name.
//!
//! It is also the first module to ship its own catalogue. The page names are
//! data and stay as they are; the one sentence this module writes itself - which
//! section a page is in - is a key the host resolves in the reader's language.

wit_bindgen::generate!({
    path: "../../sdk/module-sdk/wit",
    world: "waypointer-provider",
    generate_all,
});

use exports::arlen::waypointer::provider::{Action, Guest, SearchResult};

struct Man;

/// Where the pages are. Declared in the manifest as well, and the two must agree
/// - the manifest is what the host enforces, this is where the module looks.
const ROOT: &str = "/usr/share/man";

/// The sections worth offering. `man1` is commands, `man5` file formats, `man8`
/// administration; the rest are library and kernel interfaces that a launcher
/// search would bury the useful answers under.
const SECTIONS: [&str; 3] = ["man1", "man5", "man8"];

/// The most results a search returns, matching the manifest.
const MAX_RESULTS: usize = 20;

/// One page: its name, the same lowercased, and the section it came from.
///
/// The lowercase copy is built here rather than per query for the reason the
/// first module learned the hard way: a `to_lowercase()` per page per keystroke
/// is an allocation per page per keystroke, and it spent the whole 1 M search
/// budget before reaching the answer. `init` has its own budget for exactly this.
static PAGES: std::sync::OnceLock<Vec<(String, String, String)>> = std::sync::OnceLock::new();

impl Guest for Man {
    fn init() -> Result<(), String> {
        // Built once, for the reason `init` has its own fuel budget: a directory
        // listing per keystroke would be a host call per letter typed, and the
        // pages do not change while the session runs.
        let mut pages = Vec::new();
        for section in SECTIONS {
            let dir = format!("{ROOT}/{section}");
            let Ok(entries) = arlen::host::files::list_dir(&dir) else {
                // A section this machine does not ship is not an error: a
                // container image with no man8 is a normal machine, and the
                // other sections still answer.
                continue;
            };
            for entry in entries {
                if entry.directory {
                    continue;
                }
                // `ls.1.gz` is the page `ls` in section 1. Everything after the
                // first dot is machinery.
                let Some((name, _)) = entry.name.split_once('.') else { continue };
                if name.is_empty() {
                    continue;
                }
                pages.push((
                    name.to_string(),
                    name.to_lowercase(),
                    section.trim_start_matches("man").to_string(),
                ));
            }
        }
        // Sorted by the LOWERCASE name, because that is the key `search` binary
        // searches on. Sorting by the display name would put `Mail` before `ls`
        // and quietly break the range.
        pages.sort_by(|a, b| a.1.cmp(&b.1).then(a.2.cmp(&b.2)));
        pages.dedup();
        PAGES.set(pages).map_err(|_| "the index was built twice".to_string())
    }

    fn search(query: String) -> Vec<SearchResult> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let Some(pages) = PAGES.get() else { return Vec::new() };

        // A prefix range, found by binary search.
        //
        // **Measured rather than chosen for elegance.** Three shapes did not fit
        // the 1 M per-keystroke budget: cutting at twenty before ranking (which
        // also drops the best answer), building a result per match and ranking
        // after, and even a cheap scoring pass over all 4600 names - a
        // `contains` call pays a substring-searcher setup that dwarfs the twenty
        // bytes it looks at, which is the same wall the first module hit at forty
        // thousand names.
        //
        // So this searches by PREFIX, which is what a launcher query is: someone
        // typing `ls` wants `ls`, not `tools`. The index is sorted in `init`, so
        // finding the range is two binary searches and the work is proportional
        // to the answer rather than to the corpus.
        let lo = pages.partition_point(|(_, lower, _)| lower.as_str() < q.as_str());
        let mut out = Vec::new();
        for (name, lower, section) in &pages[lo..] {
            if !lower.starts_with(&q) {
                break;
            }
            if out.len() >= MAX_RESULTS {
                break;
            }
            out.push(SearchResult {
                id: format!("man-{section}-{name}"),
                // A page name is data, not prose, so it stays as it is in every
                // language and carries no key.
                title: name.clone(),
                title_key: None,
                // The one sentence this module writes itself. The literal is the
                // English and is what a host with no catalogue shows; the key is
                // what a German reader gets. The host resolves it, so this
                // module never learns which language that was.
                description: Some(format!("Section {section}")),
                description_key: Some("man.section".to_string()),
                // Built by hand rather than with a JSON writer because the value
                // is one of three literals from `SECTIONS`, never anything a
                // person typed. A field that took a page name would need one.
                args: Some(format!("{{\"section\": \"{section}\"}}")),
                icon: None,
                // An exact hit outranks a longer name that merely starts the
                // same, and the range is already alphabetical after that.
                relevance: if *lower == q { 1.0 } else { 0.9 },
                // What a person wants from a launcher hit on a man page is the
                // command to read it, and the host can put that on the clipboard
                // without this module asking for a way to run anything.
                action: Action::Copy(format!("man {section} {name}")),
            });
        }
        out.sort_by(|a, b| b.relevance.total_cmp(&a.relevance));
        out
    }

    fn execute(_hit: SearchResult) -> Result<(), String> {
        // Nothing to do: `copy` is the host's to fulfil.
        Ok(())
    }

    fn shutdown() {}
}

export!(Man);
