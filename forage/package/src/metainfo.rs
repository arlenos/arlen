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
/// entry makes it a `desktop-application`; anything else is a `console-application`.
///
/// The fallback used to be `desktop-application`, on the reasoning that it is the
/// store's primary browse type. That claims a GUI application on no evidence, and
/// the claim does not survive contact with `appstreamcli compose`: a
/// desktop-application with no desktop entry and no icon is rejected outright with
/// `gui-app-without-icon`, and the catalogue comes out empty. Checked against
/// appstreamcli 1.1.5. So the type that made the app more visible was the type that
/// kept it out of the composed catalogue entirely. A package that installs no
/// desktop entry is not a desktop application, and saying so is both honest and the
/// version that composes. Pure.
pub fn component_type(artifacts: Option<&Artifacts>) -> &'static str {
    match artifacts {
        Some(a) if a.desktop.is_some() => "desktop-application",
        _ => "console-application",
    }
}

/// Build an AppStream MetaInfo XML document from `meta`, typing the component from
/// what the package installs (`artifacts`; see [`component_type`]). The `id`
/// (reverse-DNS, validated at recipe parse) is the component id; `name` and `summary`
/// are required by AppStream, the rest are emitted only when present. The plain
/// recipe `description` is wrapped in a single `<p>` (AppStream descriptions are rich
/// text; one paragraph is the honest representation of a one-field description), and
/// falls back to the summary when the recipe wrote no description. Pure.
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
    // The summary stands in when there is no description, because AppStream requires
    // a description for a catalogue component and `appstreamcli compose` refuses the
    // whole component without one - so a recipe that wrote only a summary would
    // install and never be seen in the store. This invents nothing: the summary is
    // the maintainer's own sentence about their own package, promoted to the only
    // paragraph there is.
    if let Some(desc) = present(&meta.description).or_else(|| present(&meta.summary)) {
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

/// Whether a screenshot may be fetched and cached with the package.
///
/// The rule is `coder-jobs.md` 1e, and it is narrower than "no remote images"
/// and wider than "any URL in the metainfo":
///
/// A recipe already made you talk to the host its source comes from. Refusing
/// that host's screenshot protects nothing and costs the person the picture, so
/// an image served from a source host is fetchable. A THIRD party the recipe
/// never made you talk to is the thing the rule is against, and an image from
/// one stays a remote URL: the store can still render it if the machine has a
/// network and the person allows it, and nothing was fetched at build time on
/// that host's behalf.
///
/// Host comparison is exact and case-folded, never a suffix test: `evil-github.com`
/// ends with nothing useful, but `github.com.attacker.net` ends with the attacker's
/// own name and a suffix test on the other side would admit `notgithub.com`. An
/// image on a subdomain of a source host is a different host and is treated as one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenshotVerdict {
    /// Fetch it and cache it beside the artifact.
    Fetch(String),
    /// Leave it as a remote URL, with the host that made it third-party.
    LeaveRemote { url: String, host: String },
    /// Not a URL this can reason about at all.
    Unusable(String),
}

/// The host of an `https` URL, lower-cased, or `None` if it is not one.
///
/// Deliberately `https` only. A screenshot fetched over plain http is a picture
/// anyone on the path can replace, and a build that caches it has laundered it
/// into a local file the store then trusts.
fn https_host(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    // Credentials in a URL are not a host we can reason about, and an image URL
    // has no business carrying any.
    if authority.contains('@') || authority.is_empty() {
        return None;
    }
    let host = authority.split(':').next()?;
    if host.is_empty() {
        return None;
    }
    Some(host.to_ascii_lowercase())
}

