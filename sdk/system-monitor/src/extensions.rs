//! One inventory of everything that extends the system (`shell-extension-model.md`
//! SX-5): apps, modules and bridges in a single list, so the management surface is
//! one surface rather than three that each know about a third of the answer.
//!
//! The three sources have nothing in common structurally - an app comes from
//! installd's lock, a module from modulesd's summary, a bridge from its
//! `bridge.toml` - so the shared vocabulary lives here and each source converts its
//! own inventory into it. That direction matters: a converter belongs with the type
//! it converts, where a field's meaning is known, and putting all three here would
//! make this crate depend on all three daemons.
//!
//! What this module owns is the part that is easy to get wrong once the rows are
//! side by side: identity across kinds, and how health is ordered.

use std::collections::BTreeSet;

/// What kind of thing extends the system.
///
/// Part of an extension's IDENTITY, not a label on it. A module id and an app id
/// are drawn from different namespaces and may legitimately be the same string, so
/// a list keyed on the id alone would merge two unrelated things - and the merged
/// row would claim one's capabilities under the other's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ExtensionKind {
    /// An installed application.
    App,
    /// A shell module (Tier 1 WASM or Tier 2 iframe).
    Module,
    /// A foreign-app bridge.
    Bridge,
}

impl ExtensionKind {
    /// The stable lowercase key a surface can group or filter on.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Module => "module",
            Self::Bridge => "bridge",
        }
    }
}

/// Whether an extension is working, as far as anything can tell.
///
/// `Unknown` is a distinct state rather than an optimistic `Ok`, because the three
/// sources differ in what they can report: modulesd tracks crashes, installd does
/// not watch an app after installing it, and a bridge only looks unhealthy once
/// something tries to use it. A surface that rendered "no known problem" as a green
/// tick would be claiming a check that never ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Health {
    /// Running, or at least nothing has reported otherwise.
    Ok,
    /// Known broken, with whatever the source could say about why.
    Failed {
        /// The last error the source recorded, when it recorded one.
        reason: Option<String>,
    },
    /// Nothing reports on this extension's health.
    Unknown,
}

impl Health {
    /// The health a crash-reporting source states, from its two fields.
    ///
    /// `last_error` is only meaningful alongside `failed`; a source reporting a
    /// reason without the flag means the thing recovered, and the reason belongs
    /// to a past failure rather than the current state, so it is dropped.
    pub fn from_failure(failed: bool, last_error: Option<String>) -> Self {
        if failed {
            Self::Failed { reason: last_error }
        } else {
            Self::Ok
        }
    }

    /// Ordering for a surface that leads with what needs attention: failures
    /// first, then unknowns, then healthy.
    ///
    /// Unknown sorts ABOVE Ok deliberately. "We cannot tell" is closer to a
    /// problem than to a clean bill of health, and burying it under everything
    /// that is fine is how an unmonitored extension stays unnoticed.
    fn attention_rank(&self) -> u8 {
        match self {
            Self::Failed { .. } => 0,
            Self::Unknown => 1,
            Self::Ok => 2,
        }
    }
}

/// One row of the unified inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extension {
    /// The id within its kind. For a bridge this is the `bridge.<id>` app id it
    /// writes under, so it matches what audit and the LCG record for it.
    pub id: String,
    /// Which of the three this is; with `id`, the identity.
    pub kind: ExtensionKind,
    /// Display name, or the id when the source has no better one.
    pub name: String,
    /// The version the source states, when it states one. Bridges have none.
    pub version: Option<String>,
    /// Whether it is currently active. An app is enabled by being installed; a
    /// module carries an explicit flag.
    pub enabled: bool,
    /// What it may do, as the human-readable reach strings the App-access page
    /// already renders. Empty means it holds no capability, NOT that its
    /// capabilities are unknown - a source that cannot say leaves the row out
    /// rather than reporting an empty grant.
    pub granted: Vec<String>,
    /// Where it came from: an install layer, a module tier, a bridge's plugin.
    pub provenance: String,
    /// Whether it is working.
    pub health: Health,
}

impl Extension {
    /// The identity a surface addresses this row by, stable across refreshes.
    ///
    /// Kind-prefixed so it survives being put in one flat map with the other two
    /// kinds, which is exactly what a unified surface does.
    pub fn key(&self) -> String {
        format!("{}:{}", self.kind.as_str(), self.id)
    }
}

impl Extension {
    /// An installed application.
    ///
    /// Health is [`Health::Unknown`] and not a parameter, because nothing watches
    /// an app after installing it. A constructor that accepted a health here would
    /// invite a caller to pass `Ok` meaning "the install succeeded", which is a
    /// different claim and the one that turns into a false green tick.
    pub fn app(
        id: impl Into<String>,
        name: impl Into<String>,
        version: Option<String>,
        source_layer: impl Into<String>,
        granted: Vec<String>,
    ) -> Self {
        Self {
            id: id.into(),
            kind: ExtensionKind::App,
            name: name.into(),
            version,
            // Installed is active; an app has no separate enabled flag.
            enabled: true,
            granted,
            provenance: source_layer.into(),
            health: Health::Unknown,
        }
    }

