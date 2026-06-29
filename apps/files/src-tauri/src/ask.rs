//! "Ask Arlen": the scoped natural-language front-end to the faceted filter
//! (file-manager-plan.md item 6a). A folder-scoped question is mapped to a draft
//! facet selection in the existing vocabulary (project / type / time / touched),
//! plus a summary of what was read.
//!
//! The anti-Recall move: the inference is a transparent, bounded, LOCAL
//! keyword/phrase mapping over the closed facet vocabulary - not an opaque model
//! dredging the graph - the reads it performs are surfaced to the user (the
//! `reads` summary the banner shows), and the knowledge-graph read it issues is
//! scoped to the caller and audited daemon-side (the knowledge daemon's read
//! path). A richer natural-language understanding routed through the ai-daemon is
//! the model-gated upgrade; this deterministic mapping is the always-available
//! first cut that needs no model and never leaves the box.
//!
//! Pull, never push: this only DRAFTS a facet selection the user sees as editable
//! chips. Nothing is saved, moved or filtered until the user acts.

use serde::Serialize;

/// What the assistant read to draft the filter, for the transparency line.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AskReads {
    /// Files examined in the scoped folder.
    pub files: usize,
    /// Knowledge-graph entities (projects + touching apps) consulted.
    pub tags: usize,
}

/// The drafted facet selection in the existing vocabulary. Each field is a
/// (possibly empty) value list; serialized to the frontend's
/// `Partial<Record<FacetGroup, string[]>>` shape (empty lists are harmless, the
/// frontend reads `facets[group] ?? []`).
#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
pub struct AskFacets {
    /// Selected project ids (`FILE_PART_OF` membership).
    pub project: Vec<String>,
    /// Selected type kinds (the closed set document/image/audio/video/archive/code).
    #[serde(rename = "type")]
    pub kind: Vec<String>,
    /// The single recency cutoff key (day/week/month/older), as a 0-or-1 list.
    pub time: Vec<String>,
    /// Selected app ids that touched the files (`ACCESSED_BY`).
    pub touched: Vec<String>,
}

/// The `files_ask` result: a drafted facet selection plus the reads summary.
#[derive(Debug, Clone, Serialize)]
pub struct AskResult {
    /// The drafted facets in the existing vocabulary.
    pub facets: AskFacets,
    /// What was read to draft them.
    pub reads: AskReads,
}

/// Whether `q` contains `word` as a whole token (bounded by non-alphanumerics), so
/// `doc` does not match `docker` and `code` not `codex`. `q` is already lowercased.
fn contains_word(q: &str, word: &str) -> bool {
    q.split(|c: char| !c.is_alphanumeric()).any(|tok| tok == word)
}

/// Map a phrase to the single recency cutoff key (time is single-select). The
/// specific multi-word phrases are checked before the bare-word fallbacks, and
/// `older` before `old` so "older" is not eaten by a substring. Returns a facet
/// key (`day`/`week`/`month`/`older`) or `None`. `q` is lowercased.
fn infer_time(q: &str) -> Option<&'static str> {
    const PHRASES: &[(&str, &str)] = &[
        ("today", "day"),
        ("yesterday", "day"),
        ("this week", "week"),
        ("last week", "week"),
        ("past week", "week"),
        ("last 7 days", "week"),
        ("recently", "week"),
        ("recent", "week"),
        ("this month", "month"),
        ("last month", "month"),
        ("past month", "month"),
        ("last 30 days", "month"),
        ("older", "older"),
        ("a while ago", "older"),
        ("long ago", "older"),
    ];
    for (phrase, key) in PHRASES {
        if q.contains(phrase) {
            return Some(key);
        }
    }
    // Bare-word fallbacks, after the phrases (so "this week" is not shadowed).
    if contains_word(q, "today") || contains_word(q, "day") {
        return Some("day");
    }
    if contains_word(q, "week") {
        return Some("week");
    }
    if contains_word(q, "month") {
        return Some("month");
    }
    None
}

