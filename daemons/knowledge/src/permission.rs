/// Permission Profile parser for Knowledge Graph access.
///
/// Reads `~/.config/permissions/{app_id}.toml` and converts the `[graph]`
/// section into token scopes. No `[graph]` section means no graph access.
///
/// See `docs/architecture/CAPABILITY-TOKENS.md` Section 6.

use std::collections::HashMap;

use crate::token::{EntityScope, InstanceScope, RelationScope};

// ---------------------------------------------------------------------------
// Scope entry parsing
// ---------------------------------------------------------------------------

/// Parse a list of scope strings into EntityScope structs.
///
/// An entry with 3 segments (`"system.File.path"`) is a field-level grant.
/// An entry with 2 segments (`"system.Session"`) grants all fields.
/// An entry ending in `.*` (`"com.anki.*"`) is a wildcard type grant.
fn parse_scope_entries(entries: &[String]) -> Vec<EntityScope> {
    // Group field-level entries by entity type.
    let mut type_fields: HashMap<String, Vec<String>> = HashMap::new();
    let mut full_types: Vec<String> = Vec::new();

    for entry in entries {
        if entry.ends_with(".*") {
            // Wildcard: "com.anki.*" -> full type grant
            full_types.push(entry.clone());
        } else {
            // Count dot-separated segments.
            let parts: Vec<&str> = entry.splitn(3, '.').collect();
            match parts.len() {
                3 => {
                    // "system.File.path" -> type = "system.File", field = "path"
                    let entity_type = format!("{}.{}", parts[0], parts[1]);
                    type_fields
                        .entry(entity_type)
                        .or_default()
                        .push(parts[2].to_string());
                }
                2 => {
                    // "system.Session" -> full entity grant
                    full_types.push(entry.clone());
                }
                _ => {
                    // Invalid entry, skip.
                }
            }
        }
    }

    let mut scopes = Vec::new();

    // Full-type entries (fields: None).
    for entity_type in full_types {
        // If we also have field-level entries for this type, the full grant wins.
        let base = if entity_type.ends_with(".*") {
            entity_type.clone()
        } else {
            entity_type.clone()
        };
        type_fields.remove(&base);
        scopes.push(EntityScope {
            entity_type,
            fields: None,
            exclude_fields: vec![],
        });
    }

    // Field-level entries.
    for (entity_type, fields) in type_fields {
        scopes.push(EntityScope {
            entity_type,
            fields: Some(fields),
            exclude_fields: vec![],
        });
    }

    // Sort for deterministic output - the ENTITY TYPES and the field lists
    // inside each scope.
    //
    // Sorting the types alone was not enough, and the gap was invisible from
    // here: `read = ["system.File.path", "system.File.app_id"]` and the same two
    // lines swapped produced `["path","app_id"]` and `["app_id","path"]`. Same
    // authority, two different scope lists - and `lcg::declared_ceiling_json`
    // serialises this, `retention::collapse_grant_history` collapses runs of
    // grants whose ceiling STRING matches. So a profile rewritten with its
    // entries in another order (which `revoke` does, format-preservingly, every
    // time it narrows one) started a fresh run, and the Grant projection kept
    // both ends of it for an authority that never changed.
    //
    // Field order carries no meaning - `can_read` asks whether a name is in the
    // list - so canonicalising it costs nothing and makes the ceiling a function
    // of the grant rather than of its typography.
    scopes.sort_by(|a, b| a.entity_type.cmp(&b.entity_type));
    for scope in &mut scopes {
        if let Some(fields) = scope.fields.as_mut() {
            fields.sort();
        }
        scope.exclude_fields.sort();
    }
    scopes
}

// ---------------------------------------------------------------------------
// Canonical-profile graph scope extension (E0: one profile type, parsed once)
// ---------------------------------------------------------------------------

