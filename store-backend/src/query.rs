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

/// How results are ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    /// Catalog order: how the sources listed them.
    #[default]
    Relevance,
    /// The app that asks for least, first.
    ///
    /// The store's whole argument is that a capability is declared here rather
    /// than inferred from a binary, and this is what that declaration buys the
    /// user: a way to find the app that wants the least, which no store built on
    /// inference can offer.
    LeastPrivilege,
}

/// How much a card asks for, as the count of the most modest variant.
///
/// The MINIMUM over variants, not the sum or the maximum, because capabilities
/// are per-variant and the user picks the variant: the honest question is "how
/// little can I install this with", not "what is the worst packaging of it".
///
/// A card with no installable variant sorts last rather than first. It asks for
/// nothing only because there is nothing to install.
pub fn privilege_cost(card: &AppCard) -> usize {
    card.variants
        .iter()
        .map(|v| v.capabilities.capabilities.len())
        .min()
        .unwrap_or(usize::MAX)
}

/// Order cards by how little they ask for.
///
/// Ties keep catalog order rather than falling back to the name: two apps that
/// ask for the same thing have no privilege reason to outrank each other, and
/// re-sorting them alphabetically would bury whatever the sources ranked first.
/// `sort_by_key` is stable, so this holds.
pub fn sort_least_privilege(cards: &mut [AppCard]) {
    cards.sort_by_key(privilege_cost);
}

/// What is installed, as the caller read it from the install lock.
///
/// The store-backend does not read the lock itself: that lives with installd,
/// which writes it, and a store that reached into another daemon's state file
/// would break the moment either side moved. The caller passes the old side in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledVersion {
    /// Which layer it was installed from.
    pub layer: SourceLayer,
    /// The version recorded at install.
    pub version: String,
}

/// An app whose source now offers a different version than the one installed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingUpdate {
    /// The component-id.
    pub id: ComponentId,
    /// The layer it is installed from, which is the only layer compared.
    pub layer: SourceLayer,
    /// The version recorded at install.
    pub installed_version: String,
    /// The version that layer now offers.
    pub available_version: String,
}

