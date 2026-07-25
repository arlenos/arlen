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
}
