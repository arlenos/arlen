//! Faceted Knowledge-Graph query construction for the FM filter bar
//! (`file-manager-plan.md`, the `facet:` virtual location). arlen-ui built the
//! filter bar and serializes a multi-group selection into a `facet:` location;
//! this is the read path that makes the result real.
//!
//! A selection has four groups - Project / Type / Time / Touched. Groups AND
//! together (a file must satisfy every active group); values within a group OR
//! (any-of). Project and Touched constrain graph relationships
//! (`FILE_PART_OF` to a project, `ACCESSED_BY` an app), so their ids are
//! interpolated and therefore escaped. Type and Time are intrinsic file
//! attributes mapped to HARDCODED Cypher clauses keyed by a CLOSED value set, so
//! no caller value is ever interpolated for them and an unknown value is
//! fail-closed (a present-but-unsatisfiable group makes the whole query match
//! nothing, never broaden it). Pure - the live read + row mapping stay in the
//! command - so the construction is unit-tested, including injection attempts.

/// One micro-second day, the unit the recency cutoffs are built from.
const DAY_MICROS: i64 = 86_400 * 1_000_000;

/// A parsed facet selection. Project/Type/Touched are multi-select (OR within
/// the group); Time is a single recency cutoff.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct FacetSelection {
    /// Selected project ids (graph ids; `FILE_PART_OF` membership).
    pub projects: Vec<String>,
    /// Selected type kinds (closed set: document/image/audio/video/archive/code).
    pub types: Vec<String>,
    /// The single recency cutoff key (day/week/month/older), if set.
    pub time: Option<String>,
    /// Selected app ids that touched the file (`ACCESSED_BY`).
    pub touched: Vec<String>,
}

/// Escape a value for a single-quoted Cypher literal (backslash then quote),
/// matching the FM's other interpolated reads.
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

