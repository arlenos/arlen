//! The mandatory read-after-write self-check.
//!
//! Every public edit ([`checked_set`], [`checked_remove`]) runs the inner
//! handler primitive to produce a CANDIDATE text, then verifies the candidate
//! before returning it. The check is the in-memory realization of the adapter
//! schema's `verify` field: it catches a lossy edit BEFORE a byte is written,
//! which is cheaper and stronger than only re-reading the file afterwards (IP-R3
//! still layers the on-disk read-back on top). On ANY self-check failure the edit
//! is rejected and the ORIGINAL text is returned to the caller unchanged: a
//! checked edit never returns lossy text.
//!
//! The three assertions:
//!  - **(a)** the edited key now holds the new value (set) / is gone (remove);
//!  - **(b)** every OTHER modelled key is unchanged (no added, removed or
//!    value-changed sibling);
//!  - **(c)** the unmodelled bytes (comments, blank lines, unknown content)
//!    outside the target key's own region are preserved.
//!
//! (a) and (b) are non-negotiable for every format and run off the re-parsed
//! [`ConfigModel`](crate::ConfigModel). (c) compares the document with every
//! modelled value masked out, so only comments/structure remain; for a format
//! where exact masking is impractical it degrades to comparing the comment
//! content, documented per handler. The float comparison in (a) uses
//! [`ConfigValue::same_value`](crate::ConfigValue::same_value) (bitwise) so a
//! re-parsed float neither spuriously fails nor spuriously passes.

use crate::error::{EditError, SelfCheckError};
use crate::model::{ConfigModel, ConfigValue};
use crate::FormatHandler;

/// Set `key` to `value` through `handler`, then run the read-after-write
/// self-check. Returns the candidate text on success, or the original text plus
/// an [`EditError`] on any failure (the caller never receives lossy text).
pub fn checked_set(
    handler: &dyn FormatHandler,
    text: &str,
    key: &str,
    value: &ConfigValue,
) -> Result<String, EditError> {
    // Refuse an Opaque target up front: the existing value at `key`, if any, must
    // be scalar to be overwritten.
    let original = handler
        .read(text)
        .map_err(|e| EditError::Failed(format!("read original: {e}")))?;
    if matches!(original.get(key), Some(ConfigValue::Opaque)) {
        return Err(EditError::OpaqueTarget {
            key: key.to_string(),
        });
    }

    let candidate = handler.set(text, key, value)?;
    verify(handler, text, &candidate, &original, key, Some(value))?;
    Ok(candidate)
}

/// Remove `key` through `handler`, then run the read-after-write self-check.
/// An absent key is a no-op that passes the check (the candidate equals the
/// original model).
pub fn checked_remove(
    handler: &dyn FormatHandler,
    text: &str,
    key: &str,
) -> Result<String, EditError> {
    let original = handler
        .read(text)
        .map_err(|e| EditError::Failed(format!("read original: {e}")))?;
    let candidate = handler.remove(text, key)?;
    verify(handler, text, &candidate, &original, key, None)?;
    Ok(candidate)
}

/// Run all three self-check assertions against the candidate. `expected` is
/// `Some(value)` for a set (the key must now hold it) or `None` for a remove (the
/// key must be gone).
fn verify(
    handler: &dyn FormatHandler,
    original_text: &str,
    candidate_text: &str,
    original_model: &ConfigModel,
    key: &str,
    expected: Option<&ConfigValue>,
) -> Result<(), SelfCheckError> {
    // The candidate must still parse, or the edit corrupted the document.
    let candidate_model = handler
        .read(candidate_text)
        .map_err(|e| SelfCheckError::CandidateUnparsable(format!("{e}")))?;

    // (a) The edited key took.
    match expected {
        Some(want) => match candidate_model.get(key) {
            Some(got) if got.same_value(want) => {}
            _ => {
                return Err(SelfCheckError::EditDidNotApply {
                    key: key.to_string(),
                })
            }
        },
        None => {
            if candidate_model.get(key).is_some() {
                return Err(SelfCheckError::EditDidNotApply {
                    key: key.to_string(),
                });
            }
        }
    }

    // (b) Every other modelled key is unchanged. Compare both directions: no
    // sibling value changed, none vanished, and none was added.
    check_siblings_unchanged(original_model, &candidate_model, key)?;

    // (c) Unmodelled content (comments/blank lines/unknown) preserved outside the
    // target key's own region.
    check_unmodelled_preserved(handler, original_text, candidate_text, key)?;

    Ok(())
}

