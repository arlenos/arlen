//! The catalog merge/dedup data model (store-app.md section 9.1).
//!
//! The merge key is the AppStream component-id (reverse-DNS). When one id appears in
//! more than one layer, the store shows ONE merged [`AppCard`] with per-source
//! install [`Variant`]s, never N duplicate cards and never a silent pick:
//! - display metadata is taken from the RICHEST AppStream source for that id (most
//!   complete metainfo; ties resolve by layer precedence),
//! - the capability + trust panel is PER-VARIANT (a Flatpak's finish-args are not a
//!   forage recipe's `[capabilities]`), so "install the least-privilege variant" is
//!   a real choice,
//! - the default variant follows the forage resolution precedence
//!   (personal -> community -> official -> Flatpak -> apt).

use serde::{Deserialize, Serialize};

/// An AppStream component-id (reverse-DNS), the merge key across layers.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ComponentId(pub String);

/// The source layer an app variant comes from, in resolution-precedence order
/// (`distribution-and-submission.md` section 1): personal cookbooks win, apt is the
/// fallback. The `Ord` derive orders variants highest-precedence first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SourceLayer {
    /// A personal cookbook recipe (highest precedence).
    Personal,
    /// A community cookbook recipe.
    Community,
    /// An official cookbook recipe.
    Official,
    /// A Flathub Flatpak.
    Flatpak,
    /// An apt `.deb` (the fallback).
    Apt,
}

/// The display metadata shown on a card, from the richest AppStream source.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayMeta {
    /// The human name.
    pub name: String,
    /// The one-line summary.
    pub summary: Option<String>,
    /// The long description (may be markup-stripped plain text).
    pub description: Option<String>,
    /// Screenshot URLs, in order.
    #[serde(default)]
    pub screenshots: Vec<String>,
    /// The icon reference (a name or a URL).
    pub icon: Option<String>,
}

impl DisplayMeta {
    /// A coarse completeness score: how many rich fields are populated. Used to pick
    /// the richest source for a merged card (ties resolve by layer precedence).
    pub fn completeness(&self) -> usize {
        let mut n = 0;
        if !self.name.is_empty() {
            n += 1;
        }
        if self.summary.as_ref().is_some_and(|s| !s.is_empty()) {
            n += 1;
        }
        if self.description.as_ref().is_some_and(|s| !s.is_empty()) {
            n += 1;
        }
        if !self.screenshots.is_empty() {
            n += 1;
        }
        if self.icon.as_ref().is_some_and(|s| !s.is_empty()) {
            n += 1;
        }
        n
    }
}

/// A variant's capability footprint: the tier badge plus the concrete capabilities
/// it requests, so the install picker can show the least-privilege choice.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityFootprint {
    /// The trust tier badge (e.g. "verified", "community"), when the layer supplies
    /// one; `None` hides the badge.
    pub tier: Option<String>,
    /// The requested capabilities (network hosts, filesystem scopes, devices...),
    /// in a stable display order.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// Per-layer trust signals (store-app.md section 9.2). Each field is `None` when the
/// layer does not attest that signal, so the UI HIDES a blank row rather than showing
/// it empty. Populated by the resolver, not the merge.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrustSignals {
    /// A verified-publisher marker (Flathub-verified, a Debian keyring maintainer, a
    /// forage `.well-known` proof).
    pub verified_publisher: Option<String>,
    /// Reproducible-build status, when attested.
    pub reproducible_build: Option<String>,
    /// A transparent, non-identifying install-count aggregate, when available.
    pub install_count: Option<u64>,
    /// An ODRS score, when present.
    pub odrs_score: Option<f32>,
    /// A local observed-vs-declared summary from the audit ledger, when computed.
    pub observed_vs_declared: Option<String>,
}

/// One per-source app entry the compose step produced (section 9.3): the merge input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogEntry {
    /// The AppStream component-id (the merge key).
    pub id: ComponentId,
    /// Which layer this entry came from.
    pub layer: SourceLayer,
    /// This source's display metadata (may be richer or poorer than a sibling's).
    pub display: DisplayMeta,
    /// This variant's capability footprint.
    pub capabilities: CapabilityFootprint,
    /// This variant's trust signals.
    pub trust: TrustSignals,
}

/// One install option on a merged card: a source layer with its own caps + trust.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Variant {
    /// The source layer.
    pub layer: SourceLayer,
    /// This variant's capability footprint.
    pub capabilities: CapabilityFootprint,
    /// This variant's trust signals.
    pub trust: TrustSignals,
}

/// A merged app card: one per component-id, richest display, per-source variants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppCard {
    /// The AppStream component-id.
    pub id: ComponentId,
    /// The display metadata, from the richest source.
    pub display: DisplayMeta,
    /// The install variants, ordered highest-precedence first.
    pub variants: Vec<Variant>,
    /// The index (into `variants`) of the default variant: the highest-precedence
    /// one. Browsing exposes all; installing picks this unless the user chooses.
    pub default_variant: usize,
}

