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
    /// The XML did not parse.
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
}
