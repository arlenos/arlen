//! An app's capability footprint, from its enrolled permission profile.
//!
//! A Debian package declares no capabilities - `.deb` has no manifest for them -
//! so the honest source is the permission profile the enrol hook writes when the
//! app is installed. That profile is what actually confines the app at runtime,
//! which makes it a better answer than anything inferred from the archive.
//!
//! The labels are deliberately the SAME coarse vocabulary the recipe path
//! (`compose::capability_labels`) and the Flatpak path (`flatpak::context_labels`)
//! emit - `network`, `filesystem`, `notifications`, `clipboard`, `audio`,
//! `system`, plus verbatim graph scopes. The store facets AND across sources, so
//! a card whose apt variant said "networking" while its Flathub variant said
//! "network" would silently fail to match a filter the user believes is
//! exhaustive. One vocabulary or the facet lies.

use std::collections::BTreeSet;

use arlen_permissions::PermissionProfile;

/// The coarse capability labels an enrolled profile implies.
///
/// Sorted and deduped, like its siblings, so a card's labels are stable across
/// renders and comparable between variants.
///
/// Absence of a grant is absence of a label - a profile that grants nothing
/// yields nothing, which is what makes the "asks for least" sort meaningful
/// rather than an artefact of which source had the richest metadata.
pub fn profile_labels(profile: &PermissionProfile) -> Vec<String> {
    let mut labels: BTreeSet<String> = BTreeSet::new();

    // Any network reach at all. `allow_all` and a host allowlist are different
    // magnitudes but the same coarse answer: this app can leave the machine.
    // The concrete hosts are an app-detail concern, per section 9.2.
    if profile.network.allow_all || !profile.network.allowed_domains.is_empty() {
        labels.insert("network".into());
    }

    // Any filesystem grant means the app reaches files outside its own sandbox,
    // which is the same threshold the Flatpak reader uses for `filesystems=`.
    let fs = &profile.filesystem;
    if fs.home
        || fs.documents
        || fs.downloads
        || fs.pictures
        || fs.music
        || fs.videos
        || !fs.custom.is_empty()
    {
        labels.insert("filesystem".into());
    }

    if profile.notifications.enabled {
        labels.insert("notifications".into());
    }
    // Either direction is clipboard reach. Read is the one that matters for
    // privacy, but an app that can WRITE the clipboard can also displace what
    // the user copied, so both earn the label.
    if profile.clipboard.read || profile.clipboard.write {
        labels.insert("clipboard".into());
    }

    // Autostart and background are what "runs without you asking" means; the
    // power grants are the ones that can suspend the machine out from under you.
    let sys = &profile.system;
    if sys.autostart || sys.background || sys.power.suspend || sys.power.set_profile {
        labels.insert("system".into());
    }

    // The four families below were missing, which mattered most for `input`: an
    // app holding a global binding sees keys pressed in other apps' windows, and
    // it showed nothing at all here - invisible on the one surface whose job is
    // saying what an app can do, and so unreachable by the revoke built on these
    // labels. The revoke vocabulary already had a reach for each of them.
    let input = &profile.input;
    if input.register_focused_bindings || input.register_global_bindings {
        labels.insert("input".into());
    }

    // Registering a handler puts an app in front of what the user types into the
    // launcher; intercepting all of it is stronger still.
    let search = &profile.search;
    if search.open || search.register_handler || search.intercept_all {
        labels.insert("search".into());
    }

    let intents = &profile.intents;
    if intents.dispatch || intents.register || intents.preferences {
        labels.insert("intents".into());
    }

    // Either direction is reach across the system's own event traffic.
    let bus = &profile.event_bus;
    if !bus.publish.is_empty() || !bus.subscribe.is_empty() {
        labels.insert("events".into());
    }

    // Graph scopes verbatim, matching the recipe path, so a `read:system.File`
    // facet matches an apt app exactly as it matches a forage one.
    for scope in &profile.graph.read {
        labels.insert(format!("read:{scope}"));
    }
    for scope in &profile.graph.write {
        labels.insert(format!("write:{scope}"));
    }

    labels.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> PermissionProfile {
        toml::from_str(
            "[info]\napp_id = \"org.example.App\"\nname = \"Example\"\ntier = \"third-party\"\n",
        )
        .expect("a minimal profile parses")
    }

    #[test]
    fn a_profile_that_grants_nothing_has_no_labels() {
        assert!(profile_labels(&profile()).is_empty());
    }

    #[test]
    fn a_host_allowlist_is_still_network_reach() {
        let mut p = profile();
        p.network.allowed_domains = vec!["api.example.com".into()];
        assert_eq!(profile_labels(&p), vec!["network".to_string()]);

        // As is allow_all, by the same coarse answer.
        let mut q = profile();
        q.network.allow_all = true;
        assert_eq!(profile_labels(&q), vec!["network".to_string()]);
    }

    /// Every flag in each family's OR chain has to earn the label ON ITS OWN.
    /// Mutation testing found this: the tests only ever set one flag per family,
    /// so flipping any `||` in these chains to `&&` survived - a real
    /// possibility for the families with four flags, where the label would then
    /// need all four before it appeared and an app granted only one would show
    /// nothing.
    #[test]
    fn each_grant_earns_its_family_label_alone() {
        type Set = fn(&mut PermissionProfile);
        let cases: &[(&str, Set)] = &[
            ("filesystem", |p| p.filesystem.home = true),
            ("filesystem", |p| p.filesystem.documents = true),
            ("filesystem", |p| p.filesystem.downloads = true),
            ("filesystem", |p| p.filesystem.pictures = true),
            ("filesystem", |p| p.filesystem.music = true),
            ("filesystem", |p| p.filesystem.videos = true),
            ("filesystem", |p| p.filesystem.custom = vec!["/opt/x".into()]),
            ("network", |p| p.network.allow_all = true),
            ("network", |p| p.network.allowed_domains = vec!["x.example".into()]),
            ("clipboard", |p| p.clipboard.read = true),
            ("clipboard", |p| p.clipboard.write = true),
            ("system", |p| p.system.autostart = true),
            ("system", |p| p.system.background = true),
            ("system", |p| p.system.power.suspend = true),
            ("system", |p| p.system.power.set_profile = true),
            ("input", |p| p.input.register_focused_bindings = true),
            ("input", |p| p.input.register_global_bindings = true),
            ("search", |p| p.search.open = true),
            ("search", |p| p.search.register_handler = true),
            ("search", |p| p.search.intercept_all = true),
            ("intents", |p| p.intents.dispatch = true),
            ("intents", |p| p.intents.register = true),
            ("intents", |p| p.intents.preferences = true),
            ("events", |p| p.event_bus.publish = vec!["a.*".into()]),
            ("events", |p| p.event_bus.subscribe = vec!["a.*".into()]),
        ];
        for (i, (label, set)) in cases.iter().enumerate() {
            let mut p = profile();
            set(&mut p);
            let labels = profile_labels(&p);
            assert!(
                labels.iter().any(|l| l == label),
                "case {i}: this single grant alone must show {label}, got {labels:?}"
            );
        }
    }

    #[test]
    fn any_filesystem_grant_earns_the_label_once() {
        let mut p = profile();
        p.filesystem.documents = true;
        p.filesystem.downloads = true;
        assert_eq!(profile_labels(&p), vec!["filesystem".to_string()]);
    }

    /// Writing the clipboard can displace what the user copied, so it counts too.
    #[test]
    fn either_clipboard_direction_counts() {
        let mut p = profile();
        p.clipboard.write = true;
        assert_eq!(profile_labels(&p), vec!["clipboard".to_string()]);
    }

    /// The vocabulary has to match the other two sources or the ANDed facets
    /// silently fail to match across variants of the same app.
    #[test]
    fn the_labels_match_the_vocabulary_the_other_sources_emit() {
        let mut p = profile();
        p.network.allow_all = true;
        p.filesystem.home = true;
        p.notifications.enabled = true;
        p.clipboard.read = true;
        p.system.autostart = true;
        p.graph.read = vec!["system.File".into()];

        let labels = profile_labels(&p);
        for expected in ["network", "filesystem", "notifications", "clipboard", "system"] {
            assert!(labels.contains(&expected.to_string()), "missing {expected}: {labels:?}");
        }
        assert!(labels.contains(&"read:system.File".to_string()), "{labels:?}");
    }

    #[test]
    fn the_labels_are_sorted_and_deduped() {
        let mut p = profile();
        p.filesystem.home = true;
        p.filesystem.music = true;
        p.network.allow_all = true;
        let labels = profile_labels(&p);
        let mut sorted = labels.clone();
        sorted.sort();
        assert_eq!(labels, sorted);
        assert_eq!(labels.len(), 2, "{labels:?}");
    }
}
