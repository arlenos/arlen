//! SC-2: the card view model the store app renders (store-app.md section 9.4).
//!
//! The merged [`AppCard`] is the storage model (an id, display metadata and one
//! variant per source layer). The app needs a flat card: the tier badge, the
//! capability facets, the least-privilege sort key and whether the app is already
//! installed. This module derives exactly that, and nothing else.
//!
//! **The line between here and the frontend is copy.** The store app is
//! translated (`st.*`, en + de), so this emits the capability *identifiers* the
//! catalog already carries (`network`, `filesystem`, `read:File`, ...) and the
//! app renders each into its own language ("Cannot reach the network"). A Rust
//! backend writing that sentence would ship one language.
//!
//! Section 8.1 is binding on the vocabulary: the facets are the real
//! capability-token grant classes the recipe declares, never a store-only
//! taxonomy invented here.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::catalog::{AppCard, ItemKind, SourceLayer, Variant};

/// The source-tier badge shown on a card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// A forage recipe (personal, community or official cookbook).
    Forage,
    /// A Flathub Flatpak.
    Flathub,
    /// An apt `.deb`.
    Debian,
    /// Already installed by the distribution's own package manager. The badge
    /// exists so a card from this source cannot be mistaken for something the
    /// store can install: it carries no install handle.
    Installed,
}

impl From<SourceLayer> for Tier {
    fn from(layer: SourceLayer) -> Self {
        match layer {
            // All three cookbook tiers install through forage; the store badge
            // names the mechanism, not which cookbook it came from.
            SourceLayer::Personal | SourceLayer::Community | SourceLayer::Official => Tier::Forage,
            SourceLayer::Flatpak => Tier::Flathub,
            SourceLayer::Apt => Tier::Debian,
            SourceLayer::Native => Tier::Installed,
        }
    }
}

/// One catalog card, flattened for rendering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreCard {
    /// The AppStream component-id.
    pub id: String,
    /// The human name.
    pub name: String,
    /// The one-line summary (empty when the source supplies none).
    pub summary: String,
    /// The long description, when the source supplies one.
    pub description: Option<String>,
    /// The icon reference (a name or URL) from the richest source.
    pub icon: Option<String>,
    /// Screenshot URLs, in order.
    pub screenshots: Vec<String>,
    /// Which mechanism installs the default variant.
    pub tier: Tier,
    /// Whether this is an app or a foreign-app bridge. A bridge installs
    /// alongside the app it serves, so the store renders it differently.
    pub kind: ItemKind,
    /// The default variant's capability identifiers, sorted. The app renders each
    /// into its own language; this never carries user-facing copy.
    pub capabilities: Vec<String>,
    /// Least-privilege sort key: how many capabilities the default variant asks
    /// for. Fewer sorts first.
    pub cap_weight: usize,
    /// No variant requests network. Matches `CapabilityFacet::Excludes("network")`
    /// so a card surviving the "No network" filter always displays this as true.
    pub no_network: bool,
    /// No variant reaches off-machine. Today that is exactly [`Self::no_network`]
    /// (network is the only egress class the recipe vocabulary declares); it stays
    /// a separate field because the app renders it as its own facet.
    pub offline_only: bool,
    /// No variant requests a Knowledge Graph scope.
    pub no_graph: bool,
    /// The default variant carries a verified-publisher signal.
    pub verified: bool,
    /// The default variant carries a reproducible-build attestation.
    pub reproducible: bool,
    /// This component is already installed on the machine.
    pub installed: bool,
    /// Whether the store has a route to install this app. False for something the
    /// distribution installed, and for a catalog entry that names no package: the
    /// card is real and browsable, the action is not available.
    pub installable: bool,
}

/// Whether a capability label names a Knowledge Graph scope. `compose` pushes
/// graph scopes verbatim (`read:File`, `write:Project`), so the prefix is the
/// discriminator; the flat category tokens (`network`, `clipboard`, ...) never
/// carry one.
fn is_graph_scope(label: &str) -> bool {
    label.starts_with("read:") || label.starts_with("write:")
}

/// Whether a capability label names network egress.
fn is_network(label: &str) -> bool {
    label == "network"
}

/// The variant a press of Install would take: the card's declared default, or
/// the first variant if the index is somehow out of range (a merged card always
/// has at least one variant, so this is a fail-safe, not a normal path).
fn default_variant(card: &AppCard) -> Option<&Variant> {
    card.variants.get(card.default_variant).or_else(|| card.variants.first())
}

