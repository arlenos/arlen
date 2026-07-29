//! The `org.arlen.Store1` op vocabulary and the read query engine over the merged
//! catalog (store-app.md section 9.4). The IPC transport (the session socket) is a
//! thin frame around [`answer`]; the query itself is pure over a [`Catalog`], so
//! search, capability facets and the per-id lookups are tested without a socket.
//!
//! `install` is the one effectful op: it does not run here, it VALIDATES the request
//! (the id + variant must exist) and hands a resolved target back to the caller, who
//! drives the consent friction-ladder + forage/flatpak/apt. So the pure layer still
//! answers it (accept / reject) without performing the install.

use serde::{Deserialize, Serialize};

use crate::catalog::{AppCard, ComponentId, SourceLayer, TrustSignals, Variant};

/// A capability facet for discovery filtering (section 9): match cards by a
/// capability their variants request. `Requires` keeps cards a variant of which asks
/// for the capability; `Excludes` keeps cards NO variant of which asks for it (the
/// least-privilege / offline-capable facets).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapabilityFacet {
    /// Keep cards with a variant requesting `capability`.
    Requires(String),
    /// Keep cards where no variant requests `capability`.
    Excludes(String),
}

impl CapabilityFacet {
    /// Whether `card` passes this facet.
    fn matches(&self, card: &AppCard) -> bool {
        let any_variant_requests = |cap: &str| {
            card.variants
                .iter()
                .any(|v| v.capabilities.capabilities.iter().any(|c| c == cap))
        };
        match self {
            CapabilityFacet::Requires(cap) => any_variant_requests(cap),
            CapabilityFacet::Excludes(cap) => !any_variant_requests(cap),
        }
    }
}

/// A store request over `org.arlen.Store1` (section 9.4, v1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Request {
    /// Full-text search over name + summary, narrowed by capability facets (ANDed).
    Search {
        /// The query text (case-insensitive substring, empty matches all).
        query: String,
        /// Capability facets to AND against the results.
        #[serde(default)]
        facets: Vec<CapabilityFacet>,
    },
    /// List every card passing a single capability facet.
    ListByFacet {
        /// The facet to filter by.
        facet: CapabilityFacet,
    },
    /// The full merged card for an id.
    AppDetail {
        /// The component-id.
        id: ComponentId,
    },
    /// The per-variant trust signals for an id.
    TrustSignals {
        /// The component-id.
        id: ComponentId,
    },
    /// The install variants for an id.
    Variants {
        /// The component-id.
        id: ComponentId,
    },
    /// Validate + resolve an install target (the caller then drives consent + the
    /// backend installer). Does not install here.
    Install {
        /// The component-id.
        id: ComponentId,
        /// The chosen variant's layer.
        variant: SourceLayer,
    },
    /// The local observed-vs-declared summary for an id (audit-ledger read).
    ObservedVsDeclared {
        /// The component-id.
        id: ComponentId,
    },
}

/// What the store can honestly say about an app's observed-vs-declared standing
/// (store-app.md section 8.2). Structured, never prose: the store app is
/// translated, so it renders each variant in its own language. Crucially it
/// distinguishes "we have no feed" from "the feed says nothing was observed" -
/// collapsing those two into an empty panel would read as a clean bill of health
/// the system cannot actually give.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum ObservedStatus {
    /// No per-app capability-use feed exists yet, so nothing can be said. The
    /// audit ledger records the AI taxonomy (queries, tool calls, graph access,
    /// provider egress) and coarse app actions; it does NOT record "this app used
    /// its network capability", and `AuditKind::NetworkCall` is specifically the
    /// AI proxy's outbound call, not general per-app egress. Until the
    /// observe-mode feed lands the app must say so plainly rather than render an
    /// empty panel.
    Unavailable,
    /// The feed is live: `declared` is what the app asks for, `observed` the
    /// subset actually seen in use on this machine, both as capability
    /// identifiers. A declared capability absent from `observed` is "not observed
    /// on your machine", never "safe" (section 8.2's copy caveat).
    Measured {
        /// The capability identifiers the app declares.
        declared: Vec<String>,
        /// The identifiers observed in use locally.
        observed: Vec<String>,
        /// How many days the local observation window covers, so the app can say
        /// "in 3 months" rather than implying an all-time verdict.
        window_days: u32,
    },
}

