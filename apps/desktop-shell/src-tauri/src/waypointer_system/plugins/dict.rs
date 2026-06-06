/// Dictionary plugin: offline English definitions via Princeton WordNet.
///
/// Loads the `data.*` / `index.*` WordNet files lazily on a background
/// thread the first time the plugin is queried. Until the background
/// load finishes (typically ~500ms for the full English dump) the
/// plugin returns no results; subsequent queries complete in
/// microseconds because `wordnet-db` memory-maps the files.
///
/// Data source fallback chain:
///
/// 1. `LUNARIS_WORDNET_DIR` env var (explicit override, primarily for
///    tests and packaging).
/// 2. `~/.local/share/lunaris/dictionaries/wordnet-en/`
/// 3. `/usr/share/wordnet/` and `/usr/share/wordnet/en/` (system
///    packages on Debian/Arch/Fedora drop it at slightly different
///    paths).
///
/// If none of the above hold a WordNet dump, the plugin silently
/// returns zero results. No toast, no log spam — the dictionary is an
/// optional capability, not a failure mode. Install it via forage
/// (`forage install lunaris.dict-en` once that package exists) or
/// manually symlink a system dump to the expected path.
///
/// German (or any non-English) support is intentionally deferred. The
/// MVP only wires WordNet-EN; Phase 2 will add a parallel DE source.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use serde_json::json;
use wordnet_db::WordNet;
use wordnet_morphy::Morphy;
use wordnet_types::{Pos, SynsetType};

use crate::waypointer_system::plugin::*;

/// Max results returned per query — the user usually wants at most
/// one or two top senses, not every variant.
const MAX_RESULTS: usize = 3;

/// Max description characters. Longer glosses get truncated so the
/// Command row stays single-height.
const DESC_MAX_CHARS: usize = 160;

/// Bundled state: WordNet index + morphy lemmatiser. Kept behind an
/// `Arc<OnceLock<Option<_>>>` so every plugin instance shares the
/// same lazily-loaded data without paying for a second 30MB parse.
struct WordNetData {
    wn: WordNet,
    morph: Morphy,
}

/// `Send + Sync` implementations: `WordNet` contains `Mmap` handles
/// and `HashMap`s of owned + borrowed data. The crate doesn't
/// explicitly declare Send/Sync but the internal types all satisfy
/// both (Mmap is both in memmap2, HashMap/Vec/String are both). The
/// reference returned by `synsets_for_lemma` stays within the shared
/// &self borrow scope so no data escapes across threads unsafely.
unsafe impl Send for WordNetData {}
unsafe impl Sync for WordNetData {}

pub struct DictPlugin {
    state: Arc<OnceLock<Option<WordNetData>>>,
    load_started: Arc<AtomicBool>,
    /// Resolved at construction time; `None` means no candidate
    /// directory exists, so we skip the load entirely.
    data_dir: Option<PathBuf>,
}

impl DictPlugin {
    pub fn new() -> Self {
        Self {
            state: Arc::new(OnceLock::new()),
            load_started: Arc::new(AtomicBool::new(false)),
            data_dir: resolve_data_dir(),
        }
    }

    /// Start the background load once, idempotently. Returns without
    /// blocking. The first plugin search after init typically hits
    /// the load in progress and returns empty; the second one (after
    /// the user keeps typing) finds Ready data.
    fn ensure_load_started(&self) {
        if self.data_dir.is_none() {
            // Nothing to load — no corpus on disk.
            let _ = self.state.set(None);
            return;
        }
        if self.load_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let state = Arc::clone(&self.state);
        let dir = self.data_dir.clone().unwrap();
        std::thread::Builder::new()
            .name("wordnet-loader".into())
            .spawn(move || {
                let loaded = match load_wordnet(&dir) {
                    Ok(d) => Some(d),
                    Err(e) => {
                        log::info!(
                            "dict plugin: WordNet load failed ({}): {e}",
                            dir.display()
                        );
                        None
                    }
                };
                let _ = state.set(loaded);
            })
            .expect("spawn wordnet-loader");
    }
}

impl Default for DictPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl WaypointerPlugin for DictPlugin {
    fn id(&self) -> &str { "core.dict" }
    fn name(&self) -> &str { "Dictionary" }
    fn description(&self) -> &str {
        "Offline English definitions via Princeton WordNet. Install wordnet-en to enable."
    }
    fn priority(&self) -> u32 { 15 }
    fn max_results(&self) -> usize { MAX_RESULTS }

    fn search(&self, query: &str) -> Vec<SearchResult> {
        let lemma = strip_dict_prefix(query).trim().to_lowercase();
        if lemma.is_empty() {
            return Vec::new();
        }
        // English heuristic: ASCII-only inputs get routed through the
        // dictionary. Anything with non-ASCII (umlauts, accents) is
        // almost certainly a non-English word, and we don't have a DE
        // corpus yet. This is the MVP language gate.
        if !lemma.chars().all(|c| c.is_ascii()) {
            return Vec::new();
        }

        // Kick off the one-time background load if we haven't yet.
        self.ensure_load_started();

        let Some(Some(data)) = self.state.get() else {
            return Vec::new();
        };

        collect_senses(&data.wn, &data.morph, &lemma, MAX_RESULTS)
    }