/// Whether ANY variant of the card requests a capability matching `pred`. The
/// facet booleans use this (not the default variant alone) so they agree with
/// the backend's `CapabilityFacet::Excludes` filter, which is defined over every
/// variant: a card that survives a facet filter always displays that facet true.
fn any_variant_requests(card: &AppCard, pred: impl Fn(&str) -> bool) -> bool {
    card.variants
        .iter()
        .any(|v| v.capabilities.capabilities.iter().any(|c| pred(c)))
}

/// Flatten one merged card for the app. `installed` is the set of component-ids
/// the machine reports installed (from installd's `ListInstalled`); an id absent
/// from it renders as not installed, which is the honest default when the
/// install daemon is unreachable.
pub fn store_card(card: &AppCard, installed: &BTreeSet<String>) -> StoreCard {
    let default = default_variant(card);
    let capabilities = default
        .map(|v| v.capabilities.capabilities.clone())
        .unwrap_or_default();
    let no_network = !any_variant_requests(card, is_network);

    StoreCard {
        id: card.id.0.clone(),
        name: card.display.name.clone(),
        summary: card.display.summary.clone().unwrap_or_default(),
        description: card.display.description.clone(),
        icon: card.display.icon.clone(),
        screenshots: card.display.screenshots.clone(),
        tier: default.map(|v| Tier::from(v.layer)).unwrap_or(Tier::Forage),
        kind: card.kind,
        cap_weight: capabilities.len(),
        capabilities,
        no_network,
        // Network is the only off-machine class the recipe vocabulary declares,
        // so "offline only" and "no network" coincide today. Kept distinct
        // because the app shows them as two facets; if a second egress class is
        // ever added, only this line changes.
        offline_only: no_network,
        no_graph: !any_variant_requests(card, is_graph_scope),
        verified: default.is_some_and(|v| v.trust.verified_publisher.is_some()),
        reproducible: default.is_some_and(|v| v.trust.reproducible_build.is_some()),
        // Installed by US, or present as a distribution package. A `Native`
        // variant EXISTS because `/usr/share/metainfo` says the app is installed
        // on this machine, so reporting it as not-installed - which is what the
        // installd set alone does, since installd never recorded it - would tell
        // the user their editor is missing while they have it open.
        installed: installed.contains(&card.id.0)
            || card.variants.iter().any(|v| v.layer == SourceLayer::Native),
        // Whether the store can act, which is not the same as whether the app
        // exists. A distribution package has no `install_handle` because there is
        // no route to install, update or remove it from here; the same is true of
        // a DEP-11 component that states no package name. Without this the surface
        // can only find out by trying, and the honest place to know is here.
        installable: default.is_some_and(|v| v.install_handle.is_some()),
    }
}

