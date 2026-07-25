//! The catalog compose step (store-app.md section 9.3): map each source's app
//! records into [`CatalogEntry`]s the merge consumes. This module holds the forage
//! adapter (Arlen-native, the recipe schema is local); the Flathub AppStream-XML and
//! Debian DEP-11-YAML readers land alongside it as they are built.
//!
//! Pure mapping, no I/O: given an already-parsed forage `Recipe` and the layer its
//! cookbook resolves to (personal/community/official), produce one entry. The recipe
//! carries the same AppStream metadata a client renders (`recipe.md` ST-1), so a
//! forage app is a first-class catalog citizen, not a second-class listing.

use arlen_forage_recipe::{Capabilities, Recipe, ReproducibleStatus};

use crate::catalog::{
    CapabilityFootprint, CatalogEntry, ComponentId, DisplayMeta, SourceLayer, TrustSignals,
};

/// Map a parsed forage recipe to one catalog entry for the given source layer (the
/// tier its cookbook resolved to). Display comes from the recipe `[recipe]` metadata
/// (the same AppStream fields Flatpak/apt carry); the capability footprint is the
/// coarse categories the `[capabilities]` block declares (so a "needs network" /
/// "offline" facet is meaningful); the reproducible-build trust signal is populated
/// only when the recipe attests one (an unchecked status hides the row, section 9.2).
pub fn forage_entry(recipe: &Recipe, layer: SourceLayer) -> CatalogEntry {
    let meta = &recipe.recipe;
    let display = DisplayMeta {
        name: meta.name.clone(),
        summary: meta.summary.clone(),
        description: meta.description.clone(),
        screenshots: meta.screenshots.clone(),
        // A recipe declares no icon reference; the client falls back to a default.
        icon: None,
    };
    let capabilities = CapabilityFootprint {
        // The tier BADGE is a trust property of the cookbook, not the recipe; the
        // caller sets it from the cookbook's verification, so it stays None here.
        tier: None,
        capabilities: recipe.capabilities.as_ref().map(capability_labels).unwrap_or_default(),
    };
    let trust = TrustSignals {
        verified_publisher: None,
        reproducible_build: recipe.reproducible.as_ref().and_then(|r| match r.status {
            ReproducibleStatus::Verified => Some("verified".to_string()),
            ReproducibleStatus::Expected => Some("expected".to_string()),
            ReproducibleStatus::Unreproducible => Some("unreproducible".to_string()),
            // Not yet checked: hide the row rather than assert a status.
            ReproducibleStatus::Unverified => None,
        }),
        install_count: None,
        odrs_score: None,
        observed_vs_declared: None,
    };
    CatalogEntry { id: ComponentId(meta.id.clone()), layer, display, capabilities, trust }
}

/// The coarse capability categories a recipe declares, as sorted, deduped display
/// labels: a `network`/`filesystem`/`notifications`/`clipboard`/`audio` token when
/// the app requests that category, each graph scope verbatim (`read:File`), and any
/// extra category key. Coarse on purpose - the store facets on categories; the
/// concrete hosts/paths are an app-detail concern.
fn capability_labels(caps: &Capabilities) -> Vec<String> {
    let mut labels = Vec::new();
    if !caps.network.is_empty() {
        labels.push("network".to_string());
    }
    if !caps.filesystem.is_empty() {
        labels.push("filesystem".to_string());
    }
    if caps.notifications {
        labels.push("notifications".to_string());
    }
    if caps.clipboard {
        labels.push("clipboard".to_string());
    }
    if caps.audio {
        labels.push("audio".to_string());
    }
    for scope in &caps.graph {
        labels.push(scope.clone());
    }
    for key in caps.extra.keys() {
        labels.push(key.clone());
    }
    labels.sort();
    labels.dedup();
    labels
}

/// Why a composed catalog could not be read.
#[derive(Debug)]
pub enum ComposeError {
    /// The XML did not parse (Flathub reader). XML is one atomic document, so a
    /// malformed catalog cannot be partially read; the DEP-11 reader, a multi-
    /// document stream, instead skips a single bad record (it is best-effort, not
    /// `Result`).
    Xml(String),
}

