//! Map a recipe's declared `[capabilities]` to an enforceable `PermissionProfile`.
//!
//! F-R2 first step (forage-recipes.md section 11a): a recipe declares
//! capabilities in reviewable, human terms; the runtime enforces a
//! `PermissionProfile`. This is the pure translation between the two, and it is
//! deliberately conservative. A known capability maps to the narrowest matching
//! grant; an unrecognised filesystem scope becomes a literal `custom` path
//! rather than widening to a standard directory; a graph scope without a
//! `read:`/`write:` prefix grants nothing. A capability with no profile
//! counterpart at all (`audio`, or any open `extra` category) is never silently
//! dropped: it is returned in [`CapabilityMapping::unmapped`] so the caller can
//! surface it. Nothing here widens access beyond what the recipe spelled out.
//!
//! This mapper is content-neutral: a filesystem scope of `/` or `/etc` is
//! faithfully passed through as a `custom` path, because that is what the recipe
//! literally declared. Treating such a declaration as suspect (a package asking
//! for `/`) is the job of the recipe-review and install-gate layer, not this
//! translation; the mapper must not invent reach, but it also must not hide what
//! the author asked for.

use std::path::PathBuf;

use arlen_forage_recipe::Capabilities;
use arlen_permissions::{
    AppTier, ClipboardPermissions, FilesystemPermissions, GraphPermissions, NetworkPermissions,
    NotificationPermissions, PermissionProfile, ProfileInfo,
};

/// The result of mapping recipe capabilities: the enforceable profile plus any
/// declared capability that has no profile counterpart (preserved, not dropped).
#[derive(Debug, Clone)]
pub struct CapabilityMapping {
    /// The enforceable permission profile.
    pub profile: PermissionProfile,
    /// Declared capabilities with no profile counterpart, e.g. `audio`, an
    /// unrecognised `extra` category, or a malformed graph scope. Surfaced so a
    /// declaration is never silently lost.
    pub unmapped: Vec<String>,
}

/// Map `caps` to a `PermissionProfile` for `app_id` at `tier`.
pub fn capabilities_to_profile(
    app_id: &str,
    tier: AppTier,
    caps: &Capabilities,
) -> CapabilityMapping {
    let mut unmapped = Vec::new();

    let filesystem = map_filesystem(&caps.filesystem);
    let network = map_network(&caps.network, &mut unmapped);
    let graph = map_graph(&caps.graph, &mut unmapped);

    let clipboard = if caps.clipboard {
        ClipboardPermissions {
            read: true,
            write: true,
            ..Default::default()
        }
    } else {
        ClipboardPermissions::default()
    };

    // Capabilities with no profile counterpart: preserve, do not drop.
    if caps.audio {
        unmapped.push("audio".to_string());
    }
    for key in caps.extra.keys() {
        unmapped.push(key.clone());
    }

    let profile = PermissionProfile {
        info: ProfileInfo {
            app_id: app_id.to_string(),
            tier,
        },
        graph,
        event_bus: Default::default(),
        filesystem,
        network,
        notifications: NotificationPermissions {
            enabled: caps.notifications,
        },
        clipboard,
        system: Default::default(),
        input: Default::default(),
        search: Default::default(),
        intents: Default::default(),
        mcp: Default::default(),
    };
    CapabilityMapping { profile, unmapped }
}

/// Map filesystem scope names: known XDG directory names set their flag; any
/// other string (a path, or an unrecognised name) becomes a `custom` entry, so
/// it grants only that literal path and never silently widens to a standard
/// directory.
fn map_filesystem(scopes: &[String]) -> FilesystemPermissions {
    let mut fs = FilesystemPermissions::default();
    for s in scopes {
        match s.to_ascii_lowercase().as_str() {
            "home" => fs.home = true,
            "documents" => fs.documents = true,
            "downloads" => fs.downloads = true,
            "pictures" => fs.pictures = true,
            "music" => fs.music = true,
            "videos" => fs.videos = true,
            _ => fs.custom.push(PathBuf::from(s)),
        }
    }
    fs
}