/// Assert that every modelled key other than `target` is identical between the
/// original and candidate models (same set of keys, same values).
///
/// The two passes look each key up through a map built once per side, so the
/// check is O(N), not O(N^2) over [`ConfigModel::get`]'s linear scan: a large
/// config (tens of thousands of keys) is edited in linear time, not quadratic.
/// The map keeps the FIRST occurrence of a (malformed) duplicate key, matching
/// `get`'s document-order semantics; both passes iterate the entries in document
/// order so a reported collateral key is deterministic.
fn check_siblings_unchanged(
    original: &ConfigModel,
    candidate: &ConfigModel,
    target: &str,
) -> Result<(), SelfCheckError> {
    use std::collections::HashMap;

    // First-occurrence-wins index of the candidate, for O(1) sibling lookup.
    let mut candidate_index: HashMap<&str, &ConfigValue> =
        HashMap::with_capacity(candidate.len());
    for (k, v) in candidate.entries() {
        candidate_index.entry(k.as_str()).or_insert(v);
    }

    // No original sibling changed value or disappeared.
    for (k, v) in original.entries() {
        if k == target {
            continue;
        }
        match candidate_index.get(k.as_str()) {
            Some(cv) if cv.same_value(v) => {}
            _ => return Err(SelfCheckError::CollateralChange { key: k.clone() }),
        }
    }

    // No new sibling appeared in the candidate.
    let mut original_index: HashMap<&str, &ConfigValue> =
        HashMap::with_capacity(original.len());
    for (k, v) in original.entries() {
        original_index.entry(k.as_str()).or_insert(v);
    }
    for (k, _) in candidate.entries() {
        if k == target {
            continue;
        }
        if !original_index.contains_key(k.as_str()) {
            return Err(SelfCheckError::CollateralChange { key: k.clone() });
        }
    }
    Ok(())
}

/// Assert that the candidate preserved every comment, blank line, structural
/// line and unmodelled-content line the original carried, in order: nothing
/// outside the target key's own region was dropped or rewritten.
///
/// The realization is format-agnostic and span-free: a value edit only ever
/// touches the value run on the target key's own line(s), so every OTHER line of
/// the original (a comment, a blank line, a section header, an unmodelled
/// `pref(...)` line, a sibling key line) must still appear in the candidate, in
/// the same relative order. We therefore check that the original's lines, with
/// the target key's own line(s) removed, are an ORDER-PRESERVING SUBSEQUENCE of
/// the candidate's lines. A dropped or rewritten comment, a moved blank line, or
/// a clobbered neighbour breaks the subsequence and is caught.
///
/// The subsequence direction (original-minus-target ⊆ candidate) tolerates the
/// lines a legitimate insert ADDS (the new key line and, for INI, a created
/// `[section]` header), so an insert into a fresh section passes, while still
/// catching any LOSS or REWRITE of existing content. Combined with self-check
/// (b) (no sibling VALUE changed), the pair guarantees the edit was confined to
/// the target.
///
/// "The target key's own line(s)" are identified structurally: a line is
/// considered the target's own iff it carries the target key in EITHER the
/// original or the candidate. So a `set` that rewrote the target's value line is
/// not required to match (it legitimately changed), while every comment/blank/
/// neighbour line is.
fn check_unmodelled_preserved(
    handler: &dyn FormatHandler,
    original_text: &str,
    candidate_text: &str,
    target: &str,
) -> Result<(), SelfCheckError> {
    let original_kept = lines_excluding_target(handler, original_text, target);
    let candidate_kept = lines_excluding_target(handler, candidate_text, target);

    if !is_subsequence(&original_kept, &candidate_kept) {
        return Err(SelfCheckError::UnmodelledContentChanged);
    }
    Ok(())
}

/// The document's lines (verbatim, no trailing newline per line) with the lines
/// that belong to the target key removed. A line "belongs to the target" iff,
/// when the handler reads a single-line document made of just that line, the
/// target key is the modelled key it yields. This is robust across formats: it
/// identifies the target's own value-bearing line(s) without needing byte spans,
/// so a value rewrite of the target is excluded from the comparison while every
/// comment / blank / neighbour line is retained.
fn lines_excluding_target(handler: &dyn FormatHandler, text: &str, target: &str) -> Vec<String> {
    // Empty text is ZERO lines, matching the line model's own split (which yields
    // no lines for ""). Without this, `"".split('\n')` yields a phantom `[""]`
    // that is not a subsequence of a candidate carrying the first inserted key, so
    // inserting the very first key into an empty config would wrongly fail closed.
    if text.is_empty() {
        return Vec::new();
    }
    text.split('\n')
        .filter(|line| !line_is_target(handler, line, target))
        .map(|l| l.to_string())
        .collect()
}