/// Projects the canonical [`arlen_permissions::PermissionProfile`]'s `[graph]`
/// grants into the graph-layer token scope types ([`EntityScope`] /
/// [`RelationScope`] / [`InstanceScope`]). These methods live in the knowledge
/// daemon, not `sdk/permissions`, because the scope types are graph-layer
/// (`crate::token`) concepts. This replaces the knowledge-local `PermissionProfile`
/// fork so the same `{app_id}.toml` is parsed once (canonical) per connect, not
/// twice into two divergent types.
pub trait GraphScopeExt {
    /// Whether the profile grants any Knowledge Graph reach at all. Unlike the
    /// old fork's "a `[graph]` section is present" test (the canonical type's
    /// graph is always present, defaulted), this is "any graph grant is
    /// non-empty" - stricter least-privilege: an empty `[graph]` no longer mints
    /// a useless zero-scope token.
    fn has_graph_access(&self) -> bool;
    /// The read grants as token entity scopes.
    fn to_read_scopes(&self) -> Vec<EntityScope>;
    /// The write grants as token entity scopes.
    fn to_write_scopes(&self) -> Vec<EntityScope>;
    /// The relation grants as token relation scopes.
    fn to_relation_scopes(&self) -> Vec<RelationScope>;
    /// The declared instance scope (own/all) as the token instance scope.
    fn to_instance_scope(&self) -> InstanceScope;
    /// The delegated namespaces this profile grants (raw prefix strings).
    fn delegated_namespaces(&self) -> Vec<String>;
}

impl GraphScopeExt for arlen_permissions::PermissionProfile {
    fn has_graph_access(&self) -> bool {
        let g = &self.graph;
        !g.read.is_empty()
            || !g.write.is_empty()
            || !g.relations.is_empty()
            || !g.read_sensitive.is_empty()
            || !g.delegated_namespaces.is_empty()
    }

    fn to_read_scopes(&self) -> Vec<EntityScope> {
        parse_scope_entries(&self.graph.read)
    }

    fn to_write_scopes(&self) -> Vec<EntityScope> {
        parse_scope_entries(&self.graph.write)
    }

    fn to_relation_scopes(&self) -> Vec<RelationScope> {
        self.graph
            .relations
            .iter()
            .map(|r| RelationScope {
                from: r.from.clone(),
                to: r.to.clone(),
                relation_type: r.relation_type.clone(),
            })
            .collect()
    }

    fn to_instance_scope(&self) -> InstanceScope {
        match self.graph.instance_scope {
            arlen_permissions::InstanceScopeConfig::Own => InstanceScope::Own,
            arlen_permissions::InstanceScopeConfig::All => InstanceScope::All,
        }
    }

    fn delegated_namespaces(&self) -> Vec<String> {
        self.graph.delegated_namespaces.clone()
    }
}

/// The newest mtime across BOTH tiers of an app's on-disk profile, used to
/// invalidate a cached token when what governs the app changes.
///
/// Both, not just the user tier, because the system tier is the one that wins
/// (`load_tiered`): stating only the user file meant a system-tier profile could
/// be narrowed and the daemon would keep serving the cached token until restart -
/// the revoke would look applied and take no effect. A system profile APPEARING
/// also changes the effective grants without touching the user file, and taking
/// the newest of the two catches that too.
///
/// Deliberately conservative: any change to either file invalidates, including one
/// to a user file the system tier currently shadows. Re-minting a token that did
/// not need re-minting costs a profile read; missing one serves authority the user
/// believes they revoked.
///
/// `Err` only when neither tier has a readable profile, which the caller already
/// treats as "no mtime to compare".
pub fn profile_mtime(
    app_id: &str,
) -> Result<std::time::SystemTime, arlen_permissions::PermissionError> {
    newest_mtime(arlen_permissions::profile_paths(app_id)).ok_or(
        arlen_permissions::PermissionError::NotFound {
            app_id: app_id.to_string(),
        },
    )
}

