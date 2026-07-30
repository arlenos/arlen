//! SX-5: one model for everything that extends the system.
//!
//! Apps, modules and bridges are three different mechanisms with three
//! different manifests, but from the user's side they are one question: what
//! is installed, what did it get, and how do I take it back. Answering that
//! per type produces three surfaces that each look complete and none of which
//! is, which is what `shell-extension-model.md` means by folding bridge
//! management and the Activity view into ONE "what extends my system" surface.
//!
//! **The load-bearing part is the shared capability vocabulary.** A unified
//! view filters across sources, so if an app's profile said `network` while a
//! module's manifest said `networking`, a filter the user believes is
//! exhaustive would silently miss half the things that can reach the internet.
//! One vocabulary or the facet lies. That is the same reason the store's three
//! catalog sources emit one set of labels, and why the profile labeller moved
//! here rather than being reimplemented per consumer.
//!
//! The vocabulary is deliberately COARSE: `network`, `filesystem`,
//! `notifications`, `clipboard`, `system`, plus verbatim `read:`/`write:` graph
//! scopes. It answers "can this reach the internet", not "which hosts" - the
//! detail belongs on the thing's own page, where it can be read in context.

pub mod bridge;
pub mod module;
pub mod profile;

use serde::{Deserialize, Serialize};

/// Which mechanism an extension is. Kept explicit rather than inferred, because
/// revoking differs per kind: an app's grant lives in its profile, a module's in
/// the consent store, a bridge's in its delegated namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExtensionKind {
    /// An installed application.
    App,
    /// A shell module (Tier 1 WASM or Tier 2 iframe).
    Module,
    /// A foreign-app ingestion bridge.
    Bridge,
}

/// Whether an extension is currently doing anything.
///
/// Deliberately not a bool. "Installed but switched off" and "switched on but
/// crashed" are different things to a user looking for why something is not
/// working, and collapsing them loses exactly the distinction they came for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    /// Running, or ready to run.
    Active,
    /// Installed and permitted, but switched off by the user.
    Disabled,
    /// Failed, with what it said. Not a free-text catch-all: this is the state
    /// that needs a reason attached, because the user's next question is why.
    Failed(String),
    /// Not determinable from here. Honest absence rather than a guessed
    /// `Active` - a surface that renders "unknown" is correctable, one that
    /// renders a confident wrong answer is not.
    Unknown,
}

/// One row of the unified inventory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Extension {
    /// The stable id, unique within its kind.
    pub id: String,
    /// What to show the user.
    pub name: String,
    /// Which mechanism this is.
    pub kind: ExtensionKind,
    /// Coarse capability labels, sorted and deduped, in the ONE vocabulary
    /// every source emits. Empty means it asked for nothing, which is a real
    /// and meaningful answer - not "we could not tell", which is what an
    /// absent source would be.
    pub capabilities: Vec<String>,
    /// Where it came from (a cookbook, a remote, a local install), when known.
    pub provenance: Option<String>,
    /// What it is doing now.
    pub health: Health,
}

/// Merge per-source inventories into one, ordered so the surface is stable
/// across refreshes.
///
/// Sorted by kind then id rather than by source order: a list that reshuffles
/// when one source is slow to answer is a list the user cannot scan. An id may
/// legitimately repeat ACROSS kinds (an app and a bridge for the same foreign
/// program share a name), so the sort keys on both and nothing is deduped away.
pub fn merge(sources: impl IntoIterator<Item = Vec<Extension>>) -> Vec<Extension> {
    let mut all: Vec<Extension> = sources.into_iter().flatten().collect();
    all.sort_by(|a, b| {
        kind_order(a.kind)
            .cmp(&kind_order(b.kind))
            .then_with(|| a.id.cmp(&b.id))
    });
    all
}

/// Apps first, then modules, then bridges: roughly most to least familiar, so
/// the things a user recognises are not buried under machinery they did not
/// know they had.
fn kind_order(kind: ExtensionKind) -> u8 {
    match kind {
        ExtensionKind::App => 0,
        ExtensionKind::Module => 1,
        ExtensionKind::Bridge => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ext(id: &str, kind: ExtensionKind) -> Extension {
        Extension {
            id: id.to_string(),
            name: id.to_string(),
            kind,
            capabilities: Vec::new(),
            provenance: None,
            health: Health::Unknown,
        }
    }

    #[test]
    fn the_merged_order_is_stable_regardless_of_source_order() {
        let a = vec![ext("z.app", ExtensionKind::App)];
        let m = vec![ext("a.module", ExtensionKind::Module)];
        let b = vec![ext("a.bridge", ExtensionKind::Bridge)];

        let one = merge([a.clone(), m.clone(), b.clone()]);
        let two = merge([b, m, a]);
        assert_eq!(one, two, "the surface reshuffles when a source answers late");
        let ids: Vec<&str> = one.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["z.app", "a.module", "a.bridge"]);
    }

    /// An app and a bridge for the same foreign program legitimately share a
    /// name; dropping one would hide something the user has installed.
    #[test]
    fn the_same_id_in_two_kinds_is_kept_twice() {
        let merged = merge([
            vec![ext("md.obsidian", ExtensionKind::App)],
            vec![ext("md.obsidian", ExtensionKind::Bridge)],
        ]);
        assert_eq!(merged.len(), 2);
    }

    /// Failed carries its reason because "why is this not working" is the
    /// question that brought the user to the surface.
    #[test]
    fn health_distinguishes_switched_off_from_broken() {
        assert_ne!(Health::Disabled, Health::Failed("crashed".into()));
        assert_ne!(Health::Unknown, Health::Disabled);
    }
}