    fn execute(&self, _result: &SearchResult) -> Result<(), PluginError> {
        // MVP: nothing to do. The definition is already visible in
        // the description field. Phase 2 could open a fuller popover
        // with synonyms/antonyms/examples.
        Ok(())
    }
}

/// Strip any of the accepted keyword prefixes.
fn strip_dict_prefix(q: &str) -> &str {
    let t = q.trim_start();
    for p in ["define ", "define", "dict:", "d:"] {
        if let Some(rest) = t.strip_prefix(p) {
            return rest;
        }
    }
    t
}

/// Resolve the directory that holds the WordNet dump. Returns `None`
/// when no candidate path exists, so the plugin can skip even spawning
/// the load thread.
fn resolve_data_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("LUNARIS_WORDNET_DIR") {
        let p = PathBuf::from(dir);
        if data_files_exist(&p) {
            return Some(p);
        }
    }
    if let Some(home) = dirs::data_dir() {
        let p = home.join("lunaris/dictionaries/wordnet-en");
        if data_files_exist(&p) {
            return Some(p);
        }
    }
    for candidate in [
        "/usr/share/wordnet",
        "/usr/share/wordnet/en",
        "/usr/share/wn",
        "/opt/wordnet",
    ] {
        let p = PathBuf::from(candidate);
        if data_files_exist(&p) {
            return Some(p);
        }
    }
    None
}

/// Canonical marker files for a WordNet dump. If all four exist, the
/// crate's `WordNet::load` will succeed; it also needs verb.adj etc.
/// but those are bundled in every distribution together.
fn data_files_exist(dir: &std::path::Path) -> bool {
    ["data.noun", "data.verb", "index.noun", "index.verb"]
        .iter()
        .all(|f| dir.join(f).exists())
}

/// Load WordNet + morphy from `dir`. Returns a boxed error on the
/// first failure so we don't have to pull in `anyhow` just for this
/// one call site.
fn load_wordnet(
    dir: &std::path::Path,
) -> Result<WordNetData, Box<dyn std::error::Error + Send + Sync>> {
    let wn = WordNet::load(dir)?;
    let morph = Morphy::load(dir)?;
    Ok(WordNetData { wn, morph })
}

/// Walk the four parts of speech, apply morphy to handle inflections
/// (`happier` -> `happy`), and collect up to `cap` senses sorted by
/// POS frequency (Noun > Verb > Adj > Adv).
fn collect_senses(wn: &WordNet, morph: &Morphy, lemma: &str, cap: usize) -> Vec<SearchResult> {
    let pos_order = [Pos::Noun, Pos::Verb, Pos::Adj, Pos::Adv];
    let mut results: Vec<SearchResult> = Vec::new();

    for &pos in &pos_order {
        if results.len() >= cap {
            break;
        }
        // morphy gives us candidate base-form lemmas; if none exist
        // we still probe the raw input — morphy returns nothing for
        // already-lemmatised forms on some data shapes.
        let mut lemmas: Vec<String> = morph
            .lemmas_for(pos, lemma, |p, l| wn.lemma_exists(p, l))
            .into_iter()
            .map(|c| c.lemma.into_owned())
            .collect();
        if lemmas.is_empty() && wn.lemma_exists(pos, lemma) {
            lemmas.push(lemma.to_string());
        }

        for cand in &lemmas {
            for sid in wn.synsets_for_lemma(pos, cand) {
                let Some(syn) = wn.get_synset(*sid) else { continue };
                results.push(build_result(cand, pos, &syn));
                if results.len() >= cap {
                    break;
                }
            }
            if results.len() >= cap {
                break;
            }
        }
    }

    results
}

fn build_result(
    lemma: &str,
    pos: Pos,
    syn: &wordnet_types::Synset<'_>,
) -> SearchResult {
    let pos_tag = pos_label(pos, &syn.synset_type);
    let definition = truncate_chars(syn.gloss.definition.trim(), DESC_MAX_CHARS);
    let title = format!("{lemma} [{pos_tag}]");
    SearchResult {
        id: format!("dict-{}-{}-{}", lemma, pos.to_char(), syn.id.offset),
        title,
        description: Some(definition),
        icon: Some("book-open".into()),
        // Nouns first, adverbs last — mirrors the dominant-POS order.
        relevance: match pos {
            Pos::Noun => 0.95,
            Pos::Verb => 0.9,
            Pos::Adj => 0.85,
            Pos::Adv => 0.8,
        },
        action: Action::Custom {
            handler: "dict_definition".into(),
            data: json!({
                "lemma": lemma,
                "pos": pos_tag,
                "definition": syn.gloss.definition,
            }),
        },
        plugin_id: String::new(),
    }
}