/// The newest mtime among the paths that exist, or `None` when none do. Split out
/// so the both-tiers rule is tested against real files rather than by mutating the
/// process environment, which is shared and would race the other tests.
fn newest_mtime(
    paths: impl IntoIterator<Item = std::path::PathBuf>,
) -> Option<std::time::SystemTime> {
    paths
        .into_iter()
        .filter_map(|p| std::fs::metadata(&p).ok()?.modified().ok())
        .max()
}

/// The app ids of every installed app, from BOTH profile tiers (each app has a
/// `<app_id>.toml` in one of them). Used to refuse a namespace delegation that
/// would collide with an installed app's own namespace (the foreign-app-bridges
/// MEDIUM).
///
/// Both tiers because an apt-installed app has a profile only at the system tier,
/// and it is installed in exactly the sense this check means - enumerating the
/// user directory alone left every such app's namespace claimable by a bridge.
/// Sorted and deduped so an app present in both tiers is named once.
///
/// An unreadable directory contributes nothing rather than failing, which fails
/// toward permitting the delegation - the delegation is already bounded to the
/// user's own KG and cannot reach `system.*`/`shared.*`, so a missing enumeration
/// must not brick a bridge; the check is a hardening layer over that existing
/// boundary, not the boundary.
/// The ids found across `dirs`, sorted and deduped. Split out so the both-tiers
/// rule is tested against real directories rather than by pointing the process
/// environment at temp paths, which is shared state and would race.
fn ids_in(dirs: [Option<std::path::PathBuf>; 2]) -> Vec<String> {
    let mut ids: Vec<String> = dirs
        .into_iter()
        .flatten()
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flat_map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter_map(|e| {
                    e.file_name()
                        .to_str()
                        .and_then(|n| n.strip_suffix(".toml"))
                        .map(str::to_string)
                })
                .collect::<Vec<_>>()
        })
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

