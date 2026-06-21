//! App-tier entity-instance persistence: the storage schema for a registered
//! custom entity type (foreign-app-bridges piece 1).
//!
//! An app declares its entity types in `entities.toml` (registered into the
//! [`SchemaRegistry`]); this module turns a registered [`EntityDefinition`]
//! into the Ladybug node table that holds its instances, so the long-standing
//! "can declare a type but cannot write an instance" gap closes. The actual
//! authorise/validate of an instance write is `create_entity` (already built);
//! the upsert + persistence build on the table this module defines.
//!
//! Naming: a qualified type (`md.obsidian.Note`, and app ids carry dots and
//! hyphens) is not a legal Ladybug identifier, so the table name is a
//! sanitised, **hash-suffixed** encoding — injective even when two distinct
//! qualified types sanitise to the same prefix, so one app's type can never
//! land in another's table. The canonical qualified type is also stored in the
//! reserved `_type` column. Custom-type labels are not in any caller's readable
//! scope, so the read gate (RS-R1) fails closed on them until the custom-type
//! read path is designed; persisting them now is therefore write-only and safe.

use std::collections::{BTreeMap, HashMap};

use chrono::Utc;

use crate::schema::{EntityDefinition, FieldType, SchemaRegistry};
use crate::token::CapabilityToken;
use crate::utils::escape_cypher;
use crate::write::validation::FieldValidator;

/// A reserved column every entity table carries. Mirrors the reserved fields
/// `create_entity` injects, plus `_type` (the canonical qualified type, since
/// the table name is an encoded form) and `_external_key` (the bridge's stable
/// idempotency key, set by the upsert path).
const RESERVED_COLUMNS: &[(&str, &str)] = &[
    ("id", "STRING"),
    ("_type", "STRING"),
    ("_external_key", "STRING"),
    ("_owner", "STRING"),
    ("_version", "INT64"),
    ("_created_at", "STRING"),
    ("_modified_at", "STRING"),
    ("_deleted", "BOOL"),
];

/// Errors building an entity table's DDL.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EntityTableError {
    /// A field name is not a safe SQL identifier (it would corrupt the DDL).
    #[error("field name is not a safe identifier: {0}")]
    UnsafeFieldName(String),
    /// A field name collides with a reserved column.
    #[error("field name collides with a reserved column: {0}")]
    ReservedFieldName(String),
}

/// The Ladybug node-table name for a qualified entity type.
///
/// `e_<sanitised>_<hash>`: every non-alphanumeric character of the qualified
/// type becomes `_` (so the result is a legal identifier), and a stable hash of
/// the *canonical* qualified type is appended so two types that sanitise to the
/// same prefix still get distinct tables (injective). Deterministic — the same
/// qualified type always maps to the same table.
pub fn entity_table_name(qualified_type: &str) -> String {
    let sanitised: String = qualified_type
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("e_{sanitised}_{:016x}", fnv1a64(qualified_type))
}

/// A stable 64-bit FNV-1a hash. Deterministic across runs and machines (no
/// random seed), so the table name for a qualified type is stable.
fn fnv1a64(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Map a schema field type to its Ladybug column type. Text-like and reference
/// types are stored as `STRING`; integral/duration as `INT64`; `Float` as
/// `DOUBLE`; `Bool` as `BOOL`. List and `Bytes` fields are serialised to a
/// `STRING` (JSON / encoded) by the write path — Ladybug list columns are a
/// later refinement, so v1 keeps the column scalar.
fn column_type(field_type: &FieldType) -> &'static str {
    match field_type {
        FieldType::Int | FieldType::Duration => "INT64",
        FieldType::Float => "DOUBLE",
        FieldType::Bool => "BOOL",
        // String/Text/Datetime/Date/Url/Email/Path/Json/Markdown/Color/Uuid,
        // a reference id, and (v1) lists + bytes all store as STRING.
        _ => "STRING",
    }
}