/// The file extensions (lower-case, without the dot) a type kind matches. An
/// unknown kind yields an empty slice, which the builder treats fail-closed.
fn type_extensions(kind: &str) -> &'static [&'static str] {
    match kind {
        "document" => &[
            "pdf", "doc", "docx", "odt", "txt", "md", "rtf", "ppt", "pptx", "xls", "xlsx", "ods",
            "odp",
        ],
        "image" => &["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "tiff", "heic", "avif"],
        "audio" => &["mp3", "flac", "ogg", "wav", "m4a", "aac", "opus"],
        "video" => &["mp4", "mkv", "webm", "mov", "avi", "wmv"],
        "archive" => &["zip", "tar", "gz", "bz2", "xz", "zst", "7z", "rar"],
        "code" => &[
            "rs", "py", "js", "ts", "go", "c", "cpp", "h", "hpp", "java", "rb", "sh", "toml",
            "json", "yaml", "yml", "html", "css",
        ],
        _ => &[],
    }
}

/// The `f.last_accessed` predicate for a recency cutoff key, relative to
/// `now_micros`. `older` is the complement (strictly before the 30-day cutoff);
/// an unknown key yields `None` (fail-closed at the caller).
fn time_predicate(key: &str, now_micros: i64) -> Option<String> {
    let cutoff = |days: i64| now_micros - days * DAY_MICROS;
    let p = match key {
        "day" => format!("f.last_accessed >= {}", cutoff(1)),
        "week" => format!("f.last_accessed >= {}", cutoff(7)),
        "month" => format!("f.last_accessed >= {}", cutoff(30)),
        "older" => format!("f.last_accessed < {}", cutoff(30)),
        _ => return None,
    };
    Some(p)
}

/// Parse a `facet:` location into a [`FacetSelection`]. Mirrors the frontend
/// `serializeFacets`: `facet:project=a,b;type=document;time=week;touched=x`.
/// Unknown groups and empty values are skipped; Time keeps only its first value
/// (single-select). A location without the `facet:` prefix yields an empty
/// selection.
pub fn parse_facet_location(location: &str) -> FacetSelection {
    let mut sel = FacetSelection::default();
    let Some(body) = location.strip_prefix("facet:") else {
        return sel;
    };
    for part in body.split(';') {
        let Some((group, csv)) = part.split_once('=') else {
            continue;
        };
        let values: Vec<String> = csv.split(',').filter(|v| !v.is_empty()).map(str::to_string).collect();
        if values.is_empty() {
            continue;
        }
        match group {
            "project" => sel.projects = values,
            "type" => sel.types = values,
            "touched" => sel.touched = values,
            "time" => sel.time = values.into_iter().next(),
            _ => {}
        }
    }
    sel
}

/// Build the Cypher listing the File nodes matching `sel`, as of `now_micros`
/// for the recency cutoff. Returns `None` for an empty selection (the caller
/// lists the real folder instead). Groups AND; values OR. A present group that
/// resolves to no valid predicate (e.g. an unknown type kind, an unparseable
/// time key, or only-empty ids) injects `false`, so the query is fail-closed
/// (matches nothing) rather than silently dropping the constraint. Mirrors the
/// `project_members` row shape (`path` + `accessed`) so the existing
/// `members_from_rows` maps it.
pub fn facet_cypher(sel: &FacetSelection, now_micros: i64) -> Option<String> {
    if sel.projects.is_empty() && sel.types.is_empty() && sel.time.is_none() && sel.touched.is_empty()
    {
        return None;
    }

    // Relationship patterns (Project membership, App access) joined on `f`.
    let mut patterns = vec!["(f:File)".to_string()];
    if !sel.projects.is_empty() {
        patterns.push("(f)-[:FILE_PART_OF]->(p:Project)".to_string());
    }
    if !sel.touched.is_empty() {
        patterns.push("(f)-[:ACCESSED_BY]->(a:App)".to_string());
    }

    let mut wheres: Vec<String> = Vec::new();

    // Project ids: OR within the group, escaped. `false` if all are empty.
    if !sel.projects.is_empty() {
        wheres.push(id_in_clause("p.id", &sel.projects));
    }
    if !sel.touched.is_empty() {
        wheres.push(id_in_clause("a.id", &sel.touched));
    }

    // Type: OR of extension suffixes from the closed kind set (hardcoded, no
    // interpolation of caller values). A type group with no valid kind is `false`.
    if !sel.types.is_empty() {
        let exts: Vec<String> = sel
            .types
            .iter()
            .flat_map(|k| type_extensions(k).iter())
            .map(|ext| format!("f.path ENDS WITH '.{ext}'"))
            .collect();
        wheres.push(or_group(exts));
    }

    // Time: a single recency cutoff. An unknown key is `false`.
    if let Some(key) = &sel.time {
        wheres.push(time_predicate(key, now_micros).unwrap_or_else(|| "false".to_string()));
    }

    let where_clause = wheres.join(" AND ");
    Some(format!(
        "MATCH {} WHERE {} RETURN f.path AS path, f.last_accessed AS accessed \
         ORDER BY f.path LIMIT 512",
        patterns.join(", "),
        where_clause
    ))
}

/// An `<field> IN ['a','b']` clause over escaped ids, or `false` when every id
/// is empty (fail-closed rather than an empty `IN []` that matches nothing
/// ambiguously).
fn id_in_clause(field: &str, ids: &[String]) -> String {
    let escaped: Vec<String> = ids
        .iter()
        .filter(|s| !s.is_empty())
        .map(|s| format!("'{}'", escape(s)))
        .collect();
    if escaped.is_empty() {
        return "false".to_string();
    }
    format!("{field} IN [{}]", escaped.join(", "))
}

/// Join predicates with OR inside parentheses, or `false` when there are none.
fn or_group(preds: Vec<String>) -> String {
    if preds.is_empty() {
        "false".to_string()
    } else {
        format!("({})", preds.join(" OR "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000_000_000;

    #[test]
    fn parses_every_group_and_keeps_time_single() {
        let sel = parse_facet_location("facet:project=p1,p2;type=document;time=week,day;touched=app1");
        assert_eq!(sel.projects, vec!["p1", "p2"]);
        assert_eq!(sel.types, vec!["document"]);
        assert_eq!(sel.time.as_deref(), Some("week")); // single-select keeps the first
        assert_eq!(sel.touched, vec!["app1"]);
    }

    #[test]
    fn a_non_facet_or_empty_location_is_an_empty_selection() {
        assert_eq!(parse_facet_location("/home/tim"), FacetSelection::default());
        assert_eq!(parse_facet_location("facet:"), FacetSelection::default());
        assert!(facet_cypher(&FacetSelection::default(), NOW).is_none());
    }

    #[test]
    fn project_and_touched_join_on_the_file_with_escaped_ids() {
        let sel = parse_facet_location("facet:project=Thesis;touched=org.x.App");
        let cy = facet_cypher(&sel, NOW).unwrap();
        assert!(cy.contains("(f)-[:FILE_PART_OF]->(p:Project)"));
        assert!(cy.contains("(f)-[:ACCESSED_BY]->(a:App)"));
        assert!(cy.contains("p.id IN ['Thesis']"));
        assert!(cy.contains("a.id IN ['org.x.App']"));
        assert!(cy.contains(" AND "));
        assert!(cy.ends_with("ORDER BY f.path LIMIT 512"));
    }

    #[test]
    fn type_maps_to_a_closed_extension_or_group_and_never_interpolates() {
        let cy = facet_cypher(&parse_facet_location("facet:type=image"), NOW).unwrap();
        assert!(cy.contains("f.path ENDS WITH '.png'"));
        assert!(cy.contains("f.path ENDS WITH '.svg'"));
        assert!(cy.contains(" OR "));
        // An unknown kind is fail-closed (matches nothing), never unfiltered.
        let bogus = facet_cypher(&parse_facet_location("facet:type=malware"), NOW).unwrap();
        assert!(bogus.contains("WHERE false"));
    }

    #[test]
    fn time_is_a_recency_cutoff_and_older_is_the_complement() {
        let week = facet_cypher(&parse_facet_location("facet:time=week"), NOW).unwrap();
        assert!(week.contains(&format!("f.last_accessed >= {}", NOW - 7 * DAY_MICROS)));
        let older = facet_cypher(&parse_facet_location("facet:time=older"), NOW).unwrap();
        assert!(older.contains(&format!("f.last_accessed < {}", NOW - 30 * DAY_MICROS)));
    }

    #[test]
    fn a_cypher_injection_in_a_project_id_is_escaped_not_executed() {
        // A crafted facet: location cannot break out of the literal.
        let sel = parse_facet_location("facet:project=x'] RETURN 1 //");
        let cy = facet_cypher(&sel, NOW).unwrap();
        // The quote is escaped; no raw `'] RETURN` breaks the string.
        assert!(cy.contains(r"x\'] RETURN 1 //"));
        assert!(!cy.contains("['x'] RETURN 1"));
    }
}
