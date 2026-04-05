/// Waypointer Unicode character search.
///
/// Searches by codepoint (U+XXXX) or by name substring using the
/// unicode_names2 crate for the full Unicode character database.

use serde::Serialize;

/// A single Unicode character result.
#[derive(Clone, Serialize)]
pub struct UnicodeChar {
    /// Numeric codepoint.
    pub codepoint: u32,
    /// The character as a string.
    pub char_str: String,
    /// Unicode name (e.g. "BLACK HEART SUIT").
    pub name: String,
    /// Formatted codepoint (e.g. "U+2665").
    pub codepoint_hex: String,
}

/// Searches Unicode characters by name or codepoint.
///
/// Supports:
/// - Direct codepoint: "U+2764", "2764", "0x2764"
/// - Name search (case-insensitive, contains): "heart", "arrow", "smile"
/// Max 20 results, sorted by name length (shorter = more specific).
#[tauri::command]
pub fn search_unicode(query: String) -> Vec<UnicodeChar> {
    let query = query.trim();
    if query.is_empty() {
        return vec![];
    }

    // Try direct codepoint lookup first.
    if let Some(result) = lookup_codepoint(query) {
        return vec![result];
    }

    // Name search: scan common Unicode ranges for matches.
    let upper = query.to_uppercase();
    let mut results: Vec<UnicodeChar> = Vec::new();

    // Scan: Basic Latin through Supplemental Symbols.
    // Ranges chosen to cover useful characters without scanning all 1.1M codepoints.
    let ranges: &[(u32, u32)] = &[
        (0x0020, 0x007F),   // Basic Latin
        (0x00A0, 0x024F),   // Latin Extended
        (0x0370, 0x03FF),   // Greek
        (0x0400, 0x04FF),   // Cyrillic
        (0x2000, 0x27BF),   // General Punctuation through Dingbats
        (0x2900, 0x2BFF),   // Supplemental Arrows through Misc Symbols
        (0x2E00, 0x2E7F),   // Supplemental Punctuation
        (0x3000, 0x303F),   // CJK Symbols
        (0x1F300, 0x1F9FF), // Misc Symbols, Emoticons, etc.
        (0x1FA00, 0x1FAFF), // Chess, Extended-A
        (0xFE00, 0xFE0F),   // Variation Selectors
    ];

    for &(start, end) in ranges {
        if results.len() >= 20 {
            break;
        }
        for cp in start..=end {
            if results.len() >= 20 {
                break;
            }
            let Some(ch) = char::from_u32(cp) else { continue };
            let Some(name) = unicode_names2::name(ch) else { continue };
            let name_str = name.to_string();
            if name_str.to_uppercase().contains(&upper) {
                results.push(make_result(cp, ch, name_str));
            }
        }
    }

    // Sort by name length (shorter names = more specific matches).
    results.sort_by_key(|r| r.name.len());
    results
}

/// Tries to parse a codepoint from the query string.
fn lookup_codepoint(query: &str) -> Option<UnicodeChar> {
    let hex_str = if let Some(rest) = query.strip_prefix("U+").or_else(|| query.strip_prefix("u+")) {
        rest
    } else if let Some(rest) = query.strip_prefix("0x").or_else(|| query.strip_prefix("0X")) {
        rest
    } else if query.len() >= 4 && query.len() <= 6 && query.chars().all(|c| c.is_ascii_hexdigit()) {
        query
    } else {
        return None;
    };

    let cp = u32::from_str_radix(hex_str, 16).ok()?;
    let ch = char::from_u32(cp)?;
    let name = unicode_names2::name(ch)
        .map(|n| n.to_string())
        .unwrap_or_else(|| format!("U+{:04X}", cp));

    Some(make_result(cp, ch, name))
}

fn make_result(cp: u32, ch: char, name: String) -> UnicodeChar {
    UnicodeChar {
        codepoint: cp,
        char_str: ch.to_string(),
        name,
        codepoint_hex: format!("U+{:04X}", cp),
    }
}