pub fn installed_app_ids() -> Vec<String> {
    // Any valid app id resolves a profile path; the id itself is only used to
    // locate the parent dirs, then discarded.
    let user = arlen_permissions::profile_path("probe")
        .ok()
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf));
    ids_in([Some(arlen_permissions::system_permissions_dir()), user])
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse a canonical profile from a `[graph]`-only test body. The canonical
    /// type requires an `[info]` section, so prepend a minimal one; the scope
    /// projections under test come from [`GraphScopeExt`].
    fn graph_profile(content: &str) -> arlen_permissions::PermissionProfile {
        toml::from_str(&format!("[info]\napp_id = \"com.test\"\n{content}")).unwrap()
    }

    #[test]
    fn an_app_installed_at_either_tier_is_enumerated_once() {
        // The collision check means "installed", and an apt app is installed with
        // a profile only at the system tier - reading the user directory alone
        // left its namespace claimable by a bridge.
        let dir = tempfile::tempdir().unwrap();
        let system = dir.path().join("system");
        let user = dir.path().join("user");
        std::fs::create_dir_all(&system).unwrap();
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(system.join("com.apt.Only.toml"), "").unwrap();
        std::fs::write(user.join("com.user.Only.toml"), "").unwrap();
        // Present in both: named once, not twice.
        std::fs::write(system.join("com.both.App.toml"), "").unwrap();
        std::fs::write(user.join("com.both.App.toml"), "").unwrap();
        // A non-profile file is not an app.
        std::fs::write(user.join("notes.txt"), "").unwrap();

        let ids = ids_in([Some(system), Some(user)]);
        assert_eq!(
            ids,
            vec![
                "com.apt.Only".to_string(),
                "com.both.App".to_string(),
                "com.user.Only".to_string(),
            ]
        );
    }

    #[test]
    fn an_unreadable_directory_contributes_nothing_rather_than_failing() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("com.example.App.toml"), "").unwrap();
        let ids = ids_in([Some(dir.path().join("absent")), Some(real)]);
        assert_eq!(ids, vec!["com.example.App".to_string()]);
    }

    #[test]
    fn the_newest_of_the_two_tiers_is_what_invalidates() {
        // A system-tier profile is the one that governs, so a change to it has to
        // invalidate the cached token. Taking the newest of both also catches a
        // system profile APPEARING, which changes the effective grants without
        // touching the user file at all.
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("user.toml");
        let system = dir.path().join("system.toml");
        std::fs::write(&user, "a").unwrap();
        let user_at = std::fs::metadata(&user).unwrap().modified().unwrap();

        // Only the user file exists: that is the answer.
        assert_eq!(newest_mtime([system.clone(), user.clone()]), Some(user_at));

        // The system file appears, later: it wins.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&system, "b").unwrap();
        let system_at = std::fs::metadata(&system).unwrap().modified().unwrap();
        assert_eq!(newest_mtime([system.clone(), user.clone()]), Some(system_at));
        assert!(system_at > user_at, "the fixture must actually differ");

        // Neither exists: nothing to compare, rather than a bogus epoch.
        assert_eq!(newest_mtime([dir.path().join("absent.toml")]), None);
    }

    #[test]
    fn test_to_read_scopes_field_parsing() {
        let profile = graph_profile(
            r#"
[graph]
read = ["system.File.path"]
"#,
        );
        let scopes = profile.to_read_scopes();
        assert_eq!(scopes.len(), 1);
        assert_eq!(scopes[0].entity_type, "system.File");
        assert_eq!(scopes[0].fields, Some(vec!["path".to_string()]));
    }

    #[test]
    fn test_to_read_scopes_full_entity() {
        let profile = graph_profile(
            r#"
[graph]
read = ["system.Session"]
"#,
        );
        let scopes = profile.to_read_scopes();
        assert_eq!(scopes.len(), 1);
        assert_eq!(scopes[0].entity_type, "system.Session");
        assert!(scopes[0].fields.is_none());
    }

    #[test]
    fn test_to_read_scopes_wildcard() {
        let profile = graph_profile(
            r#"
[graph]
read = ["com.anki.*"]
"#,
        );
        let scopes = profile.to_read_scopes();
        assert_eq!(scopes.len(), 1);
        assert_eq!(scopes[0].entity_type, "com.anki.*");
        assert!(scopes[0].fields.is_none());
    }

    #[test]
    fn test_to_read_scopes_merge_fields() {
        let profile = graph_profile(
            r#"
[graph]
read = ["system.File.path", "system.File.name", "system.File.modified_at"]
"#,
        );
        let scopes = profile.to_read_scopes();
        assert_eq!(scopes.len(), 1);
        assert_eq!(scopes[0].entity_type, "system.File");
        let fields = scopes[0].fields.as_ref().unwrap();
        assert_eq!(fields.len(), 3);
        assert!(fields.contains(&"path".to_string()));
        assert!(fields.contains(&"name".to_string()));
        assert!(fields.contains(&"modified_at".to_string()));
    }

    #[test]
    fn test_to_read_scopes_full_overrides_fields() {
        // If both "system.File" and "system.File.path" are listed,
        // the full grant wins (fields: None).
        let profile = graph_profile(
            r#"
[graph]
read = ["system.File.path", "system.File"]
"#,
        );
        let scopes = profile.to_read_scopes();
        assert_eq!(scopes.len(), 1);
        assert_eq!(scopes[0].entity_type, "system.File");
        assert!(scopes[0].fields.is_none(), "full entity grant should override field-level");
    }

    #[test]
    fn test_to_write_scopes() {
        let profile = graph_profile(
            r#"
[graph]
write = ["com.anki.*"]
"#,
        );
        let scopes = profile.to_write_scopes();
        assert_eq!(scopes.len(), 1);
        assert_eq!(scopes[0].entity_type, "com.anki.*");
    }

    #[test]
    fn test_to_relation_scopes() {
        let profile = graph_profile(
            r#"
[graph]
relations = [
    { from = "com.anki.Card", to = "system.File", type = "REFERENCES" },
    { from = "com.anki.Card", to = "shared.Person", type = "MENTIONS" },
]
"#,
        );
        let scopes = profile.to_relation_scopes();
        assert_eq!(scopes.len(), 2);
        assert_eq!(scopes[0].from, "com.anki.Card");
        assert_eq!(scopes[0].to, "system.File");
        assert_eq!(scopes[0].relation_type, "REFERENCES");
        assert_eq!(scopes[1].relation_type, "MENTIONS");
    }

    #[test]
    fn test_instance_scope_own() {
        let profile = graph_profile(
            r#"
[graph]
instance_scope = "own"
"#,
        );
        assert_eq!(profile.to_instance_scope(), InstanceScope::Own);
    }

    #[test]
    fn test_instance_scope_all() {
        let profile = graph_profile(
            r#"
[graph]
instance_scope = "all"
"#,
        );
        assert_eq!(profile.to_instance_scope(), InstanceScope::All);
    }

    #[test]
    fn test_instance_scope_default() {
        let profile = graph_profile("[graph]\n");
        assert_eq!(profile.to_instance_scope(), InstanceScope::Own);
    }

}

