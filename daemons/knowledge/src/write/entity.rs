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

use crate::schema::{EntityDefinition, FieldType};

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
}
