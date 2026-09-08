//! Find a character by name or codepoint, as the first Tier 1 WASM module.
//!
//! It is the in-shell `unicode` waypointer plugin, moved out of the process and
//! behind the component ABI. Same behaviour, same prefix, same actions: type
//! `U+2764` and get the character, type `HEART` and get the ones whose Unicode
//! name contains it, press enter and the host copies it.
//!
//! **Why this one.** It needs no host import at all - no graph, no network, no
//! events - so anything that goes wrong is the ABI, the build, discovery or
//! consent rather than the module's own logic. That is what a first guest is
//! for.
//!
//! **It generates its own bindings.** `sdk/module-sdk` has a Rust `host.rs`
//! whose body is `// wit-bindgen-generated stub goes here once S5 wires it`,
//! twice, and no `wit-bindgen` dependency, so there is nothing for a Rust guest
//! to import. This module does what a third-party author would have to do:
//! point `wit_bindgen::generate!` at the WIT and implement the world.

// `generate_all` is not a convenience. The `waypointer-provider` world imports
// graph, network, events and log unconditionally, so a module that touches none
// of them - this one - still has to generate bindings for all four or the macro
// refuses: "missing `with` mapping for the key `arlen:host/graph@0.1.0`". The
// world says nothing about what a module actually needs; the narrowing lives in
// the manifest and the host's linker, which is a real gap between what the ABI
// expresses and what the capability model promises.
wit_bindgen::generate!({
    path: "../../sdk/module-sdk/wit",
    world: "waypointer-provider",
    generate_all,
});

use exports::arlen::waypointer::provider::{Action, Guest, SearchResult};

struct Unicode;

/// The most results a single search returns.
///
/// Matches the in-process plugin's `max_results` and the manifest's
/// `[waypointer.search] max_results`, which modulesd truncates to. Three copies
/// of one number, and the manifest's is the one that binds.
const MAX_RESULTS: usize = 20;

/// Every named codepoint, built once in `init` and read by every `search`.
///
/// **This is what the one-time `init` budget is for.** Before 8 September the
/// host handed `init` the same 1 M fuel a per-keystroke `search` gets, so a
/// module could build nothing at startup that it did not have to rebuild on
/// every keystroke - and a scan of the name space costs 200-400 M. The guest
/// paid that on every letter typed and trapped. Now `init` gets its own budget,
/// the store lives for the module's lifetime, and this lives in it.
///
/// **One buffer and a range per name, not forty thousand `String`s.** The first
/// cut allocated a `String` per entry and then uppercased it into a second one,
/// and cost between 800 M and 1 G fuel - past the budget the host grants. Two
/// allocations per name is the whole difference; the names themselves are the
/// same bytes either way.
///
/// No uppercasing either, and that is not a shortcut: every Unicode character
/// name is uppercase ASCII by definition ("HEAVY BLACK HEART"), so the old
/// `to_uppercase()` produced an identical string at the price of an allocation.
/// The needle is uppercased once per query instead.
struct Index {
    /// Every name, concatenated.
    text: String,
    /// `(codepoint, start, end)` into `text`, in codepoint order.
    spans: Vec<(u32, u32, u32)>,
    /// Which names contain each three-letter sequence.
    ///
    /// **This is what makes a query fit the per-call budget.** Measured on 8
    /// September: a substring search over the concatenated megabyte costs more
    /// than 10 M fuel however it is written, because it is a megabyte of
    /// comparisons and 1 M fuel is about ten milliseconds of work. With this, a
    /// query looks at the few hundred names that could possibly match instead of
    /// all forty thousand.
    ///
    /// One flat list, and where each bucket starts in it.
    ///
    /// Not a hash map and not a vector of vectors, and both were tried: hashing
    /// 800000 insertions costs more than the index saves, and 59319 growing
    /// `Vec`s put `init` over its budget on their own. Counting first and filling
    /// once has no per-bucket allocation at all - `starts[b]..starts[b + 1]` is
    /// the bucket.
    entries: Vec<u32>,
    starts: Vec<u32>,
}

/// The alphabet a Unicode name is written in, mapped to a small dense range.
///
/// A-Z, 0-9, space and hyphen cover every character name; anything else lands in
/// the last slot rather than being dropped, so an unexpected byte costs recall
/// on one bucket instead of losing a name.
const SYMBOLS: usize = 39;

fn symbol(b: u8) -> usize {
    match b {
        b'A'..=b'Z' => (b - b'A') as usize,
        b'0'..=b'9' => 26 + (b - b'0') as usize,
        b' ' => 36,
        b'-' => 37,
        _ => 38,
    }
}

/// The bucket a three-byte sequence belongs to.
fn trigram(w: &[u8]) -> usize {
    symbol(w[0]) * SYMBOLS * SYMBOLS + symbol(w[1]) * SYMBOLS + symbol(w[2])
}

/// The shortest name query the index can answer.
///
/// Two letters would put a fifth of the corpus in one bucket and cost more to
/// verify than it saves, and a one or two letter query is not a name search
/// anybody means. The codepoint path is unaffected: `U+41` still works.
const MIN_NAME_QUERY: usize = 3;

static NAMES: std::sync::OnceLock<Index> = std::sync::OnceLock::new();

