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

use arlen_forage_recipe::{Artifacts, RecipeMeta};
use std::path::{Path, PathBuf};

/// The metadata_license of the GENERATED metainfo file itself (not the app's
/// license): CC0-1.0 is the AppStream convention for machine-generated metadata.
const METADATA_LICENSE: &str = "CC0-1.0";

/// Escape the five XML entities so author-controlled recipe text cannot inject
/// markup into the component document. Applied to every emitted value. Characters
/// that XML 1.0 forbids even when escaped (the C0 controls except tab/newline/CR) are
/// DROPPED, so a stray control byte in author text cannot make the whole metainfo
/// document unparseable (which would fail `appstreamcli compose`).
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // XML 1.0 valid chars: tab, LF, CR, then >= 0x20. Drop every other
            // control (0x00-0x08, 0x0B, 0x0C, 0x0E-0x1F) - unrepresentable in XML.
            c if (c.is_control() && c != '\t' && c != '\n' && c != '\r') => {}
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

/// The AppStream component type for a package, from what it installs: a `.desktop`
/// entry makes it a `desktop-application`; otherwise an installed binary makes it a
/// `console-application`. A package with neither (a library/data-only package) falls
/// back to `desktop-application`, the store's primary browse type. Pure.
pub fn component_type(artifacts: Option<&Artifacts>) -> &'static str {
    match artifacts {
        Some(a) if a.desktop.is_some() => "desktop-application",
        Some(a) if !a.bin.is_empty() => "console-application",
        _ => "desktop-application",
    }
}

/// Build an AppStream MetaInfo XML document from `meta`, typing the component from
/// what the package installs (`artifacts`; see [`component_type`]). The `id`
/// (reverse-DNS, validated at recipe parse) is the component id; `name` and `summary`
/// are required by AppStream, the rest are emitted only when present. The plain
/// recipe `description` is wrapped in a single `<p>` (AppStream descriptions are rich
/// text; one paragraph is the honest representation of a one-field description). Pure.
pub fn synthesize_metainfo(meta: &RecipeMeta, artifacts: Option<&Artifacts>) -> String {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<component type=\"");
    out.push_str(component_type(artifacts));
    out.push_str("\">\n");

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

/// Whether `path` sits under a directory named `dir` (a path component match, so
/// `share/metainfo/x.xml` matches `metainfo` but `metainfo-notes/x.xml` does not).
fn under_dir(path: &Path, dir: &str) -> bool {
    path.components()
        .any(|c| c.as_os_str().to_str() == Some(dir))
}

/// Find the upstream AppStream metadata file among a fetched source's relative
/// paths, per the freedesktop convention (ST-1 upstream-first: harvest the project's
/// own metainfo when it ships one, and only [`synthesize_metainfo`] as the fallback).
/// Preference, most-canonical first: a `*.metainfo.xml` under a `metainfo/` dir, then
/// any `*.metainfo.xml`, then a legacy `*.appdata.xml` under an `appdata/`/`metainfo/`
/// dir, then any `*.appdata.xml`. Within a tier the lexicographically smallest path
/// wins, so the choice is deterministic. `None` when the source ships none. Pure -
/// the directory walk that produces `paths` is the caller's thin I/O.
pub fn find_upstream_metainfo(paths: &[PathBuf]) -> Option<PathBuf> {
    let ends = |p: &Path, suffix: &str| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(suffix))
    };
    // (tier, path) - lower tier is more canonical.
    let tier = |p: &Path| -> Option<u8> {
        if ends(p, ".metainfo.xml") {
            Some(if under_dir(p, "metainfo") { 0 } else { 1 })
        } else if ends(p, ".appdata.xml") {
            Some(if under_dir(p, "appdata") || under_dir(p, "metainfo") {
                2
            } else {
                3
            })
        } else {
            None
        }
    };
    paths
        .iter()
        .filter_map(|p| tier(p).map(|t| (t, p)))
        .min_by(|(ta, pa), (tb, pb)| ta.cmp(tb).then_with(|| pa.cmp(pb)))
        .map(|(_, p)| p.clone())
}

/// The in-package directory AppStream metadata lives in. A forage package uses the
/// prefix-less `share/` layout installd installs (it copies top-level `bin/`/`lib/`/
/// `share/`), and `appstreamcli compose` over the package prefix reads
/// `<prefix>/share/metainfo/*.xml`, so the metainfo belongs at `share/metainfo/` -
/// NOT `usr/share/metainfo/`, which would not match the package layout.
const METAINFO_DIR: &str = "share/metainfo";