/// Parse a Flathub composed-AppStream catalog (`<components>` of `<component>`) into
/// one `CatalogEntry` per desktop app (`layer = Flatpak`). Display comes from the
/// AppStream fields (the UNLOCALIZED default element, ignoring `xml:lang` variants);
/// the capability footprint (Flatpak `finish-args`) and the trust signals (Flathub
/// verification / stats, ODRS) come from SEPARATE sources per section 9.2 and stay
/// empty here. A `<component>` with no id is skipped, not guessed.
pub fn flathub_entries(xml: &str) -> Result<Vec<CatalogEntry>, ComposeError> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| ComposeError::Xml(e.to_string()))?;
    let mut entries = Vec::new();
    for component in doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "component")
    {
        let Some(id) = child_text(&component, "id") else {
            continue; // A component with no id is unusable; skip it.
        };
        let name = default_localized(&component, "name").unwrap_or_default();
        let display = DisplayMeta {
            name,
            summary: default_localized(&component, "summary"),
            description: description_text(&component),
            screenshots: screenshot_urls(&component),
            icon: icon_ref(&component),
        };
        entries.push(CatalogEntry {
            id: ComponentId(id),
            layer: SourceLayer::Flatpak,
            display,
            capabilities: CapabilityFootprint::default(),
            trust: TrustSignals::default(),
        });
    }
    Ok(entries)
}

/// The text of the first direct child element named `tag`.
fn child_text<'a>(node: &roxmltree::Node<'a, 'a>, tag: &str) -> Option<String> {
    node.children()
        .find(|c| c.is_element() && c.tag_name().name() == tag)
        .and_then(|c| c.text())
        .map(str::to_string)
}

/// The text of the unlocalized `tag` child (the one without an `xml:lang`), the
/// default the store renders; localized variants are ignored.
fn default_localized(node: &roxmltree::Node, tag: &str) -> Option<String> {
    node.children()
        .filter(|c| c.is_element() && c.tag_name().name() == tag)
        .find(|c| !c.attributes().any(|a| a.name() == "lang"))
        .and_then(|c| c.text())
        .map(str::to_string)
}

/// The unlocalized `<description>`'s concatenated `<p>` paragraph texts.
fn description_text(node: &roxmltree::Node) -> Option<String> {
    let desc = node
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "description")
        .find(|c| !c.attributes().any(|a| a.name() == "lang"))?;
    let paras: Vec<&str> = desc
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "p")
        .filter_map(|p| p.text())
        .collect();
    if paras.is_empty() {
        None
    } else {
        Some(paras.join("\n\n"))
    }
}

/// Every `<screenshots>/<screenshot>/<image>` URL, in document order.
fn screenshot_urls(node: &roxmltree::Node) -> Vec<String> {
    node.children()
        .filter(|c| c.is_element() && c.tag_name().name() == "screenshots")
        .flat_map(|shots| shots.children())
        .filter(|c| c.is_element() && c.tag_name().name() == "screenshot")
        .flat_map(|shot| shot.children())
        .filter(|c| c.is_element() && c.tag_name().name() == "image")
        .filter_map(|img| img.text().map(str::to_string))
        .collect()
}

/// The first `<icon>` reference, preferring a `type="remote"` URL the store can
/// fetch, else the first icon's text (a cached/stock name).
fn icon_ref(node: &roxmltree::Node) -> Option<String> {
    let icons: Vec<roxmltree::Node> = node
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "icon")
        .collect();
    icons
        .iter()
        .find(|c| c.attribute("type") == Some("remote"))
        .or_else(|| icons.first())
        .and_then(|c| c.text())
        .map(str::to_string)
}

// --- Debian DEP-11 (AppStream-in-YAML) reader -----------------------------------

/// A DEP-11 component document. Fields are optional so the leading header document
/// (`File: DEP-11`, no `ID`) and any partial record parse without erroring; a record
/// with no `ID` is skipped by [`dep11_entries`]. Localized fields are locale maps
/// (`{C: ..., de: ...}`); only the `C` (unlocalized) value is rendered.
#[derive(serde::Deserialize)]
struct Dep11Component {
    #[serde(rename = "ID")]
    id: Option<String>,
    #[serde(rename = "Name")]
    name: Option<std::collections::BTreeMap<String, String>>,
    #[serde(rename = "Summary")]
    summary: Option<std::collections::BTreeMap<String, String>>,
    #[serde(rename = "Description")]
    description: Option<std::collections::BTreeMap<String, String>>,
    #[serde(rename = "Icon")]
    icon: Option<Dep11Icon>,
    #[serde(rename = "Screenshots")]
    screenshots: Option<Vec<Dep11Screenshot>>,
}

#[derive(serde::Deserialize)]
struct Dep11Icon {
    remote: Option<Vec<Dep11RemoteIcon>>,
    cached: Option<Vec<Dep11CachedIcon>>,
}