/// A store response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Response {
    /// A list of matching cards (search / list_by_facet).
    Cards(Vec<AppCard>),
    /// A single card, or `None` if the id is unknown.
    Card(Option<AppCard>),
    /// Per-variant trust signals (variant layer + its signals).
    Trust(Vec<(SourceLayer, TrustSignals)>),
    /// The install variants for an id.
    Variants(Vec<Variant>),
    /// A validated install handoff: the id + variant exist; the caller proceeds.
    InstallResolved {
        /// The component-id.
        id: ComponentId,
        /// The resolved variant layer.
        variant: SourceLayer,
    },
    /// What can honestly be said about this app's observed-vs-declared standing.
    Observed(ObservedStatus),
    /// A request the backend could not satisfy (unknown id/variant, ...).
    Error(String),
}

/// The read-side catalog the backend serves: the merged cards, queried in memory.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    cards: Vec<AppCard>,
}

impl Catalog {
    /// Build from the merged cards (the output of [`crate::catalog::merge_catalog`]).
    pub fn new(cards: Vec<AppCard>) -> Self {
        Self { cards }
    }

    /// The card for an id, if present.
    pub fn card(&self, id: &ComponentId) -> Option<&AppCard> {
        self.cards.iter().find(|c| &c.id == id)
    }

    /// Search over name + summary (case-insensitive substring; empty query matches
    /// all), keeping only cards that pass EVERY facet.
    pub fn search(&self, query: &str, facets: &[CapabilityFacet]) -> Vec<AppCard> {
        let needle = query.trim().to_lowercase();
        self.cards
            .iter()
            .filter(|c| {
                let hay_name = c.display.name.to_lowercase();
                let hay_summary = c.display.summary.as_deref().unwrap_or("").to_lowercase();
                needle.is_empty() || hay_name.contains(&needle) || hay_summary.contains(&needle)
            })
            .filter(|c| facets.iter().all(|f| f.matches(c)))
            .cloned()
            .collect()
    }
}