/// Merge per-source catalog entries into one card per component-id (section 9.1).
/// Cards are returned sorted by id (deterministic); within a card, variants are
/// ordered highest-precedence first and `default_variant` is `0`. The display is the
/// richest source's (ties -> highest precedence). A duplicate (id, layer) keeps the
/// richer display and drops the poorer one, so a layer never contributes two variants
/// for the same id.
pub fn merge_catalog(entries: Vec<CatalogEntry>) -> Vec<AppCard> {
    use std::collections::BTreeMap;

    // Group by id, deterministically (BTreeMap orders by id).
    let mut groups: BTreeMap<ComponentId, Vec<CatalogEntry>> = BTreeMap::new();
    for e in entries {
        groups.entry(e.id.clone()).or_default().push(e);
    }

    let mut cards = Vec::with_capacity(groups.len());
    for (id, mut group) in groups {
        // The display source: the richest entry, ties broken by highest precedence
        // (the smallest `SourceLayer` under its precedence `Ord`).
        let display = group
            .iter()
            .max_by(|a, b| {
                a.display
                    .completeness()
                    .cmp(&b.display.completeness())
                    // A higher precedence (smaller layer) wins a tie, so reverse the
                    // layer comparison (smaller layer -> "greater" for max_by).
                    .then_with(|| b.layer.cmp(&a.layer))
            })
            .map(|e| e.display.clone())
            .unwrap_or_default();

        // One variant per layer: if a layer appears twice, keep the richer display's
        // caps/trust (defensive; the compose step should not emit duplicates).
        group.sort_by(|a, b| {
            a.layer
                .cmp(&b.layer)
                .then_with(|| b.display.completeness().cmp(&a.display.completeness()))
        });
        let mut variants: Vec<Variant> = Vec::new();
        for e in group {
            if variants.last().is_some_and(|v| v.layer == e.layer) {
                continue; // A poorer duplicate for the same layer; already have it.
            }
            variants.push(Variant { layer: e.layer, capabilities: e.capabilities, trust: e.trust });
        }

        cards.push(AppCard { id, display, variants, default_variant: 0 });
    }
    cards
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, layer: SourceLayer, name: &str, screenshots: usize) -> CatalogEntry {
        CatalogEntry {
            id: ComponentId(id.into()),
            layer,
            display: DisplayMeta {
                name: name.into(),
                summary: Some("a summary".into()),
                description: None,
                screenshots: (0..screenshots).map(|i| format!("shot{i}")).collect(),
                icon: Some("icon".into()),
            },
            capabilities: CapabilityFootprint {
                tier: Some(format!("{layer:?}")),
                capabilities: vec!["network".into()],
            },
            trust: TrustSignals::default(),
        }
    }

    #[test]
    fn one_id_across_layers_merges_to_one_card_with_variants() {
        let cards = merge_catalog(vec![
            entry("org.x.App", SourceLayer::Flatpak, "App", 2),
            entry("org.x.App", SourceLayer::Apt, "App", 0),
            entry("org.x.App", SourceLayer::Official, "App", 1),
        ]);
        assert_eq!(cards.len(), 1, "one merged card, never three");
        let card = &cards[0];
        // Variants ordered highest precedence first: Official < Flatpak < Apt.
        assert_eq!(
            card.variants.iter().map(|v| v.layer).collect::<Vec<_>>(),
            vec![SourceLayer::Official, SourceLayer::Flatpak, SourceLayer::Apt]
        );
        assert_eq!(card.default_variant, 0);
        assert_eq!(card.variants[card.default_variant].layer, SourceLayer::Official);
    }

    #[test]
    fn display_comes_from_the_richest_source() {
        // Flatpak has 3 screenshots (richest) though Official has higher precedence.
        let cards = merge_catalog(vec![
            entry("org.x.App", SourceLayer::Official, "App-official", 0),
            entry("org.x.App", SourceLayer::Flatpak, "App-flathub", 3),
        ]);
        assert_eq!(cards[0].display.name, "App-flathub");
        assert_eq!(cards[0].display.screenshots.len(), 3);
    }

    #[test]
    fn a_completeness_tie_resolves_by_precedence() {
        // Same completeness; Official (higher precedence) supplies the display.
        let cards = merge_catalog(vec![
            entry("org.x.App", SourceLayer::Flatpak, "App-flathub", 2),
            entry("org.x.App", SourceLayer::Official, "App-official", 2),
        ]);
        assert_eq!(cards[0].display.name, "App-official");
    }

    #[test]
    fn distinct_ids_stay_distinct_cards_sorted() {
        let cards = merge_catalog(vec![
            entry("org.z.Zed", SourceLayer::Flatpak, "Zed", 1),
            entry("org.a.Ay", SourceLayer::Apt, "Ay", 1),
        ]);
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].id, ComponentId("org.a.Ay".into()), "sorted by id");
        assert_eq!(cards[1].id, ComponentId("org.z.Zed".into()));
    }

    #[test]
    fn a_duplicate_layer_yields_one_variant_keeping_the_richer() {
        let cards = merge_catalog(vec![
            entry("org.x.App", SourceLayer::Flatpak, "poor", 0),
            entry("org.x.App", SourceLayer::Flatpak, "rich", 2),
        ]);
        assert_eq!(cards[0].variants.len(), 1, "one Flatpak variant, not two");
        // The richer display won the card display.
        assert_eq!(cards[0].display.name, "rich");
    }

    #[test]
    fn a_card_round_trips_through_json() {
        let cards = merge_catalog(vec![entry("org.x.App", SourceLayer::Official, "App", 1)]);
        let json = serde_json::to_string(&cards[0]).unwrap();
        let back: AppCard = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cards[0]);
    }
}
