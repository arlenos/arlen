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

    // Sort for deterministic output.
    scopes.sort_by(|a, b| a.entity_type.cmp(&b.entity_type));
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

/// The mtime of an app's on-disk profile, used to invalidate a cached token when
/// the profile changes on disk. Resolves the canonical profile path, then stats
/// it. Replaces the fork's `PermissionProfile::profile_mtime` associated fn.
pub fn profile_mtime(
    app_id: &str,
) -> Result<std::time::SystemTime, arlen_permissions::PermissionError> {
    let path = arlen_permissions::profile_path(app_id)?;
    let meta = std::fs::metadata(&path)?;
    Ok(meta.modified()?)
}

/// The app ids of every installed app, from the profile directory (each app has a
/// `<app_id>.toml` there). Used to refuse a namespace delegation that would collide
/// with an installed app's own namespace (the foreign-app-bridges MEDIUM). An
/// unreadable directory yields an EMPTY list, which fails toward permitting the
/// delegation - the delegation is already bounded to the user's own KG and cannot
/// reach `system.*`/`shared.*`, so a missing enumeration must not brick a bridge;
/// the check is a hardening layer over that existing boundary, not the boundary.
pub fn installed_app_ids() -> Vec<String> {
    // Any valid app id resolves the profile directory; the id itself is only used
    // to locate the parent dir, then discarded.
    let Some(dir) = arlen_permissions::profile_path("probe")
        .ok()
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
    else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            e.file_name()
                .to_str()
                .and_then(|n| n.strip_suffix(".toml"))
                .map(str::to_string)
        })
        .collect()
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