/// Answer a request against the catalog (the read ops purely; `install` validates and
/// resolves a handoff, it does not perform the install). Unknown ids/variants return
/// [`Response::Error`], never a panic.
pub fn answer(catalog: &Catalog, request: Request) -> Response {
    match request {
        Request::Search { query, facets } => Response::Cards(catalog.search(&query, &facets)),
        Request::ListByFacet { facet } => Response::Cards(catalog.search("", &[facet])),
        Request::AppDetail { id } => Response::Card(catalog.card(&id).cloned()),
        Request::TrustSignals { id } => match catalog.card(&id) {
            Some(card) => Response::Trust(
                card.variants.iter().map(|v| (v.layer, v.trust.clone())).collect(),
            ),
            None => Response::Error(format!("unknown app: {}", id.0)),
        },
        Request::Variants { id } => match catalog.card(&id) {
            Some(card) => Response::Variants(card.variants.clone()),
            None => Response::Error(format!("unknown app: {}", id.0)),
        },
        Request::Install { id, variant } => match catalog.card(&id) {
            Some(card) if card.variants.iter().any(|v| v.layer == variant) => {
                Response::InstallResolved { id, variant }
            }
            Some(_) => Response::Error(format!("no {variant:?} variant for {}", id.0)),
            None => Response::Error(format!("unknown app: {}", id.0)),
        },
        // The observed-vs-declared read is an audit-ledger lookup the backend wires
        // later; the pure layer reports "nothing recorded" for an unknown/empty id.
        Request::ObservedVsDeclared { id } => match catalog.card(&id) {
            // The feed does not exist yet (LCG-R8). Say so, rather than return an
            // empty summary the app would render as "nothing observed".
            Some(_) => Response::Observed(ObservedStatus::Unavailable),
            None => Response::Error(format!("unknown app: {}", id.0)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{merge_catalog, CapabilityFootprint, CatalogEntry, DisplayMeta};

    fn entry(id: &str, layer: SourceLayer, name: &str, caps: &[&str]) -> CatalogEntry {
        CatalogEntry {
            id: ComponentId(id.into()),
            layer,
            display: DisplayMeta {
                name: name.into(),
                summary: Some(format!("{name} is great")),
                ..Default::default()
            },
            capabilities: CapabilityFootprint {
                tier: None,
                capabilities: caps.iter().map(|s| s.to_string()).collect(),
            },
            trust: TrustSignals::default(),
        }
    }

    fn catalog() -> Catalog {
        Catalog::new(merge_catalog(vec![
            entry("org.x.Chat", SourceLayer::Flatpak, "Chatter", &["network"]),
            entry("org.y.Paint", SourceLayer::Official, "Painter", &[]),
            entry("org.y.Paint", SourceLayer::Flatpak, "Painter", &["network"]),
        ]))
    }

    #[test]
    fn search_matches_name_and_summary_case_insensitively() {
        let cards = catalog().search("paint", &[]);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].id, ComponentId("org.y.Paint".into()));
        // Empty query returns everything.
        assert_eq!(catalog().search("", &[]).len(), 2);
    }

    #[test]
    fn a_requires_facet_keeps_only_network_apps() {
        let net = catalog().search("", &[CapabilityFacet::Requires("network".into())]);
        // Both Chat (network) and Paint (its Flatpak variant asks network) qualify.
        assert_eq!(net.len(), 2);
    }

    #[test]
    fn an_excludes_facet_needs_every_variant_clean() {
        // "org.y.Paint" has a network-requesting Flatpak variant, so Excludes(network)
        // drops it; only apps NO variant of which asks network survive.
        let clean = catalog().search("", &[CapabilityFacet::Excludes("network".into())]);
        assert!(clean.is_empty());
    }

    #[test]
    fn multiple_facets_are_anded_not_ored() {
        // The doc promises facets are ANDed: a card must pass EVERY facet. With a
        // single facet all() == any(), so single-facet tests cannot catch an
        // all()->any() regression; this uses two facets whose union differs from
        // their intersection. Requires(network) AND Excludes(camera) keeps only a
        // card that both asks network AND has no camera-requesting variant.
        let cat = Catalog::new(merge_catalog(vec![
            entry("org.x.Chat", SourceLayer::Flatpak, "Chatter", &["network"]),
            entry("org.z.Cam", SourceLayer::Flatpak, "Camera", &["network", "camera"]),
            entry("org.y.Paint", SourceLayer::Flatpak, "Painter", &[]),
        ]));
        let facets = vec![
            CapabilityFacet::Requires("network".into()),
            CapabilityFacet::Excludes("camera".into()),
        ];
        let cards = cat.search("", &facets);
        // AND: only Chat (network, no camera). Cam fails Excludes(camera); Paint fails
        // Requires(network). An OR mutation would also let Cam and Paint through.
        assert_eq!(cards.len(), 1, "facets must intersect, not union");
        assert_eq!(cards[0].id, ComponentId("org.x.Chat".into()));
    }

    #[test]
    fn install_validates_the_variant_exists() {
        let cat = catalog();
        match answer(&cat, Request::Install {
            id: ComponentId("org.y.Paint".into()),
            variant: SourceLayer::Official,
        }) {
            Response::InstallResolved { variant, .. } => assert_eq!(variant, SourceLayer::Official),
            other => panic!("expected InstallResolved, got {other:?}"),
        }
        // A layer with no variant is rejected, not panicked.
        match answer(&cat, Request::Install {
            id: ComponentId("org.y.Paint".into()),
            variant: SourceLayer::Apt,
        }) {
            Response::Error(_) => {}
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn unknown_id_is_an_error_not_a_panic() {
        let cat = catalog();
        match answer(&cat, Request::Variants { id: ComponentId("nope".into()) }) {
            Response::Error(_) => {}
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn a_request_round_trips_through_json() {
        let req = Request::Search {
            query: "chat".into(),
            facets: vec![CapabilityFacet::Excludes("camera".into())],
        };
        let json = serde_json::to_string(&req).unwrap();
        assert_eq!(serde_json::from_str::<Request>(&json).unwrap(), req);
    }
    /// The store must be able to say "no feed" distinctly from "nothing
    /// observed": collapsing them would render as a clean bill of health the
    /// system cannot give (section 8.2's copy caveat).
    #[test]
    fn observed_reports_the_feed_as_unavailable_not_as_an_empty_result() {
        match answer(
            &catalog(),
            Request::ObservedVsDeclared { id: ComponentId("org.x.Chat".into()) },
        ) {
            Response::Observed(ObservedStatus::Unavailable) => {}
            other => panic!("expected Unavailable, got {other:?}"),
        }
    }

    /// An unknown id is still an error, not an "unavailable" reading.
    #[test]
    fn observed_on_an_unknown_id_is_an_error() {
        match answer(
            &catalog(),
            Request::ObservedVsDeclared { id: ComponentId("org.nope.Gone".into()) },
        ) {
            Response::Error(_) => {}
            other => panic!("expected Error, got {other:?}"),
        }
    }

}