    /// A shell module, whose runtime is the one source that reports crashes.
    ///
    /// Takes a [`Health`] rather than the runtime's `failed` + `last_error` pair,
    /// which [`Health::from_failure`] converts. Two adjacent booleans in a
    /// positional call - `enabled` and `failed` - are a swap waiting to happen,
    /// and the swapped row would report a working module as broken and a broken
    /// one as fine.
    pub fn module(
        id: impl Into<String>,
        name: impl Into<String>,
        version: Option<String>,
        tier: impl Into<String>,
        enabled: bool,
        health: Health,
        granted: Vec<String>,
    ) -> Self {
        Self {
            id: id.into(),
            kind: ExtensionKind::Module,
            name: name.into(),
            version,
            enabled,
            // `ModuleSummary::granted`, which modulesd fills from the same
            // `describe` the consent dialog used. So a module's row here reads
            // back the words the user was actually asked to approve.
            granted,
            provenance: tier.into(),
            health,
        }
    }

    /// A foreign-app bridge, addressed by the `bridge.<id>` app id it writes
    /// under.
    ///
    /// The prefix is applied HERE rather than expected from the caller, so the
    /// unified row always matches what audit and the LCG record for that bridge.
    /// A caller passing an already-prefixed id gets it back unchanged, since
    /// prefixing twice would address a bridge that does not exist.
    ///
    /// **The same rule lives in `bridge-ingest`'s `BridgeMeta::app_id`**, which is
    /// where a bridge's real identity is minted. The two crates share no
    /// dependency and this is one string, so it is duplicated on purpose rather
    /// than given a crate of its own; both sides say so. If that prefix ever
    /// changes, it changes in both or this inventory names bridges that do not
    /// exist.
    pub fn bridge(
        id: impl Into<String>,
        namespace: impl Into<String>,
        granted: Vec<String>,
    ) -> Self {
        let raw = id.into();
        let app_id = if raw.starts_with("bridge.") {
            raw.clone()
        } else {
            format!("bridge.{raw}")
        };
        Self {
            id: app_id,
            kind: ExtensionKind::Bridge,
            name: raw,
            version: None,
            enabled: true,
            granted,
            provenance: namespace.into(),
            // A bridge only looks unhealthy once something tries to use it, and
            // nothing polls it, so its standing state is genuinely unknown.
            health: Health::Unknown,
        }
    }
}

