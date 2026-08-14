// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The searches a person saved.
//!
//! The Searches place listed four - "Touched by cargo build", "Related to
//! Thesis", "Papers I have not read" - and nobody had saved any of them. They
//! were a hardcoded initial value, and unlike the fixtures found earlier tonight
//! this one is not in a catch: the store simply opens with them, so they render
//! in every session including a real one. Saving a new search did nothing beyond
//! the current window, because the command that would persist it had no host.
//!
//! Both halves are here: the list is read from disk, and saving writes to the
//! same file. It is a plain JSON array in the user's state directory rather than
//! a graph node, because a saved search is a preference about how to look at the
//! graph, not something the graph observed - and because it must survive the
//! graph being rebuilt.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The facets a saved search pins. Mirrors the frontend interface; `null` means
/// "any", which is why every field is an `Option`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchFacets {
    /// The result type, or `None` for any.
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// The project name, or `None` for any.
    pub project: Option<String>,
    /// Days back, or `None` for any time.
    #[serde(rename = "withinDays")]
    pub within_days: Option<i64>,
}

/// One saved search, in the shape the Searches place renders.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SavedSearch {
    /// The id the frontend minted.
    pub id: String,
    /// What the person called it.
    pub name: String,
    /// The query text.
    pub query: String,
    /// The pinned facets.
    pub facets: SearchFacets,
}

/// How many are kept. A saved search is a deliberate act, so this is a bound
/// against a runaway caller rather than a curation policy: it drops the oldest,
/// and at 200 a person has long since stopped using the list as a list.
const MAX_SAVED: usize = 200;

/// `$XDG_STATE_HOME/arlen/knowledge/saved-searches.json`, else
/// `$HOME/.local/state/...`.
///
/// State, not config: the user did not hand-write this and nothing outside this
/// app reads it, so it does not belong beside the files they edit.
fn store_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("arlen/knowledge/saved-searches.json"))
}

/// The saved searches, newest first. An absent file is an empty list, not an
/// error: a person who has saved nothing has saved nothing.
#[tauri::command]
pub async fn knowledge_searches() -> Result<Vec<SavedSearch>, String> {
    let path = store_path().ok_or_else(|| "no state directory".to_string())?;
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Save one search, newest first, replacing an entry with the same id.
///
/// Returns the list as it now stands, so the caller renders what was written
/// rather than what it hoped was written.
#[tauri::command]
pub async fn knowledge_search_save(search: SavedSearch) -> Result<Vec<SavedSearch>, String> {
    if search.name.trim().is_empty() {
        return Err("a saved search needs a name".into());
    }
    let path = store_path().ok_or_else(|| "no state directory".to_string())?;
    // PROPAGATED, not defaulted. `knowledge_searches` already tells an absent
    // file (`Ok(vec![])` - a person who has saved nothing has saved nothing) from
    // one it could not read (`Err`), and `unwrap_or_default` threw that
    // distinction away: an unreadable file started this list EMPTY and the rename
    // below put it over the real one, so a permissions blip or a corrupt byte
    // deleted every saved search and reported success with one entry in it.
    //
    // The comment on that rename says a half-file must not read as "you have
    // saved nothing". This is the same sentence about the read.
    let mut list = knowledge_searches()
        .await
        .map_err(|e| format!("not saved, because the existing list could not be read: {e}"))?;
    list.retain(|s| s.id != search.id);
    list.insert(0, search);
    list.truncate(MAX_SAVED);

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let json = serde_json::to_vec_pretty(&list).map_err(|e| e.to_string())?;
    // Temp-then-rename: an interrupted write must not leave a half-file that
    // reads as "you have saved nothing".
    let tmp = path.with_extension("arlen-tmp");
    std::fs::write(&tmp, json).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })?;
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// These tests each point `XDG_STATE_HOME` at their own directory, and cargo
    /// runs them on threads of one process - so without this they set the SAME
    /// variable underneath each other, and one test's `remove_dir_all` pulls the
    /// store out from under another mid-write. Adding the second test is what
    /// surfaced it; the first had simply never had company.
    static STATE_DIR: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn search(id: &str, name: &str) -> SavedSearch {
        SavedSearch {
            id: id.into(),
            name: name.into(),
            query: "cargo".into(),
            facets: SearchFacets { kind: None, project: Some("Arlen OS".into()), within_days: None },
        }
    }

    #[tokio::test]
    async fn saving_reads_back_and_replaces_by_id() {
        let _serial = STATE_DIR.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("arlen-searches-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: the test owns its own temp state dir; no other thread reads it.
        unsafe { std::env::set_var("XDG_STATE_HOME", &dir) };

        assert!(knowledge_searches().await.unwrap().is_empty(), "nothing saved yet");

        knowledge_search_save(search("a", "First")).await.unwrap();
        let list = knowledge_search_save(search("b", "Second")).await.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "Second", "newest first");

        // Same id again: replaced, not duplicated.
        let list = knowledge_search_save(search("a", "First, renamed")).await.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "First, renamed");

        let read_back = knowledge_searches().await.unwrap();
        assert_eq!(read_back, list, "what is on disk is what was returned");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A save that cannot read the existing list must refuse, not replace it.
    ///
    /// The write is a read-modify-rename, so defaulting the read to empty made
    /// the rename an erase: one unreadable byte and every saved search was gone,
    /// with `Ok` returned and one entry in the list to prove it "worked". The
    /// corrupt file is left exactly as it was, which is what makes it
    /// recoverable.
    #[tokio::test]
    async fn a_save_refuses_rather_than_overwrite_a_list_it_cannot_read() {
        let _serial = STATE_DIR.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("arlen-searches-corrupt-{}", std::process::id()));
        let store = dir.join("arlen/knowledge");
        std::fs::create_dir_all(&store).unwrap();
        // SAFETY: the test owns its own temp state dir; no other thread reads it.
        unsafe { std::env::set_var("XDG_STATE_HOME", &dir) };

        let path = store.join("saved-searches.json");
        std::fs::write(&path, b"{ this is not the list }").unwrap();

        let err = knowledge_search_save(search("a", "First")).await.unwrap_err();
        assert!(err.contains("could not be read"), "says why it refused: {err}");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"{ this is not the list }",
            "the unreadable file is untouched, so it can still be recovered"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_nameless_search_is_refused() {
        let s = SavedSearch { name: "   ".into(), ..search("c", "x") };
        assert!(knowledge_search_save(s).await.is_err());
    }

    #[test]
    fn the_wire_shape_matches_the_frontend() {
        let json = serde_json::to_string(&search("a", "First")).unwrap();
        // `type` and `withinDays` are the frontend's names; a rename here would
        // silently drop a facet rather than fail.
        assert!(json.contains("\"type\":null"), "{json}");
        assert!(json.contains("\"withinDays\":null"), "{json}");
    }
}
