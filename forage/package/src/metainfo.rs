//! Synthesize an AppStream MetaInfo `<component>` from a recipe's metadata.
//!
//! Store-app.md ST-1: forage apps must land in the same composed AppStream catalog
//! as Flatpak and apt, so the store renders the same metadata (name, summary,
//! description, screenshots) for all three. When the upstream project ships its own
//! `metainfo.xml` the forage pipeline harvests that (upstream-first); this is the
//! INLINE FALLBACK - a MetaInfo file synthesized from the recipe's `[recipe]` block
//! so a forage app without upstream AppStream data still gets a store page.
//!
//! The recipe metadata is author-controlled text, so every value is XML-escaped: a
//! recipe cannot inject elements or break the document. The output is a
//! `desktop-application` component that `appstreamcli compose` accepts.

use arlen_forage_recipe::RecipeMeta;

/// The metadata_license of the GENERATED metainfo file itself (not the app's
/// license): CC0-1.0 is the AppStream convention for machine-generated metadata.
const METADATA_LICENSE: &str = "CC0-1.0";

/// Escape the five XML entities so author-controlled recipe text cannot inject
/// markup into the component document. Applied to every emitted value.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// A non-empty, trimmed optional string, or `None`.
fn present(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// Append `  <tag>escaped(value)</tag>\n`.
fn push_elem(out: &mut String, tag: &str, value: &str) {
    out.push_str("  <");
    out.push_str(tag);
    out.push('>');
    out.push_str(&xml_escape(value));
    out.push_str("</");
    out.push_str(tag);
    out.push_str(">\n");
}

/// Build an AppStream MetaInfo XML document from `meta`. The `id` (reverse-DNS,
/// validated at recipe parse) is the component id; `name` and `summary` are required
/// by AppStream, the rest are emitted only when present. The plain recipe
/// `description` is wrapped in a single `<p>` (AppStream descriptions are rich text;
/// one paragraph is the honest representation of a one-field description). Pure.
pub fn synthesize_metainfo(meta: &RecipeMeta) -> String {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<component type=\"desktop-application\">\n");

    push_elem(&mut out, "id", &meta.id);
    push_elem(&mut out, "name", &meta.name);
    if let Some(summary) = present(&meta.summary) {
        push_elem(&mut out, "summary", summary);
    }
    if let Some(desc) = present(&meta.description) {
        out.push_str("  <description><p>");
        out.push_str(&xml_escape(desc));
        out.push_str("</p></description>\n");
    }

    push_elem(&mut out, "metadata_license", METADATA_LICENSE);
    if let Some(license) = present(&meta.license) {
        push_elem(&mut out, "project_license", license);
    }
    if let Some(homepage) = present(&meta.homepage) {
        out.push_str("  <url type=\"homepage\">");
        out.push_str(&xml_escape(homepage));
        out.push_str("</url>\n");
    }

    if !meta.category.is_empty() {
        out.push_str("  <categories>\n");
        for category in &meta.category {
            out.push_str("    <category>");
            out.push_str(&xml_escape(category));
            out.push_str("</category>\n");
        }
        out.push_str("  </categories>\n");
    }

    if !meta.screenshots.is_empty() {
        out.push_str("  <screenshots>\n");
        for (i, shot) in meta.screenshots.iter().enumerate() {
            // The first screenshot is the default the store leads the page with.
            let attr = if i == 0 { " type=\"default\"" } else { "" };
            out.push_str("    <screenshot");
            out.push_str(attr);
            out.push_str(">\n      <image>");
            out.push_str(&xml_escape(shot));
            out.push_str("</image>\n    </screenshot>\n");
        }
        out.push_str("  </screenshots>\n");
    }

    out.push_str("</component>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> RecipeMeta {
        // Parse a full [recipe] block so the test tracks the real schema shape.
        let recipe = arlen_forage_recipe::parse(
            r#"
[recipe]
id = "org.example.hello"
name = "Hello"
maintainer = "key:abc"
summary = "a greeter"
description = "Greets you warmly."
license = "MIT"
homepage = "https://example.org"
category = ["Utility", "Office"]
screenshots = ["https://example.org/a.png", "https://example.org/b.png"]

[[source]]
type = "git"
url = "https://github.com/example/hello"
commit = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
"#,
        )
        .expect("parses");
        recipe.recipe
    }

    #[test]
    fn synthesizes_a_full_component() {
        let xml = synthesize_metainfo(&meta());
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<component type=\"desktop-application\">"));
        assert!(xml.contains("<id>org.example.hello</id>"));
        assert!(xml.contains("<name>Hello</name>"));
        assert!(xml.contains("<summary>a greeter</summary>"));
        assert!(xml.contains("<description><p>Greets you warmly.</p></description>"));
        assert!(xml.contains("<metadata_license>CC0-1.0</metadata_license>"));
        assert!(xml.contains("<project_license>MIT</project_license>"));
        assert!(xml.contains("<url type=\"homepage\">https://example.org</url>"));
        assert!(xml.contains("<category>Utility</category>"));
        // The first screenshot is the default.
        assert!(xml.contains("<screenshot type=\"default\">\n      <image>https://example.org/a.png</image>"));
        assert!(xml.contains("<screenshot>\n      <image>https://example.org/b.png</image>"));
        assert!(xml.trim_end().ends_with("</component>"));
    }

    #[test]
    fn omits_absent_optional_fields() {
        let recipe = arlen_forage_recipe::parse(
            r#"
[recipe]
id = "org.example.bare"
name = "Bare"
maintainer = "key:abc"

[[source]]
type = "git"
url = "https://github.com/example/bare"
commit = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
"#,
        )
        .expect("parses");
        let xml = synthesize_metainfo(&recipe.recipe);
        assert!(xml.contains("<id>org.example.bare</id>"));
        assert!(xml.contains("<name>Bare</name>"));
        // No summary/description/screenshots/categories/homepage when absent.
        assert!(!xml.contains("<summary>"));
        assert!(!xml.contains("<description>"));
        assert!(!xml.contains("<screenshots>"));
        assert!(!xml.contains("<categories>"));
        assert!(!xml.contains("<url"));
        // metadata_license is always present (the generated file's own license).
        assert!(xml.contains("<metadata_license>CC0-1.0</metadata_license>"));
    }

    #[test]
    fn author_text_cannot_inject_markup() {
        let mut m = meta();
        m.name = "Evil</name><release version=\"9\"/><name>".to_string();
        m.summary = Some("a & b < c > d".to_string());
        let xml = synthesize_metainfo(&m);
        // The injected element is escaped, not emitted as markup.
        assert!(!xml.contains("<release"));
        assert!(xml.contains("Evil&lt;/name&gt;&lt;release version=&quot;9&quot;/&gt;&lt;name&gt;"));
        assert!(xml.contains("<summary>a &amp; b &lt; c &gt; d</summary>"));
    }
}