/// Write the synthesized metainfo for `meta` into the staging root at
/// `usr/share/metainfo/<id>.metainfo.xml`, creating the directory, and return the
/// written path. The forage pipeline calls this after collecting artifacts so the
/// built package carries its own AppStream component (the inline fallback; a harvested
/// upstream metainfo is preferred when one is found via [`find_upstream_metainfo`]).
/// `id` is reverse-DNS (validated at recipe parse), so it is a safe bare filename
/// with no path separators.
pub fn write_metainfo(
    staging_root: &Path,
    meta: &RecipeMeta,
    artifacts: Option<&Artifacts>,
) -> std::io::Result<PathBuf> {
    let dir = staging_root.join(METAINFO_DIR);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.metainfo.xml", meta.id));
    std::fs::write(&path, synthesize_metainfo(meta, artifacts))?;
    Ok(path)
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
        let xml = synthesize_metainfo(&meta(), None);
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
        let xml = synthesize_metainfo(&recipe.recipe, None);
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
    fn find_upstream_metainfo_prefers_the_canonical_location() {
        let p = |s: &str| PathBuf::from(s);
        // Canonical metainfo/ location wins over a stray top-level one.
        let paths = vec![
            p("README.md"),
            p("stray.metainfo.xml"),
            p("share/metainfo/org.example.hello.metainfo.xml"),
        ];
        assert_eq!(
            find_upstream_metainfo(&paths),
            Some(p("share/metainfo/org.example.hello.metainfo.xml"))
        );
        // metainfo.xml beats a legacy appdata.xml even when the appdata is canonical.
        let mixed = vec![
            p("data/appdata/org.example.hello.appdata.xml"),
            p("x.metainfo.xml"),
        ];
        assert_eq!(find_upstream_metainfo(&mixed), Some(p("x.metainfo.xml")));
        // Legacy appdata is harvested when no metainfo exists.
        let legacy = vec![p("README.md"), p("data/org.example.hello.appdata.xml")];
        assert_eq!(
            find_upstream_metainfo(&legacy),
            Some(p("data/org.example.hello.appdata.xml"))
        );
        // No AppStream file -> None (the caller synthesizes from the recipe).
        assert_eq!(find_upstream_metainfo(&[p("README.md"), p("src/main.rs")]), None);
        // `metainfo-notes/` is not a metainfo dir (component match, not substring).
        let notlike = vec![p("metainfo-notes/x.metainfo.xml"), p("share/metainfo/y.metainfo.xml")];
        assert_eq!(
            find_upstream_metainfo(&notlike),
            Some(p("share/metainfo/y.metainfo.xml"))
        );
    }

    #[test]
    fn author_text_cannot_inject_markup() {
        let mut m = meta();
        m.name = "Evil</name><release version=\"9\"/><name>".to_string();
        m.summary = Some("a & b < c > d".to_string());
        let xml = synthesize_metainfo(&m, None);
        // The injected element is escaped, not emitted as markup.
        assert!(!xml.contains("<release"));
        assert!(xml.contains("Evil&lt;/name&gt;&lt;release version=&quot;9&quot;/&gt;&lt;name&gt;"));
        assert!(xml.contains("<summary>a &amp; b &lt; c &gt; d</summary>"));
    }

    #[test]
    fn write_metainfo_places_the_component_at_the_appstream_path() {
        let staging = tempfile::tempdir().unwrap();
        let path = write_metainfo(staging.path(), &meta(), None).expect("writes");
        assert_eq!(
            path,
            staging
                .path()
                .join("share/metainfo/org.example.hello.metainfo.xml")
        );
        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(written, synthesize_metainfo(&meta(), None));
        assert!(written.contains("<id>org.example.hello</id>"));
    }

    #[test]
    fn component_type_follows_what_the_package_installs() {
        let with_artifacts = |block: &str| {
            arlen_forage_recipe::parse(&format!(
                r#"
[recipe]
id = "org.example.hello"
name = "Hello"
maintainer = "key:abc"

[[source]]
type = "git"
url = "https://github.com/example/hello"
commit = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"

[artifacts]
{block}
"#
            ))
            .expect("parses")
        };
        let desktop = with_artifacts("bin = [\"hello\"]\ndesktop = \"hello.desktop\"");
        assert_eq!(component_type(desktop.artifacts.as_ref()), "desktop-application");
        let cli = with_artifacts("bin = [\"hello\"]");
        assert_eq!(component_type(cli.artifacts.as_ref()), "console-application");
        let lib = with_artifacts("lib = [\"libhello.so\"]");
        assert_eq!(component_type(lib.artifacts.as_ref()), "desktop-application");
        assert_eq!(component_type(None), "desktop-application");
        // The synthesized document carries the derived type.
        assert!(synthesize_metainfo(&meta(), cli.artifacts.as_ref())
            .contains("<component type=\"console-application\">"));
    }

    #[test]
    fn xml_invalid_control_chars_are_dropped_but_whitespace_kept() {
        let mut m = meta();
        // A description with a NUL, a bell (0x07), and a form-feed (0x0C) - all
        // invalid in XML 1.0 - plus a legitimate newline and tab.
        m.description = Some("line1\n\tline2\u{0}\u{7}\u{c}end".to_string());
        let xml = synthesize_metainfo(&m, None);
        assert!(xml.contains("<description><p>line1\n\tline2end</p></description>"));
        // No forbidden control byte survives into the document.
        assert!(!xml.contains('\u{0}') && !xml.contains('\u{7}') && !xml.contains('\u{c}'));
    }
}