/// Flatten a list of merged cards.
pub fn store_cards(cards: &[AppCard], installed: &BTreeSet<String>) -> Vec<StoreCard> {
    cards.iter().map(|c| store_card(c, installed)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{
        merge_catalog, CapabilityFootprint, CatalogEntry, ComponentId, DisplayMeta, TrustSignals,
    };

    /// A distribution package is present and cannot be acted on, and the card has
    /// to say both. Reporting it as not-installed would deny an app the user has;
    /// reporting it as installable would offer a button with nothing behind it.
    #[test]
    fn a_distribution_package_reads_as_installed_and_not_installable() {
        let mut e = entry("com.example.App", SourceLayer::Native, &[]);
        e.install_handle = None;
        let card = card_of(vec![e]);
        let view = store_card(&card, &BTreeSet::new());
        assert!(view.installed, "metainfo exists because the app is on the machine");
        assert!(!view.installable, "the store has no route to a pacman package");
        assert_eq!(view.tier, Tier::Installed);
    }

    /// The ordinary case still reads the other way, so the flag tracks the
    /// variant rather than just being off for everything.
    #[test]
    fn a_catalog_app_with_a_package_reads_as_installable() {
        let mut e = entry("com.example.Other", SourceLayer::Flatpak, &[]);
        e.install_handle = Some("com.example.Other".into());
        let card = card_of(vec![e]);
        let view = store_card(&card, &BTreeSet::new());
        assert!(!view.installed);
        assert!(view.installable);
    }

    fn entry(id: &str, layer: SourceLayer, caps: &[&str]) -> CatalogEntry {
        CatalogEntry {
            id: ComponentId(id.into()),
            layer,
            display: DisplayMeta {
                name: "Demo".into(),
                summary: Some("A demo".into()),
                ..Default::default()
            },
            capabilities: CapabilityFootprint {
                capabilities: caps.iter().map(|c| c.to_string()).collect(),
            },
            trust: TrustSignals::default(),
            kind: ItemKind::default(),
            version: String::new(),
            install_handle: None,
        }
    }

    fn card_of(entries: Vec<CatalogEntry>) -> AppCard {
        merge_catalog(entries).into_iter().next().unwrap()
    }

    fn none() -> BTreeSet<String> {
        BTreeSet::new()
    }

    #[test]
    fn a_clean_app_reports_every_negative_facet() {
        let c = card_of(vec![entry("org.x.Notes", SourceLayer::Official, &["filesystem"])]);
        let v = store_card(&c, &none());
        assert!(v.no_network);
        assert!(v.offline_only);
        assert!(v.no_graph);
        assert_eq!(v.cap_weight, 1);
        assert_eq!(v.tier, Tier::Forage);
    }

    #[test]
    fn a_networked_app_clears_the_network_facets_but_not_the_graph_one() {
        let c = card_of(vec![entry(
            "org.x.Chat",
            SourceLayer::Official,
            &["filesystem", "network"],
        )]);
        let v = store_card(&c, &none());
        assert!(!v.no_network);
        assert!(!v.offline_only);
        // No graph scope was declared, so that facet still holds.
        assert!(v.no_graph);
        assert_eq!(v.cap_weight, 2);
    }

    #[test]
    fn a_graph_scope_clears_only_the_graph_facet() {
        let c = card_of(vec![entry("org.x.Agent", SourceLayer::Official, &["read:File"])]);
        let v = store_card(&c, &none());
        assert!(!v.no_graph);
        // A graph read is not egress: the network facets are untouched.
        assert!(v.no_network);
        assert!(v.offline_only);
    }

    /// The facet booleans must be defined over EVERY variant, matching the
    /// backend's `Excludes` filter. A card whose Flatpak variant asks for network
    /// while its forage variant does not must NOT claim "no network", or the
    /// filtered list and the card badge would contradict each other.
    #[test]
    fn a_facet_is_false_when_any_variant_requests_the_capability() {
        let c = card_of(vec![
            entry("org.x.Paint", SourceLayer::Official, &["filesystem"]),
            entry("org.x.Paint", SourceLayer::Flatpak, &["network"]),
        ]);
        let v = store_card(&c, &none());
        assert!(!v.no_network, "a networked sibling variant must clear the facet");
        // The displayed capabilities and weight still describe the DEFAULT
        // variant (what Install would actually take), which is the forage one.
        assert_eq!(v.capabilities, vec!["filesystem".to_string()]);
        assert_eq!(v.tier, Tier::Forage);
    }

    #[test]
    fn the_tier_names_the_default_variants_mechanism() {
        let flatpak = card_of(vec![entry("org.x.Only", SourceLayer::Flatpak, &[])]);
        assert_eq!(store_card(&flatpak, &none()).tier, Tier::Flathub);
        let apt = card_of(vec![entry("org.x.Deb", SourceLayer::Apt, &[])]);
        assert_eq!(store_card(&apt, &none()).tier, Tier::Debian);
        let community = card_of(vec![entry("org.x.Comm", SourceLayer::Community, &[])]);
        assert_eq!(store_card(&community, &none()).tier, Tier::Forage);
    }

    #[test]
    fn installed_reflects_the_reported_set() {
        let c = card_of(vec![entry("org.x.Notes", SourceLayer::Official, &[])]);
        assert!(!store_card(&c, &none()).installed);
        let set: BTreeSet<String> = ["org.x.Notes".to_string()].into_iter().collect();
        assert!(store_card(&c, &set).installed);
        // A different id installed must not mark this card.
        let other: BTreeSet<String> = ["org.y.Other".to_string()].into_iter().collect();
        assert!(!store_card(&c, &other).installed);
    }

    #[test]
    fn trust_signals_surface_only_when_attested() {
        let mut e = entry("org.x.Notes", SourceLayer::Official, &[]);
        let plain = store_card(&card_of(vec![e.clone()]), &none());
        assert!(!plain.verified);
        assert!(!plain.reproducible);

        e.trust.verified_publisher = Some("flathub".into());
        e.trust.reproducible_build = Some("attested".into());
        let attested = store_card(&card_of(vec![e]), &none());
        assert!(attested.verified);
        assert!(attested.reproducible);
    }

    /// The wire shape is what the app destructures; a rename would break it
    /// silently, so pin the camelCase keys.
    #[test]
    fn the_json_shape_is_camel_case() {
        let c = card_of(vec![entry("org.x.Notes", SourceLayer::Official, &["network"])]);
        let json = serde_json::to_string(&store_card(&c, &none())).unwrap();
        for key in [
            "\"id\"",
            "\"capWeight\"",
            "\"noNetwork\"",
            "\"offlineOnly\"",
            "\"noGraph\"",
            "\"tier\":\"forage\"",
            "\"installed\"",
        ] {
            assert!(json.contains(key), "missing {key} in {json}");
        }
    }
}