/// Which installed apps their own source now offers at a different version.
///
/// **A local computation over the cached catalog.** No network call: the catalog
/// is refreshed in the background, and asking the network every time a page opens
/// would make opening the store a request to every source the user has.
///
/// Two rules keep the answer honest:
///
/// **Only the installed layer counts.** If an app was installed from apt and
/// Flathub offers a newer build, that is not an update, it is a different
/// packaging with its own capabilities and trust signals. Offering it as "an
/// update" would walk the user across a trust boundary they never chose.
///
/// **A version neither side states is not a change.** Sources that do not state a
/// version yield an empty string, and comparing empty against empty - or against
/// anything - would either flag every app forever or silently hide real updates.
/// An app is only reported when both versions are known and they differ.
///
/// Note it reports "differs", not "newer": ordering distro version strings
/// correctly is per-layer and intricate (dpkg's algorithm is famously so), and a
/// wrong ordering either hides updates or offers downgrades. Both versions are
/// carried so the caller can show them and let the user decide.
pub fn outdated(
    catalog: &Catalog,
    installed: &std::collections::BTreeMap<String, InstalledVersion>,
) -> Vec<PendingUpdate> {
    let mut out = Vec::new();
    for (id, have) in installed {
        let Some(card) = catalog.card(&ComponentId(id.clone())) else {
            continue; // No longer in the catalog: nothing to compare against.
        };
        let Some(variant) = card.variants.iter().find(|v| v.layer == have.layer) else {
            continue; // That layer no longer offers it.
        };
        if have.version.is_empty() || variant.version.is_empty() {
            continue;
        }
        if variant.version != have.version {
            out.push(PendingUpdate {
                id: ComponentId(id.clone()),
                layer: have.layer,
                installed_version: have.version.clone(),
                available_version: variant.version.clone(),
            });
        }
    }
    out
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
        /// How to order what comes back. Absent means catalog order, so an
        /// older caller keeps the results it used to get.
        #[serde(default)]
        sort: SortOrder,
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
    /// Which installed apps their own source now offers at a different version.
    /// The caller supplies what is installed, read from the install lock.
    Outdated {
        /// Component-id to what is installed, from the lock.
        installed: std::collections::BTreeMap<String, InstalledVersion>,
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
    /// The apps whose source offers a different version than the installed one.
    Updates(Vec<PendingUpdate>),
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
        Request::Search {
            query,
            facets,
            sort,
        } => {
            let mut cards = catalog.search(&query, &facets);
            if sort == SortOrder::LeastPrivilege {
                sort_least_privilege(&mut cards);
            }
            Response::Cards(cards)
        }
        Request::ListByFacet { facet } => Response::Cards(catalog.search("", &[facet])),
        Request::Outdated { installed } => Response::Updates(outdated(catalog, &installed)),
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
    use std::collections::BTreeMap;
    use crate::catalog::{merge_catalog, CapabilityFootprint, CatalogEntry, DisplayMeta, ItemKind};

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
            kind: ItemKind::default(),
            version: String::new(),
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

    /// A catalog where one app is versioned on two layers.
    fn versioned_catalog() -> Catalog {
        let mut a = entry("org.x.Chat", SourceLayer::Flatpak, "Chatter", &["network"]);
        a.version = "2.0".into();
        let mut b = entry("org.x.Chat", SourceLayer::Apt, "Chatter", &[]);
        b.version = "1.0".into();
        Catalog::new(merge_catalog(vec![a, b]))
    }

    fn installed(layer: SourceLayer, version: &str) -> BTreeMap<String, InstalledVersion> {
        [(
            "org.x.Chat".to_string(),
            InstalledVersion {
                layer,
                version: version.into(),
            },
        )]
        .into_iter()
        .collect()
    }

    #[test]
    fn a_newer_version_on_the_installed_layer_is_an_update() {
        let updates = outdated(&versioned_catalog(), &installed(SourceLayer::Apt, "0.9"));
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].installed_version, "0.9");
        assert_eq!(updates[0].available_version, "1.0");
        assert_eq!(updates[0].layer, SourceLayer::Apt);
    }

    /// The rule that keeps the user on the packaging they chose: Flathub offering
    /// 2.0 for an apt-installed app is a different variant with its own
    /// capabilities and trust, not an update. Offering it as one would walk them
    /// across a trust boundary they never agreed to.
    #[test]
    fn another_layers_version_is_not_an_update() {
        let updates = outdated(&versioned_catalog(), &installed(SourceLayer::Apt, "1.0"));
        assert!(
            updates.is_empty(),
            "flathub's 2.0 must not surface for an apt install: {updates:?}"
        );
    }

    /// A source that states no version must not make every app look changed, nor
    /// hide a real change behind a comparison against nothing.
    #[test]
    fn an_unstated_version_is_not_a_change() {
        // The catalog states one, the lock does not.
        assert!(outdated(&versioned_catalog(), &installed(SourceLayer::Apt, "")).is_empty());

        // The lock states one, the catalog does not.
        let unversioned = Catalog::new(merge_catalog(vec![entry(
            "org.x.Chat",
            SourceLayer::Apt,
            "Chatter",
            &[],
        )]));
        assert!(outdated(&unversioned, &installed(SourceLayer::Apt, "1.0")).is_empty());
    }

    #[test]
    fn an_app_the_catalog_dropped_is_not_reported() {
        let empty = Catalog::new(merge_catalog(vec![]));
        assert!(outdated(&empty, &installed(SourceLayer::Apt, "1.0")).is_empty());
    }

    #[test]
    fn nothing_installed_means_nothing_outdated() {
        assert!(outdated(&versioned_catalog(), &BTreeMap::new()).is_empty());
    }

    /// The op answers the same as the function, so a caller over the socket gets
    /// what a caller in-process does.
    #[test]
    fn the_outdated_op_answers_the_same_updates() {
        let answer = answer(
            &versioned_catalog(),
            Request::Outdated {
                installed: installed(SourceLayer::Apt, "0.9"),
            },
        );
        match answer {
            Response::Updates(u) => assert_eq!(u.len(), 1),
            other => panic!("expected updates, got {other:?}"),
        }
    }

    /// The point of declaring capabilities instead of inferring them: the app
    /// that asks for least can be found. Catalog order puts Chat first; asking
    /// for least-privilege puts Paint there, because Paint can be installed
    /// asking for nothing at all.
    #[test]
    fn least_privilege_puts_the_most_modest_app_first() {
        let mut cards = catalog().search("", &[]);
        assert_eq!(cards[0].id, ComponentId("org.x.Chat".into()), "catalog order");

        sort_least_privilege(&mut cards);
        assert_eq!(cards[0].id, ComponentId("org.y.Paint".into()));
        assert_eq!(cards[1].id, ComponentId("org.x.Chat".into()));
    }

    /// The cost is the MINIMUM over variants, not the maximum: Paint's Flatpak
    /// variant asks for the network, but its Official one asks for nothing, and
    /// the user is the one who picks. Ranking it by its worst packaging would
    /// bury an app that can be installed cleanly.
    #[test]
    fn the_cost_is_the_variant_the_user_could_choose() {
        let cards = catalog().search("paint", &[]);
        let paint = &cards[0];
        assert_eq!(paint.variants.len(), 2, "both variants merged onto one card");
        assert_eq!(privilege_cost(paint), 0);
    }

    /// A card with nothing to install must not top a least-privilege list by
    /// virtue of asking for nothing.
    #[test]
    fn a_card_with_no_variant_sorts_last() {
        let mut empty = catalog().search("paint", &[])[0].clone();
        empty.variants.clear();
        assert_eq!(privilege_cost(&empty), usize::MAX);

        let mut cards = catalog().search("", &[]);
        cards.push(empty.clone());
        sort_least_privilege(&mut cards);
        assert_eq!(cards.last().unwrap().variants.len(), 0);
    }

    /// The order is opt-in, so a caller that does not ask keeps what it got.
    #[test]
    fn the_default_order_is_unchanged() {
        let plain = answer(
            &catalog(),
            Request::Search {
                query: String::new(),
                facets: vec![],
                sort: SortOrder::default(),
            },
        );
        let sorted = answer(
            &catalog(),
            Request::Search {
                query: String::new(),
                facets: vec![],
                sort: SortOrder::LeastPrivilege,
            },
        );
        let ids = |r: Response| match r {
            Response::Cards(c) => c.into_iter().map(|c| c.id).collect::<Vec<_>>(),
            other => panic!("expected cards, got {other:?}"),
        };
        assert_eq!(ids(plain)[0], ComponentId("org.x.Chat".into()));
        assert_eq!(ids(sorted)[0], ComponentId("org.y.Paint".into()));
    }

    /// An older caller sends no `sort` at all; it must still parse.
    #[test]
    fn a_request_without_a_sort_field_still_parses() {
        let req: Request =
            serde_json::from_str(r#"{"Search":{"query":"x","facets":[]}}"#).expect("should parse");
        assert_eq!(
            req,
            Request::Search {
                query: "x".into(),
                facets: vec![],
                sort: SortOrder::Relevance,
            }
        );
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
            sort: SortOrder::LeastPrivilege,
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