impl Guest for Unicode {
    fn init() -> Result<(), String> {
        // The whole assigned space, once. `char::from_u32` skips surrogates and
        // `name` returns nothing for the unassigned, so what lands here is
        // exactly the named codepoints - about forty thousand of them.
        let mut text = String::with_capacity(1 << 20);
        let mut spans = Vec::with_capacity(50_000);
        for cp in 0x20..0x11_0000u32 {
            let Some(ch) = char::from_u32(cp) else { continue };
            let Some(name) = unicode_names2::name(ch) else { continue };
            let start = text.len() as u32;
            text.extend(name);
            spans.push((cp, start, text.len() as u32));
            // A separator, so nothing can match across two names.
            text.push('\n');
        }

        // Then the trigram buckets, counted first and filled once.
        let buckets = SYMBOLS * SYMBOLS * SYMBOLS;
        let bytes = text.as_bytes();
        let mut starts = vec![0u32; buckets + 1];
        for (_, start, end) in &spans {
            for w in bytes[*start as usize..*end as usize].windows(MIN_NAME_QUERY) {
                starts[trigram(w) + 1] += 1;
            }
        }
        for b in 0..buckets {
            starts[b + 1] += starts[b];
        }
        let mut entries = vec![0u32; starts[buckets] as usize];
        let mut at = starts.clone();
        for (i, (_, start, end)) in spans.iter().enumerate() {
            for w in bytes[*start as usize..*end as usize].windows(MIN_NAME_QUERY) {
                let b = trigram(w);
                entries[at[b] as usize] = i as u32;
                at[b] += 1;
            }
        }

        NAMES
            .set(Index { text, spans, entries, starts })
            .map_err(|_| "the index was built twice".to_string())
    }

    fn search(query: String) -> Vec<SearchResult> {
        let q = query.trim();
        if q.is_empty() {
            return Vec::new();
        }

        // A codepoint the person typed out: one answer, no scan.
        if let Some(cp) = parse_codepoint(q) {
            if let Some(ch) = char::from_u32(cp) {
                return vec![hit(cp, ch, &name_of(ch, cp), 1.0)];
            }
        }

        // Name search, over the index `init` built. What this used to do - walk
        // the whole codepoint space per query - is what the module could not
        // afford, and the index is why it now can.
        let needle = q.to_uppercase();
        let Some(index) = NAMES.get() else {
            // `init` is called before any search, so this is unreachable rather
            // than a fallback. Answering nothing is the honest response to a
            // state that should not exist: rescanning here would hide it.
            return Vec::new();
        };
        // The rarest of the query's trigrams decides which names to look at.
        //
        // Every name that contains the needle contains every one of its
        // trigrams, so any single bucket is a superset of the answer and the
        // smallest one is the cheapest superset. Then each candidate is verified
        // with a real substring test, so the bucket is a filter and never the
        // answer.
        if needle.len() < MIN_NAME_QUERY {
            return Vec::new();
        }
        let bytes = needle.as_bytes();
        let mut best: &[u32] = &[];
        let mut first = true;
        for w in bytes.windows(MIN_NAME_QUERY) {
            let b = trigram(w);
            let list = &index.entries[index.starts[b] as usize..index.starts[b + 1] as usize];
            if first || list.len() < best.len() {
                best = list;
                first = false;
            }
        }
        let mut out = Vec::new();
        let mut seen = u32::MAX;
        for i in best {
            // A name lands in its bucket once per occurrence of the trigram, so
            // the same name can appear twice in a row; the list is built in name
            // order, so one comparison is enough to skip the repeat.
            if *i == seen {
                continue;
            }
            seen = *i;
            if out.len() >= MAX_RESULTS {
                break;
            }
            let (cp, start, end) = index.spans[*i as usize];
            let name = &index.text[start as usize..end as usize];
            let Some(at) = name.find(&needle) else { continue };
            let Some(ch) = char::from_u32(cp) else { continue };
            let relevance = if at == 0 { 0.9 } else { 0.5 };
            out.push(hit(cp, ch, name, relevance));
        }
        out.sort_by(|a, b| b.relevance.total_cmp(&a.relevance));
        out
    }

    fn execute(_hit: SearchResult) -> Result<(), String> {
        // Nothing to do: the action is `copy`, which the host fulfils. A module
        // only sees `execute` for its own `custom` actions.
        Ok(())
    }

    fn shutdown() {}
}

/// One result, in the shape the ABI has.
///
/// **The keys are here and this module still sends none, which is the honest
/// answer rather than an omission.** When this was written the ABI had no
/// `title-key`, so its own report said a module's results were untranslatable by
/// construction; that was the finding that led to ruling 2 and the fields now
/// exist. But nothing on a unicode result is prose. "HEAVY BLACK HEART" is the
/// character's name in the standard, the same in every language, and `U+2764` is
/// a number. A key here would point at a catalogue entry that could only repeat
/// them. The `man` module is where the fields earn their place: the sentence
/// "Section 1" is written BY the module and has a German form.
fn hit(cp: u32, ch: char, name: &str, relevance: f32) -> SearchResult {
    SearchResult {
        id: format!("u-{cp:04X}"),
        title: format!("{ch}  {name}"),
        title_key: None,
        description: Some(format!("U+{cp:04X}")),
        description_key: None,
        args: None,
        icon: None,
        relevance,
        action: Action::Copy(ch.to_string()),
    }
}

/// A character's Unicode name, or its codepoint when it has none.
fn name_of(ch: char, cp: u32) -> String {
    unicode_names2::name(ch)
        .map(|n| n.to_string())
        .unwrap_or_else(|| format!("U+{cp:04X}"))
}

/// `U+2764`, `0x2764` and a bare `2764` all mean the same character.
fn parse_codepoint(s: &str) -> Option<u32> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("U+").or_else(|| s.strip_prefix("u+")) {
        return u32::from_str_radix(hex, 16).ok();
    }
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        return u32::from_str_radix(hex, 16).ok();
    }
    if s.len() >= 4 && s.chars().all(|c| c.is_ascii_hexdigit()) {
        return u32::from_str_radix(s, 16).ok();
    }
    None
}

export!(Unicode);
