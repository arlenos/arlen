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
}

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
            // A separator, so a search over the whole buffer cannot match across
            // two names. No Unicode name contains a newline, so it can never be
            // part of a needle either.
            text.push('\n');
        }
        NAMES
            .set(Index { text, spans })
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
        // ONE search over the whole buffer, not one per name.
        //
        // Measured on 8 September: forty thousand `contains` calls cost more than
        // 50 M fuel even with the index in hand, because each one pays a
        // substring-searcher setup that dwarfs the twenty bytes it looks at. A
        // single pass over the concatenated megabyte, then a binary search per
        // hit to find which name it landed in, does the same work once.
        let mut out = Vec::new();
        for (at, _) in index.text.match_indices(&needle) {
            if out.len() >= MAX_RESULTS {
                break;
            }
            let at = at as u32;
            // The span whose start is the last one at or before the match.
            let i = match index.spans.binary_search_by_key(&at, |(_, start, _)| *start) {
                Ok(i) => i,
                Err(0) => continue,
                Err(i) => i - 1,
            };
            let (cp, start, end) = index.spans[i];
            if at >= end {
                continue; // landed on a separator, which is not part of a name
            }
            let Some(ch) = char::from_u32(cp) else { continue };
            let name = &index.text[start as usize..end as usize];
            let relevance = if at == start { 0.9 } else { 0.5 };
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
/// NB no `title_key`/`description_key`. The shell's own `SearchResult` carries
/// them so a result can name a catalogue entry instead of prose, and the wire
/// type says in its doc that it deliberately did not follow. So this title -
/// the character and its Unicode name - is untranslatable by construction, and
/// so is every other module's.
fn hit(cp: u32, ch: char, name: &str, relevance: f32) -> SearchResult {
    SearchResult {
        id: format!("u-{cp:04X}"),
        title: format!("{ch}  {name}"),
        description: Some(format!("U+{cp:04X}")),
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
