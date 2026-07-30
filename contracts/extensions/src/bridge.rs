//! A bridge's capability footprint, in the shared vocabulary.
//!
//! A bridge is the narrowest of the three by construction. It holds exactly one
//! delegated namespace, writes only within it (enforced at the daemon's grant
//! check and refused at config load), and reaches nothing else - no network of
//! its own, no filesystem grants, no clipboard. So its footprint is one
//! `write:` scope, and that honest narrowness is worth surfacing: in a list
//! sorted by what things asked for, a bridge should sit at the modest end
//! rather than looking unmeasured.

use std::collections::BTreeSet;

/// The coarse capability labels a bridge's delegated namespace implies.
///
/// `namespace` is the bridge's `[bridge] id`, which is also what it may write.
/// The label is the namespace itself rather than any concrete type under it,
/// because the bridge's authority IS the whole subtree - naming one type would
/// understate it.
///
/// An empty or reserved namespace yields NOTHING rather than a `write:`
/// label. A bridge cannot hold such a grant (`NamespaceGrant::new` refuses
/// `system` and `shared`, and the config refuses to load), so emitting a label
/// for one would render authority in the inventory that does not exist -
/// worse than rendering none, because it reads as confirmed.
pub fn bridge_labels(namespace: &str) -> Vec<String> {
    let ns = namespace.trim();
    if ns.is_empty() || is_reserved(ns) {
        return Vec::new();
    }
    let mut labels: BTreeSet<String> = BTreeSet::new();
    labels.insert(format!("write:{ns}"));
    labels.into_iter().collect()
}

/// The namespaces no bridge can hold, mirroring the daemon's grant floor.
fn is_reserved(ns: &str) -> bool {
    ns.split('.')
        .next()
        .is_some_and(|first| matches!(first, "system" | "shared"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bridge_carries_its_delegated_namespace_as_a_write_scope() {
        assert_eq!(
            bridge_labels("md.obsidian"),
            vec!["write:md.obsidian".to_string()]
        );
    }

    /// A bridge cannot hold a reserved grant, so the inventory must not render
    /// authority for one - a confirmed-looking wrong answer is worse than none.
    #[test]
    fn a_reserved_or_empty_namespace_yields_no_authority() {
        for ns in ["", "  ", "system", "shared", "system.core"] {
            assert!(
                bridge_labels(ns).is_empty(),
                "{ns:?} rendered authority a bridge cannot hold"
            );
        }
    }

    /// The `write:` prefix has to match what the other two sources emit, or a
    /// scope filter finds apps and modules but never bridges.
    #[test]
    fn the_scope_prefix_matches_the_other_sources() {
        let mut p: arlen_permissions::PermissionProfile = toml::from_str(
            "[info]\napp_id = \"org.example.App\"\nname = \"Example\"\ntier = \"third-party\"\n",
        )
        .expect("a minimal profile parses");
        p.graph.write = vec!["md.obsidian".into()];
        assert_eq!(crate::profile::profile_labels(&p), bridge_labels("md.obsidian"));
    }
}