/// Decide each screenshot against the hosts this recipe already fetches from.
pub fn screenshot_verdicts(source_urls: &[String], screenshots: &[String]) -> Vec<ScreenshotVerdict> {
    let sources: Vec<String> = source_urls.iter().filter_map(|u| https_host(u)).collect();
    screenshots
        .iter()
        .map(|shot| match https_host(shot) {
            None => ScreenshotVerdict::Unusable(shot.clone()),
            Some(host) if sources.contains(&host) => ScreenshotVerdict::Fetch(shot.clone()),
            Some(host) => ScreenshotVerdict::LeaveRemote {
                url: shot.clone(),
                host,
            },
        })
        .collect()
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
    fn a_recipe_with_only_a_summary_still_describes_itself() {
        let mut m = meta();
        m.description = None;
        let xml = synthesize_metainfo(&m, None);
        assert!(
            xml.contains("<description><p>a greeter</p></description>"),
            "without one, compose refuses the component and the app is never listed: {xml}",
        );
    }

    #[test]
    fn synthesizes_a_full_component() {
        let xml = synthesize_metainfo(&meta(), None);
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<component type=\"console-application\">"));
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
        // Neither a desktop entry nor a binary: not a desktop application, and
        // typing it as one gets it rejected by compose rather than listed.
        assert_eq!(component_type(lib.artifacts.as_ref()), "console-application");
        assert_eq!(component_type(None), "console-application");
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

#[cfg(test)]
mod screenshot_tests {
    use super::*;

    fn shots(sources: &[&str], shots: &[&str]) -> Vec<ScreenshotVerdict> {
        screenshot_verdicts(
            &sources.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            &shots.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        )
    }

    #[test]
    fn an_image_from_a_host_the_recipe_already_uses_is_fetched() {
        let v = shots(
            &["https://downloads.example.org/app-1.2.tar.gz"],
            &["https://downloads.example.org/shot.png"],
        );
        assert_eq!(v, vec![ScreenshotVerdict::Fetch("https://downloads.example.org/shot.png".into())]);
    }

    #[test]
    fn an_image_from_a_host_nobody_asked_you_to_talk_to_stays_remote() {
        let v = shots(
            &["https://downloads.example.org/app.tar.gz"],
            &["https://cdn.tracker.net/shot.png"],
        );
        assert_eq!(
            v,
            vec![ScreenshotVerdict::LeaveRemote {
                url: "https://cdn.tracker.net/shot.png".into(),
                host: "cdn.tracker.net".into(),
            }]
        );
    }

    #[test]
    fn a_subdomain_is_a_different_host() {
        // Not a suffix test in either direction: `img.example.org` is not
        // `example.org`, and `notexample.org` must not pass because it ends with
        // the same letters.
        let v = shots(
            &["https://example.org/app.tar.gz"],
            &["https://img.example.org/a.png", "https://notexample.org/b.png"],
        );
        assert!(matches!(v[0], ScreenshotVerdict::LeaveRemote { .. }));
        assert!(matches!(v[1], ScreenshotVerdict::LeaveRemote { .. }));
    }

    #[test]
    fn the_host_comparison_ignores_case_and_a_port() {
        let v = shots(
            &["https://Example.ORG:443/app.tar.gz"],
            &["https://example.org/shot.png"],
        );
        assert!(matches!(v[0], ScreenshotVerdict::Fetch(_)));
    }

    #[test]
    fn plain_http_is_never_fetched() {
        // A picture anyone on the path can replace, laundered into a local file
        // the store then treats as the package's own.
        let v = shots(&["http://example.org/app.tar.gz"], &["http://example.org/shot.png"]);
        assert_eq!(v, vec![ScreenshotVerdict::Unusable("http://example.org/shot.png".into())]);
    }

    #[test]
    fn a_url_carrying_credentials_is_not_a_host_we_reason_about() {
        let v = shots(&["https://example.org/a.tar.gz"], &["https://u:p@example.org/s.png"]);
        assert!(matches!(v[0], ScreenshotVerdict::Unusable(_)));
    }

    #[test]
    fn a_recipe_with_no_https_source_fetches_nothing() {
        // A personal cookbook pointing at a git+ssh remote, say. Nothing is
        // fetched on its behalf and every image stays where it is.
        let v = shots(&["git@codeberg.org:me/app.git"], &["https://codeberg.org/shot.png"]);
        assert!(matches!(v[0], ScreenshotVerdict::LeaveRemote { .. }));
    }
}