/// Whether a single physical line carries the target key as its modelled key.
/// Reads the line in isolation through the handler; if the sole modelled key is
/// `target`, the line is the target's own. A comment/blank/section/unmodelled
/// line yields no modelled key and is therefore never the target's.
///
/// Reading a line in isolation can mis-handle a sectioned format (an INI key line
/// alone loses its `[section]` prefix, so its modelled key would be the bare
/// local key, not `section.key`). To stay correct there, the match also accepts
/// the line whose sole modelled local key equals the target's last dotted
/// segment AND the line is not itself a section header or comment. This keeps the
/// exclusion precise for INI without a full re-parse-with-context.
fn line_is_target(handler: &dyn FormatHandler, line: &str, target: &str) -> bool {
    let model = match handler.read(line) {
        Ok(m) => m,
        Err(_) => return false,
    };
    if model.len() != 1 {
        return false;
    }
    let only = &model.entries()[0].0;
    if only == target {
        return true;
    }
    // Section-stripped match: an INI key line read alone yields the bare local
    // key; accept it when it equals the target's last segment.
    //
    // KNOWN LIMITATION (latent, no shipped handler hits it): this bare-last-segment
    // match also excludes a sibling key in a DIFFERENT section that shares the
    // target's last segment (target `a.x` excludes a `[b] x = ...` line). A handler
    // that reformatted ONLY that sibling's comment/whitespace while leaving its
    // value intact would escape this (c) check. Assertion (b) still catches any
    // sibling VALUE change, so the gap is confined to a value-preserving cosmetic
    // rewrite of a same-last-segment cross-section sibling. The precise fix is
    // section-context line identification (track the section while scanning, match
    // the full `section.local` path); the built-in line-model handlers never
    // rewrite a sibling line, so it is deferred rather than risk the safety code.
    match target.rsplit_once('.') {
        Some((_, last)) => only == last,
        None => false,
    }
}