#[derive(serde::Deserialize)]
struct Dep11RemoteIcon {
    url: Option<String>,
}

#[derive(serde::Deserialize)]
struct Dep11CachedIcon {
    name: Option<String>,
}

#[derive(serde::Deserialize)]
struct Dep11Screenshot {
    #[serde(rename = "source-image")]
    source_image: Option<Dep11Image>,
}

#[derive(serde::Deserialize)]
struct Dep11Image {
    url: Option<String>,
}

/// Parse a Debian DEP-11 catalog (a multi-document YAML stream: a header document
/// then one document per component) into one `CatalogEntry` per app (`layer = Apt`).
/// Display comes from the `C` (unlocalized) locale value of each field; the capability
/// footprint (the apt-enrolled profile, section 5) and the trust signals (Debian
/// keyring / popcon / reproduce.debian.net) come from SEPARATE sources per section 9.2
/// and stay empty here.
///
/// Best-effort per record, matching the module's skip-don't-guess philosophy: a
/// document with no `ID` (the header, a partial record) is skipped, and a single
/// document that fails to deserialize (a corrupt or non-conformant record, e.g. a
/// `Name:` that is a scalar rather than the expected locale map) is skipped too,
/// keeping the rest. A real DEP-11 catalog carries thousands of components, so one
/// bad record must not drop every Debian app from the store. There is no whole-stream
/// error: an entirely unparseable input simply yields no entries.
pub fn dep11_entries(yaml: &str) -> Vec<CatalogEntry> {
    use serde::Deserialize;
    let mut entries = Vec::new();
    for doc in serde_yaml::Deserializer::from_str(yaml) {
        let Ok(comp) = Dep11Component::deserialize(doc) else {
            continue; // A corrupt/non-conformant record is skipped, not fatal to the source.
        };
        let Some(id) = comp.id else {
            continue; // The header document or a record with no id.
        };
        let c = |m: &Option<std::collections::BTreeMap<String, String>>| {
            m.as_ref().and_then(|m| m.get("C").cloned())
        };
        let display = DisplayMeta {
            name: c(&comp.name).unwrap_or_default(),
            summary: c(&comp.summary),
            description: c(&comp.description),
            screenshots: comp
                .screenshots
                .unwrap_or_default()
                .into_iter()
                .filter_map(|s| s.source_image.and_then(|i| i.url))
                .collect(),
            icon: comp.icon.and_then(dep11_icon_ref),
        };
        entries.push(CatalogEntry {
            id: ComponentId(id),
            layer: SourceLayer::Apt,
            display,
            capabilities: CapabilityFootprint::default(),
            trust: TrustSignals::default(),
        });
    }
    entries
}

/// The best icon reference from a DEP-11 `Icon` block: a remote URL the store can
/// fetch if present, else the first cached icon's name.
fn dep11_icon_ref(icon: Dep11Icon) -> Option<String> {
    icon.remote
        .and_then(|r| r.into_iter().find_map(|i| i.url))
        .or_else(|| icon.cached.and_then(|c| c.into_iter().find_map(|i| i.name)))
}

// --- compose orchestration (section 9.3: "produces the one merged model") --------

/// The already-read source contents the compose step merges. Held as text (not file
/// paths) so the orchestration is pure and testable; the daemon reads the files.
#[derive(Debug, Default)]
pub struct SourceInputs {
    /// `(recipe.toml text, the cookbook's resolved tier)` per forage recipe.
    pub forage: Vec<(String, SourceLayer)>,
    /// The Flathub composed-AppStream catalog XML, when present on the image.
    pub flathub_xml: Option<String>,
    /// The Debian DEP-11 catalog YAML, when present on the image.
    pub dep11_yaml: Option<String>,
}