/// Map network `host:port` scopes to allowed domains. A bare `*` (or `*:*`)
/// grants all; otherwise the host (a trailing numeric `:port` stripped) is added
/// to the allow-list. A malformed entry adds a domain that simply matches
/// nothing, never `allow_all`.
fn map_network(scopes: &[String], unmapped: &mut Vec<String>) -> NetworkPermissions {
    let mut net = NetworkPermissions::default();
    for s in scopes {
        if s == "*" || s == "*:*" {
            net.allow_all = true;
            continue;
        }
        let host = match s.rsplit_once(':') {
            Some((h, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => h,
            _ => s.as_str(),
        };
        if host.is_empty() {
            // A scope with no host (`:443`, ``) grants nothing; surface it rather
            // than drop it silently, matching the graph mapper's contract.
            unmapped.push(format!("network:{s}"));
        } else {
            net.allowed_domains.push(host.to_string());
        }
    }
    net
}

/// Map graph scopes `read:Type` / `write:Type` to read/write grants. A scope
/// without a recognised prefix, or with an empty type, grants nothing and is
/// recorded as unmapped rather than dropped.
fn map_graph(scopes: &[String], unmapped: &mut Vec<String>) -> GraphPermissions {
    use arlen_forage_recipe::GraphScope;
    let mut g = GraphPermissions::default();
    for s in scopes {
        match arlen_forage_recipe::parse_graph_scope(s) {
            Some(GraphScope::Read(t)) => g.read.push(t),
            Some(GraphScope::Write(t)) => g.write.push(t),
            None => unmapped.push(format!("graph:{s}")),
        }
    }
    g
}

/// One capability that changed between an installed recipe and an update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityChange {
    /// Human description, e.g. `network host api.example.com`.
    pub description: String,
    /// Whether this addition is high-impact and must block on explicit consent
    /// before the update proceeds (the S16 "high-impact always confirm"
    /// doctrine applied to capability changes). Low-impact additions are granted
    /// at the ceiling and prompt on first use instead.
    pub high_impact: bool,
}

/// The capability delta of an upgrade: declared additions (some consent-gated)
/// and removals (narrowing, applied freely).
#[derive(Debug, Clone, Default)]
pub struct CapabilityDiff {
    /// Capabilities the new version declares that the old did not.
    pub added: Vec<CapabilityChange>,
    /// Capabilities the old version declared that the new dropped (narrowing).
    pub removed: Vec<String>,
}

impl CapabilityDiff {
    /// Whether the upgrade must block on explicit consent: any high-impact
    /// addition (a new network host, new filesystem access, or a new graph
    /// write) requires confirmation before the update proceeds.
    pub fn requires_consent(&self) -> bool {
        self.added.iter().any(|c| c.high_impact)
    }

    /// Whether nothing changed.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

/// Diff two declared capability sets (the installed recipe vs an update,
/// forage-recipes.md section 11). Additions are classified high-impact per the
/// S16 doctrine: a new network host, any new filesystem access, and a new graph
/// write block on consent; new graph reads and the notification/clipboard/audio
/// flags are low-impact (prompt-on-use). Removals always narrow freely. The
/// classification errs toward high-impact, so the gate can only over-prompt,
/// never silently admit a privilege widening.
pub fn diff_capabilities(old: &Capabilities, new: &Capabilities) -> CapabilityDiff {
    let mut added = Vec::new();
    let mut removed = Vec::new();

    diff_list(&old.network, &new.network, "network host", true, &mut added, &mut removed);
    diff_list(&old.filesystem, &new.filesystem, "filesystem", true, &mut added, &mut removed);
    diff_graph(&old.graph, &new.graph, &mut added, &mut removed);

    diff_flag(old.notifications, new.notifications, "notifications", &mut added, &mut removed);
    diff_flag(old.clipboard, new.clipboard, "clipboard", &mut added, &mut removed);
    diff_flag(old.audio, new.audio, "audio", &mut added, &mut removed);

    // Unrecognised `extra` categories: a newly-declared one is conservatively
    // high-impact (its reach is unknown, so it cannot be assessed as safe) and
    // must not bypass the consent gate by being unmodelled. Removal narrows.
    for key in new.extra.keys() {
        if !old.extra.contains_key(key) {
            added.push(CapabilityChange {
                description: format!("capability {key}"),
                high_impact: true,
            });
        }
    }
    for key in old.extra.keys() {
        if !new.extra.contains_key(key) {
            removed.push(format!("capability {key}"));
        }
    }

    CapabilityDiff { added, removed }
}

/// Diff a string-list capability category with a fixed impact for additions.
fn diff_list(
    old: &[String],
    new: &[String],
    label: &str,
    high_impact: bool,
    added: &mut Vec<CapabilityChange>,
    removed: &mut Vec<String>,
) {
    for s in new {
        if old.contains(s) {
            continue;
        }
        let description = format!("{label} {s}");
        if !added.iter().any(|c| c.description == description) {
            added.push(CapabilityChange { description, high_impact });
        }
    }
    for s in old {
        if !new.contains(s) {
            let description = format!("{label} {s}");
            if !removed.contains(&description) {
                removed.push(description);
            }
        }
    }
}

/// Diff graph scopes: a new `write:` is high-impact, anything else (a read, or a
/// malformed scope that grants nothing) is low-impact.
fn diff_graph(
    old: &[String],
    new: &[String],
    added: &mut Vec<CapabilityChange>,
    removed: &mut Vec<String>,
) {
    for s in new {
        if old.contains(s) {
            continue;
        }
        let description = format!("graph {s}");
        if !added.iter().any(|c| c.description == description) {
            added.push(CapabilityChange {
                description,
                // Deliberately the same case-sensitive `write:` test that
                // `map_graph` uses to grant a write. The two must stay coupled:
                // anything the diff treats as not-a-write, the mapper also
                // refuses to grant as a write, so a `"WRITE:"` typo grants
                // nothing and correctly needs no write consent. Do not make one
                // side case-insensitive without the other.
                high_impact: s.starts_with("write:"),
            });
        }
    }
    for s in old {
        if !new.contains(s) {
            let description = format!("graph {s}");
            if !removed.contains(&description) {
                removed.push(description);
            }
        }
    }
}

/// Diff a boolean capability flag. A newly-true flag is a low-impact addition
/// (prompt-on-use); a newly-false flag is a removal.
fn diff_flag(
    old: bool,
    new: bool,
    label: &str,
    added: &mut Vec<CapabilityChange>,
    removed: &mut Vec<String>,
) {
    match (old, new) {
        (false, true) => added.push(CapabilityChange {
            description: label.to_string(),
            high_impact: false,
        }),
        (true, false) => removed.push(label.to_string()),
        _ => {}
    }
}

/// The capabilities a recipe declares that exceed a curated cap.
///
/// A cookbook may sign a capability cap, a curated upper bound on what the
/// in-repo recipe is allowed to declare (forage-recipes.md section 7a: "the
/// recipe may declare at most these capabilities"). This returns every declared
/// capability that is not within the cap; an empty result means the recipe is
/// within bounds. The check is a strict subset (not the consent heuristic):
/// each allowlist entry must be present in the cap, each requested boolean must
/// be granted by the cap, and each `extra` category must match the cap's value
/// exactly. The caller refuses an install whose recipe exceeds the cap.
pub fn cap_exceeded(recipe: &Capabilities, cap: &Capabilities) -> Vec<String> {
    let mut over = Vec::new();
    for host in &recipe.network {
        if !cap.network.contains(host) {
            over.push(format!("network {host}"));
        }
    }
    for scope in &recipe.filesystem {
        if !cap.filesystem.contains(scope) {
            over.push(format!("filesystem {scope}"));
        }
    }
    for g in &recipe.graph {
        if !cap.graph.contains(g) {
            over.push(format!("graph {g}"));
        }
    }
    if recipe.notifications && !cap.notifications {
        over.push("notifications".to_string());
    }
    if recipe.clipboard && !cap.clipboard {
        over.push("clipboard".to_string());
    }
    if recipe.audio && !cap.audio {
        over.push("audio".to_string());
    }
    // An unknown `extra` category counts as exceeding unless the cap declares it
    // with the same value: an unrecognised category cannot be proven within an
    // upper bound, so it fails closed.
    for (key, value) in &recipe.extra {
        match cap.extra.get(key) {
            Some(cap_value) if cap_value == value => {}
            _ => over.push(format!("extra.{key}")),
        }
    }
    over
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps() -> Capabilities {
        Capabilities::default()
    }

    /// Every dimension of the cap check, in BOTH directions: a recipe within the
    /// cap must draw no complaint, and one that exceeds it must draw exactly one.
    ///
    /// Mutation testing found each `!` in `cap_exceeded` could be DELETED
    /// undetected - which inverts that dimension's verdict, so a recipe staying
    /// inside its signed cap would be refused and one exceeding it would install.
    /// The surviving mutants say the within-cap direction was never tested, and
    /// this is the check that stops an install exceeding what the cookbook signed.
    #[test]
    fn each_dimension_of_the_cap_is_checked_in_both_directions() {
        type Set = fn(&mut Capabilities);
        let dims: &[(&str, Set)] = &[
            ("network", |c| c.network = vec!["api.example.com".into()]),
            ("filesystem", |c| c.filesystem = vec!["home".into()]),
            ("graph", |c| c.graph = vec!["read:system.File".into()]),
            ("notifications", |c| c.notifications = true),
            ("clipboard", |c| c.clipboard = true),
            ("audio", |c| c.audio = true),
            ("extra", |c| {
                c.extra.insert("input:global".into(), toml::Value::Boolean(true));
            }),
        ];
        for (name, set) in dims {
            let mut recipe = caps();
            set(&mut recipe);

            // Within: the cap grants exactly what the recipe asks.
            let mut cap = caps();
            set(&mut cap);
            assert!(
                cap_exceeded(&recipe, &cap).is_empty(),
                "{name}: a recipe inside its cap must not be reported as exceeding"
            );

            // Exceeds: the cap grants nothing of this dimension.
            let over = cap_exceeded(&recipe, &caps());
            assert_eq!(
                over.len(),
                1,
                "{name}: asking for what the cap withholds must be reported once, got {over:?}"
            );
        }
    }

    /// Every named directory maps to its own flag and none falls through to
    /// `custom`. Mutation testing found four of the six arms could be deleted
    /// undetected, which silently turns a declared directory grant into a
    /// nonsense custom path - the app declares `downloads` and gets a relative
    /// path called "downloads" instead.
    #[test]
    fn each_named_directory_maps_to_its_own_flag() {
        type Get = fn(&FilesystemPermissions) -> bool;
        let dirs: &[(&str, Get)] = &[
            ("home", |f| f.home),
            ("documents", |f| f.documents),
            ("downloads", |f| f.downloads),
            ("pictures", |f| f.pictures),
            ("music", |f| f.music),
            ("videos", |f| f.videos),
        ];
        for (name, get) in dirs {
            let fs = map_filesystem(&[name.to_string()]);
            assert!(get(&fs), "{name} must set its own flag");
            assert!(fs.custom.is_empty(), "{name} must not fall through to custom");
        }
        // And something that is not a named directory still does.
        let fs = map_filesystem(&["/opt/data".to_string()]);
        assert_eq!(fs.custom, vec![PathBuf::from("/opt/data")]);
    }

    /// A `host:port` scope keeps only the host, but only when the port really is
    /// one. Mutation testing found the guard could be replaced with `true` and its
    /// `&&` with `||`, either of which would strip a trailing colon-segment that
    /// is not a port - turning `db.example.com:replica` into `db.example.com`,
    /// silently widening the allowlist entry to the whole host.
    #[test]
    fn only_a_real_port_is_stripped_from_a_host_scope() {
        let mut unmapped = Vec::new();
        let net = map_network(&["api.example.com:443".to_string()], &mut unmapped);
        assert_eq!(net.allowed_domains, vec!["api.example.com".to_string()]);

        for not_a_port in ["db.example.com:replica", "db.example.com:", "db.example.com:8a"] {
            let net = map_network(&[not_a_port.to_string()], &mut unmapped);
            assert_eq!(
                net.allowed_domains,
                vec![not_a_port.to_string()],
                "{not_a_port} has no port to strip, so it stays whole"
            );
        }
    }

    /// Additions and removals are found in both directions, and a repeat is not
    /// double-counted. Mutation testing found the `!` in each membership check and
    /// the dedup comparison could be flipped, which changes what
    /// `requires_consent` sees - the difference between prompting and not.
    #[test]
    fn the_diff_finds_each_side_once_and_only_once() {
        let mut old = caps();
        old.graph = vec!["read:system.File".into()];
        old.network = vec!["old.example".into()];
        let mut new = caps();
        // One graph scope kept, one added; the network entry dropped.
        new.graph = vec!["read:system.File".into(), "write:md.note.Note".into()];

        let d = diff_capabilities(&old, &new);
        let added: Vec<&str> = d.added.iter().map(|c| c.description.as_str()).collect();
        assert_eq!(added, vec!["graph write:md.note.Note"], "only the new scope is added");
        assert!(
            d.removed.iter().any(|r| r.contains("old.example")),
            "the dropped host is removed: {:?}",
            d.removed
        );
        assert!(
            !d.removed.iter().any(|r| r.contains("read:system.File")),
            "a scope present on both sides is neither added nor removed: {:?}",
            d.removed
        );

        // An `extra` category dropped between versions is a removal; one still
        // present is not. Mutation testing found that `!` could be deleted, which
        // reports every surviving category as removed.
        let mut with_extra = caps();
        with_extra.extra.insert("input:global".into(), toml::Value::Boolean(true));
        with_extra.extra.insert("kept".into(), toml::Value::Boolean(true));
        let mut fewer = caps();
        fewer.extra.insert("kept".into(), toml::Value::Boolean(true));
        let d = diff_capabilities(&with_extra, &fewer);
        assert!(
            d.removed.iter().any(|r| r.contains("input:global")),
            "the dropped category is removed: {:?}",
            d.removed
        );
        assert!(
            !d.removed.iter().any(|r| r.contains("kept")),
            "a category on both sides is not removed: {:?}",
            d.removed
        );

        // The same scope listed twice is reported once.
        let mut twice = caps();
        twice.graph = vec!["write:md.note.Note".into(), "write:md.note.Note".into()];
        let d = diff_capabilities(&caps(), &twice);
        assert_eq!(d.added.len(), 1, "a repeated scope is not double-counted: {:?}", d.added);
    }

    /// An `extra` category present in the cap but with a DIFFERENT value still
    /// exceeds: an unrecognised value cannot be proven within an upper bound.
    /// Mutation testing found the `cap_value == value` guard could be replaced
    /// with `true`, which would accept any value the cap happened to mention.
    #[test]
    fn an_extra_category_with_a_different_value_still_exceeds() {
        let mut recipe = caps();
        recipe.extra.insert("quota_mb".into(), toml::Value::Integer(500));
        let mut cap = caps();
        cap.extra.insert("quota_mb".into(), toml::Value::Integer(10));
        assert_eq!(cap_exceeded(&recipe, &cap), vec!["extra.quota_mb".to_string()]);
    }

    /// `is_empty` is what "nothing changed" means to the update flow, so a diff
    /// with content must not report empty. Mutation testing found it could be
    /// replaced with `true`, and its `&&` with `||` - either of which makes a
    /// real change read as no change.
    #[test]
    fn a_diff_with_content_is_not_empty() {
        let mut added = caps();
        added.network = vec!["api.example.com".into()];
        let only_added = diff_capabilities(&caps(), &added);
        assert!(!only_added.added.is_empty());
        assert!(only_added.removed.is_empty());
        assert!(!only_added.is_empty(), "an addition alone is a change");

        let only_removed = diff_capabilities(&added, &caps());
        assert!(only_removed.added.is_empty());
        assert!(!only_removed.removed.is_empty());
        assert!(!only_removed.is_empty(), "a removal alone is a change");

        assert!(diff_capabilities(&added, &added).is_empty(), "no change is empty");
    }

    fn map(c: &Capabilities) -> CapabilityMapping {
        capabilities_to_profile("org.example.app", AppTier::ThirdParty, c)
    }

    #[test]
    fn known_filesystem_names_set_flags_unknown_become_custom() {
        let mut c = caps();
        c.filesystem = vec![
            "home".into(),
            "Documents".into(),
            "/opt/data".into(),
            "weird".into(),
        ];
        let m = map(&c);
        assert!(m.profile.filesystem.home);
        assert!(m.profile.filesystem.documents);
        assert!(!m.profile.filesystem.downloads);
        // Path and unrecognised name are preserved as literal custom paths, not
        // widened to a standard directory.
        assert_eq!(
            m.profile.filesystem.custom,
            vec![PathBuf::from("/opt/data"), PathBuf::from("weird")]
        );
    }

    #[test]
    fn network_strips_port_and_star_means_all() {
        let mut c = caps();
        c.network = vec!["api.example.com:443".into(), "cdn.example.org".into()];
        let m = map(&c);
        assert!(!m.profile.network.allow_all);
        assert_eq!(
            m.profile.network.allowed_domains,
            vec!["api.example.com".to_string(), "cdn.example.org".to_string()]
        );

        let mut c2 = caps();
        c2.network = vec!["*".into()];
        assert!(map(&c2).profile.network.allow_all);

        // A scope with no host grants nothing and is surfaced, not dropped.
        let mut c3 = caps();
        c3.network = vec![":443".into()];
        let m3 = map(&c3);
        assert!(m3.profile.network.allowed_domains.is_empty());
        assert!(m3.unmapped.contains(&"network::443".to_string()));
    }

    #[test]
    fn graph_scopes_split_into_read_and_write() {
        let mut c = caps();
        c.graph = vec![
            "read:File".into(),
            "write:Tag".into(),
            "bogus".into(),
            "read:".into(),
        ];
        let m = map(&c);
        assert_eq!(m.profile.graph.read, vec!["File".to_string()]);
        assert_eq!(m.profile.graph.write, vec!["Tag".to_string()]);
        // A malformed scope and an empty type are surfaced, not granted.
        assert!(m.unmapped.contains(&"graph:bogus".to_string()));
        assert!(m.unmapped.contains(&"graph:read:".to_string()));
    }

    #[test]
    fn flag_capabilities_map_through() {
        let mut c = caps();
        c.notifications = true;
        c.clipboard = true;
        let m = map(&c);
        assert!(m.profile.notifications.enabled);
        assert!(m.profile.clipboard.read);
        assert!(m.profile.clipboard.write);
        // Conservative: clipboard does not imply sensitive or history access.
        assert!(!m.profile.clipboard.read_sensitive);
        assert!(!m.profile.clipboard.history);
    }

    #[test]
    fn audio_and_extra_are_preserved_as_unmapped() {
        let mut c = caps();
        c.audio = true;
        c.extra.insert("bluetooth".into(), toml::Value::Boolean(true));
        let m = map(&c);
        assert!(m.unmapped.contains(&"audio".to_string()));
        assert!(m.unmapped.contains(&"bluetooth".to_string()));
    }

    #[test]
    fn identical_capabilities_diff_empty_and_need_no_consent() {
        let mut c = caps();
        c.network = vec!["api.example.com:443".into()];
        c.notifications = true;
        let d = diff_capabilities(&c, &c.clone());
        assert!(d.is_empty());
        assert!(!d.requires_consent());
    }

    #[test]
    fn a_new_network_host_requires_consent() {
        let mut old = caps();
        old.network = vec!["a.example.com:443".into()];
        let mut new = caps();
        new.network = vec!["a.example.com:443".into(), "b.evil.com:443".into()];
        let d = diff_capabilities(&old, &new);
        assert!(d.requires_consent(), "a new network host is high-impact");
        assert!(d
            .added
            .iter()
            .any(|c| c.description == "network host b.evil.com:443" && c.high_impact));
    }

    #[test]
    fn a_new_filesystem_scope_requires_consent() {
        let mut new = caps();
        new.filesystem = vec!["home".into()];
        let d = diff_capabilities(&caps(), &new);
        assert!(d.requires_consent());
        assert!(d.added.iter().any(|c| c.description == "filesystem home" && c.high_impact));
    }

    #[test]
    fn graph_write_is_high_impact_read_is_not() {
        let mut new = caps();
        new.graph = vec!["read:File".into(), "write:Tag".into()];
        let d = diff_capabilities(&caps(), &new);
        assert!(d.requires_consent(), "a new graph write blocks");
        let read = d.added.iter().find(|c| c.description == "graph read:File").unwrap();
        let write = d.added.iter().find(|c| c.description == "graph write:Tag").unwrap();
        assert!(!read.high_impact);
        assert!(write.high_impact);
    }

    #[test]
    fn low_impact_flag_additions_do_not_block() {
        let mut new = caps();
        new.notifications = true;
        new.clipboard = true;
        let d = diff_capabilities(&caps(), &new);
        assert!(!d.requires_consent(), "notifications/clipboard prompt on use, not block");
        assert_eq!(d.added.len(), 2);
    }

    #[test]
    fn a_new_unknown_extra_capability_requires_consent() {
        let mut new = caps();
        new.extra.insert("bluetooth".into(), toml::Value::Boolean(true));
        let d = diff_capabilities(&caps(), &new);
        // An unmodelled capability must not slip past the gate by being unknown.
        assert!(d.requires_consent());
        assert!(d
            .added
            .iter()
            .any(|c| c.description == "capability bluetooth" && c.high_impact));
    }

    #[test]
    fn removals_narrow_freely_without_consent() {
        let mut old = caps();
        old.network = vec!["a.example.com:443".into()];
        old.notifications = true;
        let new = caps();
        let d = diff_capabilities(&old, &new);
        assert!(!d.requires_consent(), "narrowing never needs consent");
        assert!(d.added.is_empty());
        assert!(d.removed.contains(&"network host a.example.com:443".to_string()));
        assert!(d.removed.contains(&"notifications".to_string()));
    }

    #[test]
    fn empty_capabilities_grant_nothing() {
        let m = map(&caps());
        assert!(m.unmapped.is_empty());
        assert!(!m.profile.network.allow_all);
        assert!(m.profile.network.allowed_domains.is_empty());
        assert!(m.profile.graph.read.is_empty());
        assert!(!m.profile.filesystem.home);
        assert!(!m.profile.notifications.enabled);
        assert!(!m.profile.clipboard.read);
        assert_eq!(m.profile.info.app_id, "org.example.app");
    }

    #[test]
    fn a_recipe_within_the_cap_does_not_exceed_it() {
        let cap = Capabilities {
            network: vec!["api.example.com:443".into(), "cdn.example.com:443".into()],
            filesystem: vec!["home".into()],
            notifications: true,
            ..Default::default()
        };
        let recipe = Capabilities {
            network: vec!["api.example.com:443".into()],
            filesystem: vec!["home".into()],
            ..Default::default()
        };
        assert!(cap_exceeded(&recipe, &cap).is_empty());
    }

    #[test]
    fn a_network_host_outside_the_cap_is_flagged() {
        let cap = Capabilities {
            network: vec!["api.example.com:443".into()],
            ..Default::default()
        };
        let recipe = Capabilities {
            network: vec!["api.example.com:443".into(), "evil.example.net:443".into()],
            ..Default::default()
        };
        let over = cap_exceeded(&recipe, &cap);
        assert_eq!(over, vec!["network evil.example.net:443".to_string()]);
    }

    #[test]
    fn a_boolean_capability_the_cap_withholds_is_flagged() {
        let cap = Capabilities::default();
        let recipe = Capabilities {
            clipboard: true,
            ..Default::default()
        };
        assert_eq!(cap_exceeded(&recipe, &cap), vec!["clipboard".to_string()]);
    }

    #[test]
    fn an_extra_category_absent_from_the_cap_fails_closed() {
        let mut recipe = Capabilities::default();
        recipe
            .extra
            .insert("usb".to_string(), toml::Value::Boolean(true));
        let over = cap_exceeded(&recipe, &Capabilities::default());
        assert_eq!(over, vec!["extra.usb".to_string()]);
        // The same category, same value, declared in the cap is within bounds.
        let mut cap = Capabilities::default();
        cap.extra.insert("usb".to_string(), toml::Value::Boolean(true));
        assert!(cap_exceeded(&recipe, &cap).is_empty());
    }
}