/// The type-kind trigger words. A query may match several kinds (the type facet
/// unions); the matched kinds are returned in the vocabulary's order. Whole-word
/// matched, so an extension or noun does not partially collide.
fn infer_types(q: &str) -> Vec<String> {
    const KINDS: &[(&str, &[&str])] = &[
        (
            "document",
            &[
                "document", "documents", "doc", "docs", "pdf", "pdfs", "text", "note", "notes",
                "spreadsheet", "spreadsheets", "slide", "slides", "presentation", "presentations",
            ],
        ),
        (
            "image",
            &[
                "image", "images", "photo", "photos", "picture", "pictures", "screenshot",
                "screenshots", "png", "jpg", "jpeg",
            ],
        ),
        (
            "audio",
            &["audio", "music", "song", "songs", "sound", "sounds", "mp3", "podcast", "podcasts"],
        ),
        (
            "video",
            &["video", "videos", "movie", "movies", "clip", "clips", "mp4", "footage"],
        ),
        (
            "archive",
            &["archive", "archives", "zip", "zips", "tarball", "tarballs", "backup", "backups"],
        ),
        ("code", &["code", "source", "script", "scripts", "program", "programs", "snippet", "snippets"]),
    ];
    KINDS
        .iter()
        .filter(|(_, words)| words.iter().any(|w| contains_word(q, w)))
        .map(|(kind, _)| (*kind).to_string())
        .collect()
}

/// Match known `(id, name)` pairs against the query: an entity whose name appears
/// as a contiguous (case-insensitive) phrase in the query is selected. Returns the
/// matched ids, deduped, in input order. Used for both projects and touching apps.
/// `q` is lowercased; an empty name never matches.
fn match_named(q: &str, pairs: &[(String, String)]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (id, name) in pairs {
        let name_l = name.trim().to_lowercase();
        if name_l.is_empty() {
            continue;
        }
        if q.contains(&name_l) && !out.contains(id) {
            out.push(id.clone());
        }
    }
    out
}

/// Infer a draft facet selection from a natural-language `query`, matching project
/// and touching-app names against the (already capability-scoped) `projects` and
/// `touched` vocab read from the graph. Pure, so the mapping is unit-tested without
/// a folder or a daemon.
pub fn infer_facets(
    query: &str,
    projects: &[(String, String)],
    touched: &[(String, String)],
) -> AskFacets {
    let q = query.to_lowercase();
    AskFacets {
        project: match_named(&q, projects),
        kind: infer_types(&q),
        time: infer_time(&q).map(|k| vec![k.to_string()]).unwrap_or_default(),
        touched: match_named(&q, touched),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proj(id: &str, name: &str) -> (String, String) {
        (id.to_string(), name.to_string())
    }

    #[test]
    fn infers_type_and_time_from_a_question() {
        let f = infer_facets("show me images from this week", &[], &[]);
        assert_eq!(f.kind, vec!["image"]);
        assert_eq!(f.time, vec!["week"]);
        assert!(f.project.is_empty());
    }

    #[test]
    fn documents_today() {
        let f = infer_facets("documents I worked on today", &[], &[]);
        assert_eq!(f.kind, vec!["document"]);
        assert_eq!(f.time, vec!["day"]);
    }

    #[test]
    fn multiple_types_union_in_vocab_order() {
        let f = infer_facets("videos and photos", &[], &[]);
        // Vocabulary order is document, image, audio, video, ...; image precedes video.
        assert_eq!(f.kind, vec!["image", "video"]);
    }

    #[test]
    fn matches_a_project_name_to_its_id() {
        let projects = vec![proj("p-arlen", "Arlen"), proj("p-other", "Holiday Photos")];
        let f = infer_facets("code in the arlen project", &projects, &[]);
        assert_eq!(f.project, vec!["p-arlen"]);
        assert_eq!(f.kind, vec!["code"]);
    }

    #[test]
    fn whole_word_matching_avoids_false_type_hits() {
        // "codex" must not trip "code"; "docker" must not trip "doc".
        let f = infer_facets("the codex docker setup", &[], &[]);
        assert!(f.kind.is_empty());
    }

    #[test]
    fn time_phrase_beats_the_bare_word_fallback() {
        // "this month" maps to month, not shadowed by a stray "week".
        assert_eq!(infer_time("files from this month"), Some("month"));
        assert_eq!(infer_time("older backups"), Some("older"));
        assert_eq!(infer_time("nothing temporal here"), None);
    }

    #[test]
    fn matches_a_touching_app() {
        let apps = vec![("app.firefox".to_string(), "firefox".to_string())];
        let f = infer_facets("pages I opened in firefox", &[], &apps);
        assert_eq!(f.touched, vec!["app.firefox"]);
    }

    #[test]
    fn an_empty_or_plain_query_drafts_nothing() {
        let f = infer_facets("", &[proj("p", "Arlen")], &[]);
        assert!(f.project.is_empty() && f.kind.is_empty() && f.time.is_empty());
        let f2 = infer_facets("just some files", &[], &[]);
        assert_eq!(f2, AskFacets::default());
    }
}