#[cfg(test)]
mod ceiling_determinism {
    use super::*;

    /// The Grant projection's retention rests on this.
    ///
    /// `lcg::declared_ceiling_json` serialises these scope collections, and
    /// `retention::collapse_grant_history` collapses runs of grants whose ceiling
    /// STRING is identical. If the same profile could serialise two ways, every
    /// run would be length one, nothing would ever collapse, and the projection
    /// would grow forever while looking maintained.
    ///
    /// `parse_scope_entries` groups field entries in a `HashMap`, whose iteration
    /// order is randomised per process - so the `sort_by` at the end of that
    /// function is load-bearing for retention, several files away from anything
    /// that says so. This is what says so.
    /// The scopes AS SERIALISED, because the serialised form is what the ceiling
    /// actually is - comparing the structs would be comparing a proxy for the
    /// thing the retention rule keys on.
    fn ceiling_of(entries: &[&str]) -> String {
        let list = entries.iter().map(|e| format!("\"{e}\"")).collect::<Vec<_>>().join(", ");
        let p = toml::from_str::<arlen_permissions::PermissionProfile>(&format!(
            "[info]\napp_id = \"com.test\"\n[graph]\nread = [{list}]\n"
        ))
        .expect("a profile with a read list parses");
        serde_json::to_string(&p.to_read_scopes()).expect("scopes serialise")
    }

    /// The same grants written in a different ORDER must produce the same scopes.
    /// A profile rewritten by a tool that sorts its keys differently is the same
    /// authority and must not read as a new ceiling.
    #[test]
    fn input_order_does_not_change_the_scopes() {
        let a = ceiling_of(&[
            "system.File.path",
            "system.Project",
            "system.File.app_id",
            "system.Session",
        ]);
        let b = ceiling_of(&[
            "system.Session",
            "system.File.app_id",
            "system.Project",
            "system.File.path",
        ]);
        assert_eq!(a, b, "the ceiling must not depend on the order entries were written in");
    }

    /// And repeated parses inside one process agree - the case that would break
    /// if the `HashMap` reached the output without the sort.
    #[test]
    fn repeated_parses_agree() {
        let entries = ["system.File.path", "system.App.name", "system.Project.root_path"];
        let first = ceiling_of(&entries);
        for _ in 0..8 {
            assert_eq!(ceiling_of(&entries), first);
        }
    }

    /// The residual named in the 12 Aug report: a full-type grant and a
    /// field-level one for the SAME type. `parse_scope_entries` removes the
    /// field-level entry when a full grant covers it, so only one scope carries
    /// that type - which is what keeps `sort_by(entity_type)` a total order
    /// rather than one with ties resolved by whatever the HashMap yielded.
    #[test]
    fn a_full_grant_and_a_field_grant_for_one_type_collapse_to_one_scope() {
        let s = ceiling_of(&["system.File.path", "system.File", "system.Project"]);
        assert_eq!(
            s.matches("\"system.File\"").count(),
            1,
            "one scope per entity type, or sort_by(entity_type) has ties to break: {s}"
        );
    }
}