/// Compose the merged [`Catalog`] from every configured source. Best-effort: a source
/// that fails to parse (one malformed recipe, an unreadable catalog) is SKIPPED, never
/// fatal, so a single bad input cannot blank the whole store. Returns the deduped,
/// merged catalog the `org.arlen.Store1` backend serves.
pub fn compose_catalog(inputs: SourceInputs) -> crate::query::Catalog {
    let mut entries = Vec::new();
    for (toml, layer) in &inputs.forage {
        if let Ok(recipe) = arlen_forage_recipe::parse(toml) {
            entries.push(forage_entry(&recipe, *layer));
        }
    }
    if let Some(xml) = &inputs.flathub_xml {
        if let Ok(es) = flathub_entries(xml) {
            entries.extend(es);
        }
    }
    if let Some(yaml) = &inputs.dep11_yaml {
        // Best-effort per record: a corrupt document is skipped inside, never fatal.
        entries.extend(dep11_entries(yaml));
    }
    crate::query::Catalog::new(crate::catalog::merge_catalog(entries))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::merge_catalog;

    fn recipe_toml(id: &str, extra: &str) -> Recipe {
        let text = format!(
            r#"
[recipe]
id = "{id}"
name = "Demo App"
summary = "a demo"
description = "a longer demo description"
screenshots = ["https://example.org/a.png"]
maintainer = "key1"
{extra}

[[source]]
type = "git"
url = "https://github.com/example/demo"
commit = "0000000000000000000000000000000000000000"
"#
        );
        arlen_forage_recipe::parse(&text).expect("valid recipe")
    }

    #[test]
    fn maps_recipe_metadata_to_the_display() {
        let r = recipe_toml("org.demo.App", "");
        let e = forage_entry(&r, SourceLayer::Official);
        assert_eq!(e.id, ComponentId("org.demo.App".into()));
        assert_eq!(e.layer, SourceLayer::Official);
        assert_eq!(e.display.name, "Demo App");
        assert_eq!(e.display.summary.as_deref(), Some("a demo"));
        assert_eq!(e.display.screenshots.len(), 1);
    }

    #[test]
    fn coarse_capability_labels_enable_faceting() {
        let r = recipe_toml(
            "org.demo.App",
            "[capabilities]\nnetwork = [\"example.org:443\"]\nnotifications = true\ngraph = [\"read:File\"]",
        );
        let e = forage_entry(&r, SourceLayer::Community);
        // Sorted + deduped coarse categories.
        assert_eq!(e.capabilities.capabilities, vec!["network", "notifications", "read:File"]);
    }

    #[test]
    fn an_unverified_reproducible_status_hides_the_row() {
        // No [reproducible] block -> None (hidden).
        let r = recipe_toml("org.demo.App", "");
        assert!(forage_entry(&r, SourceLayer::Official).trust.reproducible_build.is_none());
        // An explicit verified status is shown.
        let r = recipe_toml("org.demo.App", "[reproducible]\nstatus = \"verified\"");
        assert_eq!(
            forage_entry(&r, SourceLayer::Official).trust.reproducible_build.as_deref(),
            Some("verified")
        );
    }

    #[test]
    fn a_forage_entry_flows_through_the_merge() {
        let r = recipe_toml("org.demo.App", "");
        let cards = merge_catalog(vec![forage_entry(&r, SourceLayer::Official)]);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].variants[0].layer, SourceLayer::Official);
    }

    const FLATHUB_XML: &str = r#"<?xml version="1.0"?>
<components>
  <component type="desktop-application">
    <id>org.gnome.Calculator</id>
    <name>Calculator</name>
    <name xml:lang="de">Taschenrechner</name>
    <summary>Do calculations</summary>
    <summary xml:lang="de">Rechnen</summary>
    <description><p>A powerful calculator.</p><p>It has modes.</p></description>
    <icon type="cached">org.gnome.Calculator.png</icon>
    <icon type="remote">https://dl.flathub.org/icon.png</icon>
    <screenshots>
      <screenshot type="default"><image>https://dl.flathub.org/a.png</image></screenshot>
      <screenshot><image>https://dl.flathub.org/b.png</image></screenshot>
    </screenshots>
  </component>
  <component type="desktop-application">
    <name>No Id</name>
  </component>
</components>"#;

    #[test]
    fn flathub_reader_maps_the_unlocalized_appstream_fields() {
        let entries = flathub_entries(FLATHUB_XML).unwrap();
        // The id-less component is skipped.
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.id, ComponentId("org.gnome.Calculator".into()));
        assert_eq!(e.layer, SourceLayer::Flatpak);
        assert_eq!(e.display.name, "Calculator", "the unlocalized name, not the de one");
        assert_eq!(e.display.summary.as_deref(), Some("Do calculations"));
        assert_eq!(
            e.display.description.as_deref(),
            Some("A powerful calculator.\n\nIt has modes.")
        );
        assert_eq!(e.display.screenshots, vec!["https://dl.flathub.org/a.png", "https://dl.flathub.org/b.png"]);
        // The remote icon wins over the cached one.
        assert_eq!(e.display.icon.as_deref(), Some("https://dl.flathub.org/icon.png"));
    }

    #[test]
    fn flathub_reader_rejects_malformed_xml() {
        assert!(matches!(flathub_entries("<components><oops"), Err(ComposeError::Xml(_))));
    }

    #[test]
    fn flathub_entries_flow_through_the_merge() {
        let entries = flathub_entries(FLATHUB_XML).unwrap();
        let cards = merge_catalog(entries);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].variants[0].layer, SourceLayer::Flatpak);
    }

    const DEP11_YAML: &str = r#"File: DEP-11