/// Merge the three inventories into one list.
///
/// Ordered for a management surface: what needs attention first, then by kind, then
/// by id, so the list is deterministic and a refresh does not reshuffle rows under
/// the reader's cursor.
///
/// **Rows are never merged across kinds**, even when they share an id. A module and
/// an app with the same string are two things, and combining them would attribute
/// one's capabilities to the other. Within a kind a repeated id IS a duplicate -
/// the same extension reported twice by one source - and the first is kept, since a
/// source listing something twice has no second opinion to offer.
pub fn unify(rows: impl IntoIterator<Item = Extension>) -> Vec<Extension> {
    let mut seen: BTreeSet<(ExtensionKind, String)> = BTreeSet::new();
    let mut out: Vec<Extension> = Vec::new();
    for row in rows {
        if seen.insert((row.kind, row.id.clone())) {
            out.push(row);
        }
    }
    out.sort_by(|a, b| {
        a.health
            .attention_rank()
            .cmp(&b.health.attention_rank())
            .then(a.kind.cmp(&b.kind))
            .then(a.id.cmp(&b.id))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(kind: ExtensionKind, id: &str, health: Health) -> Extension {
        Extension {
            id: id.to_string(),
            kind,
            name: id.to_string(),
            version: None,
            enabled: true,
            granted: Vec::new(),
            provenance: String::new(),
            health,
        }
    }

    #[test]
    fn an_app_and_a_module_sharing_an_id_stay_two_rows() {
        // The failure this prevents: one row claiming the other's capabilities
        // under its name. The ids come from different namespaces, so a collision
        // is legitimate rather than a mistake to resolve.
        let got = unify([
            row(ExtensionKind::App, "com.example.thing", Health::Ok),
            row(ExtensionKind::Module, "com.example.thing", Health::Ok),
        ]);
        assert_eq!(got.len(), 2);
        assert_ne!(got[0].key(), got[1].key());
    }

    #[test]
    fn one_source_listing_the_same_thing_twice_yields_one_row() {
        let got = unify([
            row(ExtensionKind::Module, "a", Health::Ok),
            row(ExtensionKind::Module, "a", Health::Ok),
        ]);
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn what_needs_attention_comes_first_and_unknown_beats_healthy() {
        // Unknown above Ok is the point: an extension nothing monitors must not
        // sort in with the ones that are confirmed fine.
        let got = unify([
            row(ExtensionKind::App, "healthy", Health::Ok),
            row(ExtensionKind::App, "unwatched", Health::Unknown),
            row(ExtensionKind::App, "broken", Health::Failed { reason: None }),
        ]);
        let ids: Vec<&str> = got.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["broken", "unwatched", "healthy"]);
    }

    #[test]
    fn the_order_is_stable_across_refreshes() {
        // A surface refreshing under the reader's cursor must not reshuffle. The
        // same set in any input order sorts the same way.
        let a = unify([
            row(ExtensionKind::Bridge, "bridge.md", Health::Ok),
            row(ExtensionKind::App, "z", Health::Ok),
            row(ExtensionKind::Module, "m", Health::Ok),
        ]);
        let b = unify([
            row(ExtensionKind::Module, "m", Health::Ok),
            row(ExtensionKind::Bridge, "bridge.md", Health::Ok),
            row(ExtensionKind::App, "z", Health::Ok),
        ]);
        assert_eq!(a, b);
    }

    #[test]
    fn a_key_survives_being_put_in_one_flat_map() {
        let app = row(ExtensionKind::App, "x", Health::Ok);
        let module = row(ExtensionKind::Module, "x", Health::Ok);
        assert_eq!(app.key(), "app:x");
        assert_eq!(module.key(), "module:x");
    }

    #[test]
    fn a_bridge_is_addressed_by_the_id_audit_records_for_it() {
        let b = Extension::bridge("md.obsidian", "md.obsidian", vec!["write md.obsidian.*".into()]);
        assert_eq!(b.id, "bridge.md.obsidian");
        // The display name stays the bare id; only the address is prefixed.
        assert_eq!(b.name, "md.obsidian");
    }

    #[test]
    fn the_bridge_prefix_matches_what_bridge_ingest_mints() {
        // Pinned against the literal rather than against the other crate, which
        // this one cannot see. If `BridgeMeta::app_id` ever stops producing
        // `bridge.<id>`, this test still passes and the doc note on both sides is
        // what catches it - so the note is the mechanism and this only fixes the
        // shape it has today.
        assert_eq!(Extension::bridge("md.obsidian", "", Vec::new()).id, "bridge.md.obsidian");
    }

    #[test]
    fn prefixing_an_already_prefixed_bridge_id_would_address_nothing() {
        let b = Extension::bridge("bridge.md.obsidian", "md.obsidian", Vec::new());
        assert_eq!(b.id, "bridge.md.obsidian");
    }

    #[test]
    fn an_apps_health_is_unknown_rather_than_ok() {
        // Nothing watches an app after install, so claiming Ok would be claiming
        // a check that never ran. This is why `app` takes no health argument.
        let a = Extension::app("com.example.notes", "Notes", Some("1.2".into()), "apt", Vec::new());
        assert_eq!(a.health, Health::Unknown);
        assert!(a.enabled, "installed is active");
    }

    #[test]
    fn a_crashed_module_carries_what_it_said() {
        let m = Extension::module(
            "com.example.clock", "Clock", None, "wasm", true,
            Health::from_failure(true, Some("trap at instantiate".into())), Vec::new(),
        );
        assert_eq!(m.health, Health::Failed { reason: Some("trap at instantiate".into()) });
        let ok = Extension::module(
            "com.example.clock", "Clock", None, "wasm", true,
            Health::from_failure(false, None), Vec::new(),
        );
        assert_eq!(ok.health, Health::Ok);
    }

    #[test]
    fn a_reason_without_a_failure_flag_is_a_past_failure_not_a_current_one() {
        // A source that reports a reason but says it is not failed has recovered.
        // Keeping the reason would render a working module as broken forever.
        assert_eq!(Health::from_failure(false, Some("old crash".into())), Health::Ok);
        assert_eq!(
            Health::from_failure(true, Some("now".into())),
            Health::Failed { reason: Some("now".into()) }
        );
        // Failed with nothing to say is still failed.
        assert_eq!(Health::from_failure(true, None), Health::Failed { reason: None });
    }

    #[test]
    fn a_module_carries_the_grants_the_user_approved() {
        // The summary now carries them, filled from the same `describe` the
        // consent dialog used, so this row shows what was actually asked for
        // rather than a second wording of it.
        let m = Extension::module(
            "a", "A", None, "iframe", true, Health::Ok,
            vec!["connect to api.example.com".into()],
        );
        assert_eq!(m.granted, ["connect to api.example.com"]);

        // And an empty list still means "declares nothing", which is now the
        // truth for a module rather than a gap: the summary is built from the
        // manifest, so there is always an answer.
        let none = Extension::module("b", "B", None, "wasm", true, Health::Ok, Vec::new());
        assert!(none.granted.is_empty());
    }

    #[test]
    fn an_empty_grant_list_is_not_the_same_as_an_unknown_one() {
        // Documented contract: a source that cannot report capabilities omits the
        // row rather than reporting it with nothing granted, so an empty list can
        // be rendered as "holds nothing" without lying.
        let r = row(ExtensionKind::Module, "a", Health::Ok);
        assert!(r.granted.is_empty());
    }
}