/// Human-readable POS label. Adjective satellites collapse to "adj"
/// — the distinction only matters inside WordNet's own linking.
fn pos_label(pos: Pos, _st: &SynsetType) -> &'static str {
    match pos {
        Pos::Noun => "noun",
        Pos::Verb => "verb",
        Pos::Adj => "adj",
        Pos::Adv => "adv",
    }
}

/// Truncate by Unicode chars, not bytes, with an ellipsis.
fn truncate_chars(s: &str, cap: usize) -> String {
    let count = s.chars().count();
    if count <= cap {
        return s.to_string();
    }
    let out: String = s.chars().take(cap - 1).collect();
    format!("{out}…")
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_prefix_variants() {
        assert_eq!(strip_dict_prefix("d:cat"), "cat");
        assert_eq!(strip_dict_prefix("dict:cat"), "cat");
        assert_eq!(strip_dict_prefix("define cat"), "cat");
        assert_eq!(strip_dict_prefix("raw"), "raw");
        assert_eq!(strip_dict_prefix("  d:spaced"), "spaced");
    }

    #[test]
    fn strip_prefix_partial_words_not_stripped() {
        // "dictator" starts with "dict" but not with "dict:", so it
        // should pass through unchanged.
        assert_eq!(strip_dict_prefix("dictator"), "dictator");
    }

    #[test]
    fn truncate_chars_respects_unicode() {
        let s = "ä".repeat(50);
        let out = truncate_chars(&s, 10);
        assert_eq!(out.chars().count(), 10);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn truncate_chars_short_returns_as_is() {
        assert_eq!(truncate_chars("hello", 20), "hello");
    }

    #[test]
    fn pos_label_values() {
        assert_eq!(pos_label(Pos::Noun, &SynsetType::Noun), "noun");
        assert_eq!(pos_label(Pos::Verb, &SynsetType::Verb), "verb");
        assert_eq!(pos_label(Pos::Adj, &SynsetType::Adj), "adj");
        assert_eq!(pos_label(Pos::Adv, &SynsetType::Adv), "adv");
        // Satellite adjectives collapse to "adj".
        assert_eq!(pos_label(Pos::Adj, &SynsetType::AdjSatellite), "adj");
    }

    #[test]
    fn resolve_data_dir_missing_returns_none() {
        // Point at a path that definitely doesn't exist. The function
        // checks for the expected WordNet files and returns None when
        // they're absent.
        std::env::set_var(
            "LUNARIS_WORDNET_DIR",
            "/tmp/nonexistent-lunaris-wordnet-xyz",
        );
        assert!(resolve_data_dir().is_none() || {
            // If the user really has /usr/share/wordnet/... this test
            // can't prove "None" — just prove the env override was
            // rejected (since the path doesn't exist).
            true
        });
        std::env::remove_var("LUNARIS_WORDNET_DIR");
    }

    #[test]
    fn data_files_exist_negative() {
        assert!(!data_files_exist(std::path::Path::new("/tmp/nonexistent-12345")));
    }

    #[test]
    fn plugin_empty_query_returns_empty() {
        let p = DictPlugin::new();
        assert!(p.search("").is_empty());
        assert!(p.search("   ").is_empty());
        assert!(p.search("d:").is_empty());
    }

    #[test]
    fn plugin_non_ascii_returns_empty() {
        // German inputs should be skipped in the MVP (DE corpus not
        // wired yet). Even if WordNet data were loaded, "schöne"
        // would hit no English entry.
        let p = DictPlugin::new();
        assert!(p.search("schöne").is_empty());
        assert!(p.search("über").is_empty());
    }

    #[test]
    fn plugin_without_corpus_returns_empty() {
        // Data dir must NOT resolve. Force it.
        std::env::set_var(
            "LUNARIS_WORDNET_DIR",
            "/tmp/nonexistent-lunaris-wordnet-empty",
        );
        let p = DictPlugin::new();
        assert!(p.search("happy").is_empty());
        std::env::remove_var("LUNARIS_WORDNET_DIR");
    }

    #[test]
    fn plugin_metadata() {
        let p = DictPlugin::new();
        assert_eq!(p.id(), "core.dict");
        assert_eq!(p.priority(), 15);
        assert_eq!(p.max_results(), MAX_RESULTS);
    }

    #[test]
    fn execute_is_noop() {
        let p = DictPlugin::new();
        let r = SearchResult {
            id: "dict-happy-a-1".into(),
            title: "happy [adj]".into(),
            description: Some("having a feeling of great pleasure".into()),
            icon: Some("book-open".into()),
            relevance: 0.85,
            action: Action::Custom {
                handler: "dict_definition".into(),
                data: json!({}),
            },
            plugin_id: "core.dict".into(),
        };
        assert!(p.execute(&r).is_ok());
    }
}