Version: '0.8'
Origin: debian-bookworm-main
---
Type: desktop-application
ID: org.gnome.gedit
Name:
  C: Text Editor
  de: Texteditor
Summary:
  C: Edit text files
Description:
  C: <p>A GNOME text editor.</p>
Icon:
  cached:
    - name: org.gnome.gedit.png
      width: 64
  remote:
    - url: https://debian.example/gedit.png
Screenshots:
  - default: true
    source-image:
      url: https://debian.example/shot.png
---
Type: desktop-application
Name:
  C: No Id App
"#;

    #[test]
    fn dep11_reader_maps_the_c_locale_fields() {
        let entries = dep11_entries(DEP11_YAML);
        // Header doc + the id-less record are both skipped.
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.id, ComponentId("org.gnome.gedit".into()));
        assert_eq!(e.layer, SourceLayer::Apt);
        assert_eq!(e.display.name, "Text Editor", "the C locale, not the de one");
        assert_eq!(e.display.summary.as_deref(), Some("Edit text files"));
        assert_eq!(e.display.description.as_deref(), Some("<p>A GNOME text editor.</p>"));
        assert_eq!(e.display.screenshots, vec!["https://debian.example/shot.png"]);
        // The remote icon URL wins over the cached name.
        assert_eq!(e.display.icon.as_deref(), Some("https://debian.example/gedit.png"));
    }

    #[test]
    fn dep11_reader_skips_a_corrupt_record_and_keeps_the_rest() {
        // A real DEP-11 catalog is thousands of documents; one non-conformant record
        // (here a `Name:` that is a scalar, not the expected locale map) must not
        // drop every Debian app. The two well-formed records survive it.
        let yaml = r#"File: DEP-11
---
Type: desktop-application
ID: good.one
Name:
  C: Good One
---
Type: desktop-application
ID: bad.two
Name: "not a locale map"
---
Type: desktop-application
ID: good.three
Name:
  C: Good Three
"#;
        let entries = dep11_entries(yaml);
        let ids: Vec<&str> = entries.iter().map(|e| e.id.0.as_str()).collect();
        assert_eq!(ids, vec!["good.one", "good.three"], "the corrupt record is skipped, the rest kept");
    }

    #[test]
    fn dep11_entries_flow_through_the_merge() {
        let cards = merge_catalog(dep11_entries(DEP11_YAML));
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].variants[0].layer, SourceLayer::Apt);
    }

    const FORAGE_TOML: &str = r#"
[recipe]
id = "org.forage.Tool"
name = "Forage Tool"
summary = "a tool"
maintainer = "key1"

[[source]]
type = "git"
url = "https://github.com/example/tool"
commit = "0000000000000000000000000000000000000000"
"#;

    #[test]
    fn compose_catalog_merges_every_source() {
        let inputs = SourceInputs {
            forage: vec![(FORAGE_TOML.to_string(), SourceLayer::Community)],
            flathub_xml: Some(FLATHUB_XML.to_string()),
            dep11_yaml: Some(DEP11_YAML.to_string()),
        };
        let catalog = compose_catalog(inputs);
        // One forage + one Flathub + one DEP-11 app, all distinct ids -> 3 cards.
        assert!(catalog.card(&ComponentId("org.forage.Tool".into())).is_some());
        assert!(catalog.card(&ComponentId("org.gnome.Calculator".into())).is_some());
        assert!(catalog.card(&ComponentId("org.gnome.gedit".into())).is_some());
    }

    #[test]
    fn compose_catalog_skips_a_malformed_source() {
        let inputs = SourceInputs {
            forage: vec![("this is not valid toml {{{".to_string(), SourceLayer::Personal)],
            flathub_xml: Some("<not xml".to_string()),
            dep11_yaml: Some(DEP11_YAML.to_string()),
        };
        // The bad forage + bad XML are skipped; the good DEP-11 app still lands.
        let catalog = compose_catalog(inputs);
        assert!(catalog.card(&ComponentId("org.gnome.gedit".into())).is_some());
    }
}