/// Whether `name` is a safe, unquoted Ladybug identifier (a column name we can
/// interpolate into DDL): ASCII alphanumeric or `_`, not starting with a digit,
/// non-empty, bounded. Reserved columns start with `_`; a user field may too,
/// but it must not *collide* with a reserved column (checked separately).
fn is_safe_identifier(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Build the idempotent `CREATE NODE TABLE IF NOT EXISTS` DDL for a registered
/// entity type: the reserved columns plus one column per declared field, keyed
/// on `id`. `qualified_type` is the canonical `{namespace}.{Type}`.
///
/// Fails closed if a field name is not a safe identifier or collides with a
/// reserved column, rather than emit DDL that could corrupt the table or shadow
/// a reserved column.
pub fn entity_table_ddl(
    qualified_type: &str,
    def: &EntityDefinition,
) -> Result<String, EntityTableError> {
    let table = entity_table_name(qualified_type);
    let reserved: std::collections::BTreeSet<&str> =
        RESERVED_COLUMNS.iter().map(|(n, _)| *n).collect();

    let mut columns: Vec<String> = RESERVED_COLUMNS
        .iter()
        .map(|(name, ty)| format!("{name} {ty}"))
        .collect();

    // Field columns in a deterministic order (BTreeMap-sorted), so the DDL is
    // stable across runs.
    let mut field_names: Vec<&String> = def.fields.keys().collect();
    field_names.sort();
    for name in field_names {
        if !is_safe_identifier(name) {
            return Err(EntityTableError::UnsafeFieldName(name.clone()));
        }
        if reserved.contains(name.as_str()) {
            return Err(EntityTableError::ReservedFieldName(name.clone()));
        }
        let ty = column_type(&def.fields[name].field_type);
        columns.push(format!("{name} {ty}"));
    }

    Ok(format!(
        "CREATE NODE TABLE IF NOT EXISTS {table}({}, PRIMARY KEY(id))",
        columns.join(", ")
    ))
}

/// The deterministic node id for a bridge upsert: `{qualified_type}:{external_key}`.
///
/// Stable (the same external key always maps to the same node, so a re-sync
/// MERGEs in place rather than duplicating) and globally unique per (type, key)
/// with no hashing, so there is no collision to reason about. Stored as the
/// table's `id` primary key.
pub fn entity_node_id(qualified_type: &str, external_key: &str) -> String {
    format!("{qualified_type}:{external_key}")
}

/// Render a schema-validated field value as a Cypher literal. String-like values
/// are single-quoted and [`escape_cypher`]-escaped so a value can never break out
/// of the literal (the injection-safety guarantee); numbers and bools render
/// natively; an array/object (stored in a `STRING` column, see [`column_type`])
/// is JSON-serialised then escaped; null renders as `null`.
fn field_literal(value: &serde_json::Value) -> String {
    use serde_json::Value;
    match value {
        Value::String(s) => format!("'{}'", escape_cypher(s)),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Null => "null".to_string(),
        other => {
            let json = serde_json::to_string(other).unwrap_or_default();
            format!("'{}'", escape_cypher(&json))
        }
    }
}

/// Build the idempotent upsert statement for a validated entity write.
///
/// MERGE on the deterministic id (so a re-sync of the same external key updates
/// the existing node in place, never duplicates), setting the reserved
/// provenance columns + `_version=1` on first insert and bumping `_version` +
/// `_modified_at` on every later write. Every string value (`owner`,
/// `external_key`, the canonical type, and each field) is escaped into its
/// literal; the table name and field names are safe identifiers validated at
/// table creation, so they are interpolated directly.
fn build_upsert_cypher(
    qualified_type: &str,
    external_key: &str,
    owner: &str,
    fields: &BTreeMap<String, serde_json::Value>,
    now: &str,
) -> String {
    let table = entity_table_name(qualified_type);
    let id = escape_cypher(&entity_node_id(qualified_type, external_key));
    let qt = escape_cypher(qualified_type);
    let ek = escape_cypher(external_key);
    let owner_lit = escape_cypher(owner);
    let now_lit = escape_cypher(now);

    // Field assignments, deterministic order, applied identically on create and
    // on match (the upsert overwrites the field set each sync).
    let field_sets: String = fields
        .iter()
        .map(|(name, v)| format!(", n.{name}={}", field_literal(v)))
        .collect();

    format!(
        "MERGE (n:{table} {{id: '{id}'}}) \
         ON CREATE SET n._type='{qt}', n._external_key='{ek}', n._owner='{owner_lit}', \
         n._created_at='{now_lit}', n._modified_at='{now_lit}', n._version=1, n._deleted=false{field_sets} \
         ON MATCH SET n._modified_at='{now_lit}', n._version=n._version+1{field_sets} \
         RETURN n._version AS version"
    )
}

/// Authorise, validate, and build the persistence plan for an app-tier entity
/// upsert (foreign-app-bridges piece 1): the general "an app writes an instance
/// of its own declared entity type, idempotently" path that the not-wired
/// declare-but-cannot-write gap needs.
///
/// Returns `(table_ddl, upsert_cypher)`: the caller ensures the dynamic table
/// exists with the DDL (idempotent `CREATE NODE TABLE IF NOT EXISTS`), then runs
/// the upsert. Fail-closed at every step:
/// - a non-empty `external_key` is required (the idempotency key);
/// - `system.*` / `shared.*` are structurally unwritable by a third party;
/// - the caller's token must grant write to the type, AND the type must be in
///   the caller's own namespace (peer-attested `app_id`), so an app can only
///   write its own data;
/// - the type must be registered and the fields must validate against its
///   schema (unknown field, wrong type, or missing-required all reject).
///
/// The scoping is the caller's peer-attested `app_id` + the namespace bound (the
/// guarantee `foreign-app-bridges.md` requires: namespace-bounded, audited).
/// A macaroon-format token (attenuate-only caveats) is the planned upgrade; the
/// guarantee is what matters and is delivered here.
pub fn plan_entity_upsert(
    registry: &SchemaRegistry,
    token: &CapabilityToken,
    qualified_type: &str,
    external_key: &str,
    fields: HashMap<String, serde_json::Value>,
) -> Result<(String, String), String> {
    if external_key.trim().is_empty() {
        return Err("upsert requires a non-empty external_key".into());
    }
    if qualified_type.starts_with("system.") || qualified_type.starts_with("shared.") {
        return Err(format!(
            "namespace not writable by a third party: {qualified_type}"
        ));
    }
    if !token.can_write(qualified_type) {
        return Err(format!("permission denied for {qualified_type}"));
    }
    // The caller may only write its own namespace (the type prefix must be the
    // attested app_id). This is the cross-tenant boundary.
    let prefix = format!("{}.", token.app_id);
    if !qualified_type.starts_with(&prefix) {
        return Err(format!(
            "namespace violation: {} cannot write {qualified_type}",
            token.app_id
        ));
    }
    let def = registry
        .get_entity(qualified_type)
        .ok_or_else(|| format!("entity type not registered: {qualified_type}"))?;
    FieldValidator::new(registry)
        .validate_create(qualified_type, &fields)
        .map_err(|e| format!("validation: {e}"))?;

    let ddl = entity_table_ddl(qualified_type, def).map_err(|e| format!("schema: {e}"))?;
    let now = Utc::now().to_rfc3339();
    let ordered: BTreeMap<String, serde_json::Value> = fields.into_iter().collect();
    let cypher = build_upsert_cypher(qualified_type, external_key, &token.app_id, &ordered, &now);
    Ok((ddl, cypher))
}

/// The deterministic Kuzu REL TABLE name for a bridge edge. A REL TABLE binds one
/// (FROM, TO) node-table pair, so the name is keyed by the edge type AND both
/// endpoint types: the same edge label between two different type pairings gets
/// its own table rather than colliding. The sanitised edge label is kept in the
/// name for recognisability; the hash makes it unique + collision-free.
pub fn entity_rel_table_name(edge_type: &str, from_type: &str, to_type: &str) -> String {
    let sanitised: String = edge_type
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let key = format!("{edge_type}\u{0}{from_type}\u{0}{to_type}");
    format!("r_{sanitised}_{:016x}", fnv1a64(&key))
}

/// Build the idempotent edge-MERGE for a bridge link. Both endpoint nodes must
/// already exist (the `MATCH` binds them by their deterministic ids); if either
/// is absent - a forward reference to a not-yet-synced node - the `MERGE` does
/// not run and `linked` is 0, which a re-sync resolves once both exist. The
/// `MERGE` is idempotent, so a re-sync never duplicates the edge. Ids are
/// `escape_cypher`-escaped; the table + rel names are validated identifiers.
fn build_link_cypher(
    rel_table: &str,
    from_table: &str,
    to_table: &str,
    from_id: &str,
    to_id: &str,
) -> String {
    let from = escape_cypher(from_id);
    let to = escape_cypher(to_id);
    format!(
        "MATCH (a:{from_table} {{id: '{from}'}}), (b:{to_table} {{id: '{to}'}}) \
         MERGE (a)-[:{rel_table}]->(b) \
         RETURN count(*) AS linked"
    )
}

/// Authorise, validate, and build the persistence plan for an app-tier entity
/// LINK (foreign-app-bridges piece 2): an app creates an edge between two
/// instances of its OWN declared entity types, idempotently.
///
/// Returns `(rel_table_ddl, merge_cypher)`: the caller ensures the dynamic REL
/// TABLE exists with the DDL (idempotent `CREATE REL TABLE IF NOT EXISTS`), then
/// runs the MERGE. Fail-closed, mirroring [`plan_entity_upsert`] for BOTH
/// endpoints so a bridge can never forge a cross-tenant or system edge:
/// - both external keys are non-empty;
/// - the edge type is a safe identifier (it becomes the rel-table label);
/// - `system.*` / `shared.*` are unwritable as either endpoint by a third party;
/// - both endpoint types must be in the caller's own namespace (the attested
///   `app_id` prefix) AND token-writable, so an app can only link its own nodes
///   to each other (no edge to another tenant's, a shared, or a system node);
/// - both endpoint types must be registered.
///
/// A cross-namespace edge (e.g. to a `shared.Person`) is deliberately NOT
/// permitted here: it is a separate, carefully-scoped feature; the anti-poisoning
/// guarantee is that a bridge's edges stay inside its own namespace.
#[allow(clippy::too_many_arguments)]
pub fn plan_entity_link(
    registry: &SchemaRegistry,
    token: &CapabilityToken,
    edge_type: &str,
    from_type: &str,
    from_key: &str,
    to_type: &str,
    to_key: &str,
) -> Result<(String, String), String> {
    if from_key.trim().is_empty() || to_key.trim().is_empty() {
        return Err("link requires non-empty endpoint keys".into());
    }
    if !is_safe_identifier(edge_type) {
        return Err(format!("invalid edge type: {edge_type:?}"));
    }
    // Both endpoints pass the same namespace + registration gate the upsert uses.
    for ty in [from_type, to_type] {
        if ty.starts_with("system.") || ty.starts_with("shared.") {
            return Err(format!("namespace not linkable by a third party: {ty}"));
        }
        if !token.can_write(ty) {
            return Err(format!("permission denied for {ty}"));
        }
        let prefix = format!("{}.", token.app_id);
        if !ty.starts_with(&prefix) {
            return Err(format!(
                "namespace violation: {} cannot link {ty}",
                token.app_id
            ));
        }
        if registry.get_entity(ty).is_none() {
            return Err(format!("entity type not registered: {ty}"));
        }
    }

    let from_table = entity_table_name(from_type);
    let to_table = entity_table_name(to_type);
    let rel_table = entity_rel_table_name(edge_type, from_type, to_type);
    let ddl = format!(
        "CREATE REL TABLE IF NOT EXISTS {rel_table}(FROM {from_table} TO {to_table})"
    );
    let from_id = entity_node_id(from_type, from_key);
    let to_id = entity_node_id(to_type, to_key);
    let cypher = build_link_cypher(&rel_table, &from_table, &to_table, &from_id, &to_id);
    Ok((ddl, cypher))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::FieldDefinition;
    use std::collections::HashMap;

    fn def(fields: &[(&str, FieldType)]) -> EntityDefinition {
        let mut map = HashMap::new();
        for (name, ty) in fields {
            map.insert(
                (*name).to_string(),
                FieldDefinition {
                    field_type: ty.clone(),
                    ..Default::default()
                },
            );
        }
        EntityDefinition {
            fields: map,
            ..Default::default()
        }
    }

    #[test]
    fn table_name_is_a_legal_identifier() {
        let name = entity_table_name("md.obsidian.Note");
        assert!(name.starts_with("e_md_obsidian_Note_"));
        assert!(name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_'));
    }

    #[test]
    fn table_name_is_injective_across_sanitisation_collisions() {
        // Two distinct qualified types that sanitise to the same prefix (a
        // hyphen vs a dot) must still get distinct tables — otherwise one app's
        // type could land in another's table.
        let a = entity_table_name("com.a-b.Note");
        let b = entity_table_name("com.a.b.Note");
        assert_ne!(a, b, "the hash suffix must disambiguate: {a} vs {b}");
    }

    #[test]
    fn table_name_is_deterministic() {
        assert_eq!(
            entity_table_name("md.obsidian.Note"),
            entity_table_name("md.obsidian.Note")
        );
    }

    #[test]
    fn ddl_has_reserved_and_field_columns_with_mapped_types() {
        let d = def(&[
            ("title", FieldType::String),
            ("count", FieldType::Int),
            ("ratio", FieldType::Float),
            ("done", FieldType::Bool),
        ]);
        let ddl = entity_table_ddl("md.obsidian.Note", &d).expect("builds");
        assert!(ddl.contains("CREATE NODE TABLE IF NOT EXISTS e_md_obsidian_Note_"));
        // Reserved columns present.
        assert!(ddl.contains("id STRING"));
        assert!(ddl.contains("_type STRING"));
        assert!(ddl.contains("_external_key STRING"));
        assert!(ddl.contains("_owner STRING"));
        assert!(ddl.contains("PRIMARY KEY(id)"));
        // Field columns with the mapped Ladybug types.
        assert!(ddl.contains("title STRING"));
        assert!(ddl.contains("count INT64"));
        assert!(ddl.contains("ratio DOUBLE"));
        assert!(ddl.contains("done BOOL"));
    }

    #[test]
    fn an_unsafe_field_name_fails_closed() {
        let d = def(&[("bad name", FieldType::String)]);
        assert_eq!(
            entity_table_ddl("com.x.T", &d),
            Err(EntityTableError::UnsafeFieldName("bad name".into()))
        );
    }

    #[test]
    fn a_field_colliding_with_a_reserved_column_fails_closed() {
        let d = def(&[("_owner", FieldType::String)]);
        assert_eq!(
            entity_table_ddl("com.x.T", &d),
            Err(EntityTableError::ReservedFieldName("_owner".into()))
        );
    }

    // Empirically validate the generated DDL against a real Ladybug instance:
    // the table is created and a node carrying every column type round-trips.
    // This is what confirms the FieldType -> column-type mapping (incl. DOUBLE).
    #[test]
    fn generated_ddl_creates_a_working_table() {
        use lbug::{Connection, Database, SystemConfig, Value};

        let tmp = tempfile::TempDir::new().unwrap();
        let db = Database::new(tmp.path().join("g").to_str().unwrap(), SystemConfig::default())
            .unwrap();
        let conn = Connection::new(&db).unwrap();

        let d = def(&[
            ("title", FieldType::String),
            ("count", FieldType::Int),
            ("ratio", FieldType::Float),
            ("done", FieldType::Bool),
        ]);
        let ddl = entity_table_ddl("md.obsidian.Note", &d).expect("builds");
        conn.query(&ddl).expect("the generated DDL is valid Ladybug");

        let table = entity_table_name("md.obsidian.Note");
        conn.query(&format!(
            "CREATE (:{table} {{id:'n1', _type:'md.obsidian.Note', _owner:'md.obsidian', \
             _version:1, _deleted:false, title:'Hello', count:3, ratio:0.5, done:true}})"
        ))
        .expect("a node with every column type inserts");

        let mut qr = conn
            .query(&format!("MATCH (n:{table} {{id:'n1'}}) RETURN n.title, n.count, n.ratio, n.done"))
            .expect("the node reads back");
        let row = qr.next().expect("one row");
        assert!(matches!(&row[0], Value::String(s) if s == "Hello"));
        assert!(matches!(row[1], Value::Int64(3)));
        assert!(matches!(row[2], Value::Double(d) if (d - 0.5).abs() < f64::EPSILON));
        assert!(matches!(row[3], Value::Bool(true)));
    }

    #[test]
    fn entity_node_id_is_type_scoped_and_stable() {
        assert_eq!(entity_node_id("md.obsidian.Note", "k1"), "md.obsidian.Note:k1");
        // Same key under a different type is a different node (no cross-type collision).
        assert_ne!(
            entity_node_id("md.obsidian.Note", "k1"),
            entity_node_id("md.obsidian.Task", "k1"),
        );
    }

    #[test]
    fn upsert_cypher_escapes_string_field_values() {
        let mut fields = BTreeMap::new();
        fields.insert(
            "title".to_string(),
            serde_json::json!("O'Brien'}) DETACH DELETE n //"),
        );
        let cypher = build_upsert_cypher(
            "md.obsidian.Note",
            "note-1",
            "md.obsidian",
            &fields,
            "2026-01-01T00:00:00Z",
        );
        // The single quote in the value is escaped, so it cannot close the literal.
        assert!(cypher.contains("O\\'Brien"), "value not escaped: {cypher}");
        assert!(!cypher.contains("O'Brien'"), "unescaped breakout present: {cypher}");
        assert!(cypher.contains("MERGE (n:e_md_obsidian_Note_"));
        assert!(cypher.contains("ON CREATE SET"));
        assert!(cypher.contains("ON MATCH SET"));
        assert!(cypher.contains("n.title="));
    }

    #[test]
    fn upsert_round_trips_idempotently_on_a_real_graph() {
        use lbug::{Connection, Database, SystemConfig, Value};
        let tmp = tempfile::TempDir::new().unwrap();
        let db =
            Database::new(tmp.path().join("g").to_str().unwrap(), SystemConfig::default()).unwrap();
        let conn = Connection::new(&db).unwrap();

        let d = def(&[("title", FieldType::String), ("count", FieldType::Int)]);
        conn.query(&entity_table_ddl("md.obsidian.Note", &d).unwrap())
            .expect("table DDL is valid");

        let mut f1 = BTreeMap::new();
        f1.insert("title".to_string(), serde_json::json!("First"));
        f1.insert("count".to_string(), serde_json::json!(1));
        conn.query(&build_upsert_cypher(
            "md.obsidian.Note",
            "n-1",
            "md.obsidian",
            &f1,
            "2026-01-01T00:00:00Z",
        ))
        .expect("first upsert");

        // Re-sync the SAME external key with new values: must update in place.
        let mut f2 = BTreeMap::new();
        f2.insert("title".to_string(), serde_json::json!("Second"));
        f2.insert("count".to_string(), serde_json::json!(2));
        conn.query(&build_upsert_cypher(
            "md.obsidian.Note",
            "n-1",
            "md.obsidian",
            &f2,
            "2026-01-02T00:00:00Z",
        ))
        .expect("re-sync upsert");

        let table = entity_table_name("md.obsidian.Note");
        // Exactly one node (no duplicate on re-sync).
        let mut c = conn
            .query(&format!("MATCH (n:{table}) RETURN count(*) AS c"))
            .unwrap();
        assert!(matches!(c.next().unwrap()[0], Value::Int64(1)), "one node");

        // Updated fields, bumped version, owner stamped on first insert.
        let mut qr = conn
            .query(&format!(
                "MATCH (n:{table} {{id:'md.obsidian.Note:n-1'}}) \
                 RETURN n.title, n.count, n._version, n._owner"
            ))
            .unwrap();
        let r = qr.next().unwrap();
        assert!(matches!(&r[0], Value::String(s) if s == "Second"));
        assert!(matches!(r[1], Value::Int64(2)));
        assert!(matches!(r[2], Value::Int64(2)), "version bumped to 2");
        assert!(matches!(&r[3], Value::String(s) if s == "md.obsidian"));
    }

    #[test]
    fn plan_entity_upsert_enforces_namespace_and_schema() {
        use crate::token::{CapabilityToken, EntityScope, InstanceScope};

        let mut reg = SchemaRegistry::new(vec![]);
        reg.load_from_str(
            "[meta]\nnamespace = \"md.obsidian\"\n\n[entities.Note]\n\
             [entities.Note.fields.title]\ntype = \"string\"\n",
        )
        .unwrap();
        // A token whose write scopes WOULD match all three types, so the
        // namespace/system guards (not merely the scope check) are what reject.
        let scope = |t: &str| EntityScope {
            entity_type: t.to_string(),
            fields: None,
            exclude_fields: vec![],
        };
        let token = CapabilityToken::new(
            "md.obsidian".into(),
            1234,
            vec![],
            vec![
                scope("md.obsidian.Note"),
                scope("com.other.Note"),
                scope("system.File"),
            ],
            vec![],
            InstanceScope::Own,
        );
        let mut fields = HashMap::new();
        fields.insert("title".to_string(), serde_json::json!("hi"));

        // Own namespace, registered type, valid fields → planned.
        assert!(plan_entity_upsert(&reg, &token, "md.obsidian.Note", "k1", fields.clone()).is_ok());
        // system.* is structurally unwritable even with a matching scope.
        assert!(plan_entity_upsert(&reg, &token, "system.File", "k1", fields.clone()).is_err());
        // A foreign namespace is rejected by the namespace bound.
        assert!(plan_entity_upsert(&reg, &token, "com.other.Note", "k1", fields.clone()).is_err());
        // An empty external_key is rejected (no idempotency key).
        assert!(plan_entity_upsert(&reg, &token, "md.obsidian.Note", "", fields.clone()).is_err());
        // An unknown field for the registered type fails validation.
        let mut bad = HashMap::new();
        bad.insert("nope".to_string(), serde_json::json!("x"));
        assert!(plan_entity_upsert(&reg, &token, "md.obsidian.Note", "k1", bad).is_err());
    }

    fn obsidian_token() -> crate::token::CapabilityToken {
        use crate::token::{CapabilityToken, EntityScope, InstanceScope};
        let scope = |t: &str| EntityScope {
            entity_type: t.to_string(),
            fields: None,
            exclude_fields: vec![],
        };
        // Write scopes WOULD match every endpoint below, so the namespace/system
        // guards (not the scope check) are what reject.
        CapabilityToken::new(
            "md.obsidian".into(),
            1234,
            vec![],
            vec![
                scope("md.obsidian.Note"),
                scope("com.other.Note"),
                scope("system.File"),
            ],
            vec![],
            InstanceScope::Own,
        )
    }

    fn obsidian_registry() -> SchemaRegistry {
        let mut reg = SchemaRegistry::new(vec![]);
        reg.load_from_str(
            "[meta]\nnamespace = \"md.obsidian\"\n\n[entities.Note]\n\
             [entities.Note.fields.title]\ntype = \"string\"\n",
        )
        .unwrap();
        reg
    }

    #[test]
    fn plan_entity_link_enforces_namespace_on_both_endpoints() {
        let reg = obsidian_registry();
        let token = obsidian_token();

        // Own namespace, both registered, a safe edge -> planned.
        let (ddl, cypher) =
            plan_entity_link(&reg, &token, "LINKS_TO", "md.obsidian.Note", "a", "md.obsidian.Note", "b")
                .unwrap();
        assert!(ddl.contains("CREATE REL TABLE IF NOT EXISTS r_LINKS_TO_"));
        assert!(cypher.contains("MERGE (a)-[:r_LINKS_TO_"));
        assert!(cypher.contains("md.obsidian.Note:a"), "from id present: {cypher}");
        assert!(cypher.contains("md.obsidian.Note:b"), "to id present: {cypher}");

        // system.* as either endpoint is structurally unlinkable.
        assert!(plan_entity_link(&reg, &token, "LINKS_TO", "md.obsidian.Note", "a", "system.File", "b").is_err());
        assert!(plan_entity_link(&reg, &token, "LINKS_TO", "system.File", "a", "md.obsidian.Note", "b").is_err());
        // A foreign-namespace endpoint is rejected even with a matching scope (no
        // cross-tenant edge).
        assert!(plan_entity_link(&reg, &token, "LINKS_TO", "md.obsidian.Note", "a", "com.other.Note", "b").is_err());
        // An unregistered endpoint type is rejected.
        assert!(plan_entity_link(&reg, &token, "LINKS_TO", "md.obsidian.Note", "a", "md.obsidian.Ghost", "b").is_err());
        // An unsafe edge type (it becomes the rel-table label) is rejected.
        assert!(plan_entity_link(&reg, &token, "bad edge!", "md.obsidian.Note", "a", "md.obsidian.Note", "b").is_err());
        // An empty endpoint key is rejected.
        assert!(plan_entity_link(&reg, &token, "LINKS_TO", "md.obsidian.Note", "", "md.obsidian.Note", "b").is_err());
    }

    #[test]
    fn link_cypher_escapes_endpoint_ids_against_injection() {
        // A single quote in an id cannot close the literal and inject Cypher.
        let cypher = build_link_cypher("r_x", "ta", "tb", "md.obsidian.Note:k') DETACH DELETE", "tb:k2");
        assert!(cypher.contains("k\\') DETACH DELETE"), "id not escaped: {cypher}");
        assert!(!cypher.contains("k') DETACH DELETE\"}"), "unescaped breakout: {cypher}");
        assert!(cypher.contains("MERGE (a)-[:r_x]->(b)"), "merge intact: {cypher}");
    }

    #[test]
    fn rel_table_name_is_unique_per_edge_and_endpoint_pair() {
        // The same edge label between different type pairings gets distinct tables
        // (a REL TABLE binds one FROM/TO pair), and the name is a legal identifier.
        let a = entity_rel_table_name("LINKS_TO", "md.obsidian.Note", "md.obsidian.Note");
        let b = entity_rel_table_name("LINKS_TO", "md.obsidian.Note", "md.obsidian.Tag");
        assert_ne!(a, b, "different endpoint pairs must not collide");
        assert!(is_safe_identifier(&a), "rel table name is a legal identifier: {a}");
        // Deterministic across calls.
        assert_eq!(a, entity_rel_table_name("LINKS_TO", "md.obsidian.Note", "md.obsidian.Note"));
    }

    #[test]
    fn link_merges_an_idempotent_edge_on_a_real_graph() {
        use lbug::{Connection, Database, SystemConfig, Value};
        let tmp = tempfile::TempDir::new().unwrap();
        let db =
            Database::new(tmp.path().join("g").to_str().unwrap(), SystemConfig::default()).unwrap();
        let conn = Connection::new(&db).unwrap();

        let d = def(&[("title", FieldType::String)]);
        conn.query(&entity_table_ddl("md.obsidian.Note", &d).unwrap())
            .unwrap();
        for key in ["n-1", "n-2"] {
            let mut f = BTreeMap::new();
            f.insert("title".to_string(), serde_json::json!(key));
            conn.query(&build_upsert_cypher(
                "md.obsidian.Note",
                key,
                "md.obsidian",
                &f,
                "2026-01-01T00:00:00Z",
            ))
            .unwrap();
        }

        let ty = "md.obsidian.Note";
        let table = entity_table_name(ty);
        let rel = entity_rel_table_name("LINKS_TO", ty, ty);
        conn.query(&format!(
            "CREATE REL TABLE IF NOT EXISTS {rel}(FROM {table} TO {table})"
        ))
        .unwrap();

        let link = build_link_cypher(
            &rel,
            &table,
            &table,
            &entity_node_id(ty, "n-1"),
            &entity_node_id(ty, "n-2"),
        );
        // First link creates the edge.
        let mut qr = conn.query(&link).unwrap();
        assert!(matches!(qr.next().unwrap()[0], Value::Int64(1)), "edge created");
        // Re-link is idempotent (MERGE): still exactly one edge.
        conn.query(&link).unwrap();
        let mut ec = conn
            .query(&format!(
                "MATCH (:{table})-[r:{rel}]->(:{table}) RETURN count(*) AS c"
            ))
            .unwrap();
        assert!(matches!(ec.next().unwrap()[0], Value::Int64(1)), "one edge after re-link");

        // A forward reference to a not-yet-synced node links nothing (count 0),
        // which a re-sync resolves once the target exists.
        let fwd = build_link_cypher(
            &rel,
            &table,
            &table,
            &entity_node_id(ty, "n-1"),
            &entity_node_id(ty, "n-3"),
        );
        let mut fc = conn.query(&fwd).unwrap();
        assert!(matches!(fc.next().unwrap()[0], Value::Int64(0)), "absent endpoint not linked");
    }
}
