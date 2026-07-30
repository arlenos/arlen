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