/// Whether `needle` is an order-preserving subsequence of `haystack` (every
/// element of `needle` appears in `haystack` in the same relative order, not
/// necessarily contiguously).
fn is_subsequence(needle: &[String], haystack: &[String]) -> bool {
    let mut it = haystack.iter();
    needle.iter().all(|n| it.any(|h| h == n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ParseError;
    use crate::Format;

    /// A handler with a known trivial parser - `k = v` lines are String entries,
    /// `#`/blank lines are unmodelled - whose edit primitives return a CANDIDATE
    /// the test injects verbatim, so the self-check runs against fully-controlled
    /// candidate text (a faithful edit, or a deliberately lossy one).
    struct MockHandler {
        candidate: String,
    }
    impl MockHandler {
        fn returning(candidate: &str) -> Self {
            Self { candidate: candidate.to_string() }
        }
    }
    impl FormatHandler for MockHandler {
        fn format(&self) -> Format {
            Format::Flat
        }
        fn read(&self, text: &str) -> Result<ConfigModel, ParseError> {
            let mut entries = Vec::new();
            for line in text.split('\n') {
                let t = line.trim();
                if t.is_empty() || t.starts_with('#') {
                    continue;
                }
                if let Some((k, v)) = t.split_once('=') {
                    entries.push((k.trim().to_string(), ConfigValue::String(v.trim().to_string())));
                }
            }
            Ok(ConfigModel::from_entries(entries))
        }
        fn set(&self, _text: &str, _key: &str, _value: &ConfigValue) -> Result<String, EditError> {
            Ok(self.candidate.clone())
        }
        fn remove(&self, _text: &str, _key: &str) -> Result<String, EditError> {
            Ok(self.candidate.clone())
        }
    }

    const ORIGINAL: &str = "# top comment\ngreeting = hello\nname = world";

    fn s(v: &str) -> ConfigValue {
        ConfigValue::String(v.to_string())
    }

    #[test]
    fn a_faithful_set_passes_the_self_check() {
        let h = MockHandler::returning("# top comment\ngreeting = hi\nname = world");
        let out = checked_set(&h, ORIGINAL, "greeting", &s("hi")).unwrap();
        assert!(out.contains("greeting = hi"));
    }

    #[test]
    fn a_set_that_did_not_apply_is_rejected() {
        // (a): the candidate still holds the old value.
        let h = MockHandler::returning(ORIGINAL);
        let err = checked_set(&h, ORIGINAL, "greeting", &s("hi")).unwrap_err();
        assert!(matches!(
            err,
            EditError::SelfCheck(SelfCheckError::EditDidNotApply { .. })
        ));
    }

    #[test]
    fn a_set_that_clobbers_a_sibling_is_rejected() {
        // (b): the candidate changed `name`.
        let h = MockHandler::returning("# top comment\ngreeting = hi\nname = CLOBBERED");
        let err = checked_set(&h, ORIGINAL, "greeting", &s("hi")).unwrap_err();
        assert!(matches!(
            err,
            EditError::SelfCheck(SelfCheckError::CollateralChange { .. })
        ));
    }

    #[test]
    fn a_set_that_adds_a_sibling_is_rejected() {
        // (b): the candidate introduced a new key.
        let h = MockHandler::returning("# top comment\ngreeting = hi\nname = world\nextra = sneaked");
        let err = checked_set(&h, ORIGINAL, "greeting", &s("hi")).unwrap_err();
        assert!(matches!(
            err,
            EditError::SelfCheck(SelfCheckError::CollateralChange { .. })
        ));
    }

    #[test]
    fn a_set_that_drops_a_comment_is_rejected() {
        // (c): the candidate dropped the unmodelled comment line.
        let h = MockHandler::returning("greeting = hi\nname = world");
        let err = checked_set(&h, ORIGINAL, "greeting", &s("hi")).unwrap_err();
        assert!(matches!(
            err,
            EditError::SelfCheck(SelfCheckError::UnmodelledContentChanged)
        ));
    }

    #[test]
    fn a_faithful_remove_passes_and_an_incomplete_one_is_rejected() {
        let good = MockHandler::returning("# top comment\nname = world");
        let out = checked_remove(&good, ORIGINAL, "greeting").unwrap();
        assert!(!out.contains("greeting"));

        // The candidate still carries the key, so the remove did not take.
        let bad = MockHandler::returning(ORIGINAL);
        let err = checked_remove(&bad, ORIGINAL, "greeting").unwrap_err();
        assert!(matches!(
            err,
            EditError::SelfCheck(SelfCheckError::EditDidNotApply { .. })
        ));
    }

    /// A handler whose read reports the target as a non-scalar [`ConfigValue::Opaque`].
    struct OpaqueHandler;
    impl FormatHandler for OpaqueHandler {
        fn format(&self) -> Format {
            Format::Flat
        }
        fn read(&self, _text: &str) -> Result<ConfigModel, ParseError> {
            Ok(ConfigModel::from_entries(vec![(
                "greeting".to_string(),
                ConfigValue::Opaque,
            )]))
        }
        fn set(&self, _t: &str, _k: &str, _v: &ConfigValue) -> Result<String, EditError> {
            Ok(String::new())
        }
        fn remove(&self, _t: &str, _k: &str) -> Result<String, EditError> {
            Ok(String::new())
        }
    }

    #[test]
    fn an_opaque_target_is_refused_before_editing() {
        // A structured/array existing value is unsettable: refused up front, so a
        // structure can never be flattened into a scalar.
        let err = checked_set(&OpaqueHandler, "greeting = x", "greeting", &s("hi")).unwrap_err();
        assert!(matches!(err, EditError::OpaqueTarget { .. }));
    }

    #[test]
    fn is_subsequence_respects_order_and_multiplicity() {
        let v = |xs: &[&str]| xs.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert!(is_subsequence(&[], &v(&["a", "b"])), "empty needle is a subsequence");
        assert!(is_subsequence(&v(&["a", "c"]), &v(&["a", "b", "c"])), "non-contiguous");
        assert!(is_subsequence(&v(&["a", "b"]), &v(&["a", "b"])), "exact");
        assert!(!is_subsequence(&v(&["b", "a"]), &v(&["a", "b"])), "order matters");
        assert!(!is_subsequence(&v(&["a", "a"]), &v(&["a"])), "multiplicity matters");
        assert!(!is_subsequence(&v(&["x"]), &v(&["a", "b"])), "missing element");
    }
}
