//! Schema file parser for `entities.toml` files.
//!
//! Parses the TOML schema format defined in ENTITY-SCHEMA-SYSTEM.md
//! into structured Rust types.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::SchemaError;

/// A complete schema file for one application.
#[derive(Debug, Clone, Deserialize)]
pub struct SchemaFile {
    /// Schema metadata.
    pub meta: SchemaMeta,
    /// Entity type definitions keyed by local name (e.g. "Card", "Deck").
    #[serde(default)]
    pub entities: HashMap<String, EntityDefinition>,
    /// Relation definitions keyed by relation name (e.g. "SIMILAR_TO").
    #[serde(default)]
    pub relations: HashMap<String, RelationDefinition>,
}

/// Schema metadata section.
#[derive(Debug, Clone, Deserialize)]
pub struct SchemaMeta {
    /// Schema format version (currently 1).
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Application namespace (reverse-domain, e.g. "com.anki").
    pub namespace: String,
    /// Human-readable description.
    #[serde(default)]
    pub description: String,
}

fn default_schema_version() -> u32 {
    1
}

/// Definition of a single entity type.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct EntityDefinition {
    /// Entity schema version for migrations.
    #[serde(default = "default_entity_version")]
    pub version: u32,
    /// Human-readable description.
    #[serde(default)]
    pub description: String,
    /// Lucide icon name for UI.
    #[serde(default)]
    pub icon: String,
    /// Field definitions.
    #[serde(default)]
    pub fields: HashMap<String, FieldDefinition>,
    /// Lifecycle configuration.
    #[serde(default)]
    pub lifecycle: LifecycleConfig,
    /// The type's name in front of a person, declared by whoever defined the
    /// type. Empty means undeclared, and an undeclared type is shown by its
    /// identifier rather than by a name derived from it.
    #[serde(default)]
    pub display_name: String,
    /// How the interface lays this type out. Absent means [`DisplayClass::Other`].
    #[serde(default)]
    pub display_class: DisplayClass,
    /// Which declared field carries an entry's title.
    ///
    /// **NONE MEANS THE ENTRY SHOWS ITS IDENTIFIER.** It does not fall back to
    /// the first string field: a library confidently showing the wrong line is
    /// worse than one showing a plain id, and every other guess of that shape
    /// has been removed from this tree.
    pub title_field: Option<String>,
    /// Which declared field carries the quiet second line, when the type has one.
    pub subtitle_field: Option<String>,
}

/// How the interface lays a type out.
///
/// A **closed set** rather than free text (`bridge-architecture.md`, "A bridged
/// type declares how it is displayed"): the library renders what it is told
/// instead of guessing a layout for a class it has never seen. `other` always
/// exists and renders generically, so an unusual source is never blocked from
/// shipping - and an unrecognised value is refused at parse rather than quietly
/// becoming `other`, so a typo in a connector's schema is something its author
/// finds out about.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DisplayClass {
    /// Short authored text: notes, entries, snippets.
    Notes,
    /// Longer authored or collected text: papers, books, files.
    Documents,
    /// Correspondence, which has a sender and a moment.
    Messages,
    /// Images, audio, video.
    Media,
    /// Everything else, laid out generically. Also the value a type that
    /// declared none is read as.
    #[default]
    Other,
}

impl DisplayClass {
    /// The wire form, which is also the TOML spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Notes => "notes",
            Self::Documents => "documents",
            Self::Messages => "messages",
            Self::Media => "media",
            Self::Other => "other",
        }
    }
}

impl EntityDefinition {
    /// What to call this type in front of a person: its declared display name,
    /// or the qualified identifier when it declared none. The identifier is the
    /// deliberate fallback - `md.obsidian.Note` is plain and true, whereas
    /// deriving "Note" from it invents a name its author never wrote.
    pub fn display_label<'a>(&'a self, qualified_type: &'a str) -> &'a str {
        let declared = self.display_name.trim();
        if declared.is_empty() {
            qualified_type
        } else {
            declared
        }
    }
}

fn default_entity_version() -> u32 {
    1
}

/// Definition of a single field on an entity.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FieldDefinition {
    /// Field type.
    #[serde(rename = "type")]
    pub field_type: FieldType,
    /// Whether the field is required on entity creation.
    #[serde(default)]
    pub required: bool,
    /// Default value (must match field type).
    pub default: Option<toml::Value>,
    /// Human-readable description.
    #[serde(default)]
    pub description: String,
    /// Whether the field is indexed for fast queries.
    #[serde(default)]
    pub indexed: bool,
    /// Whether the field has a uniqueness constraint.
    #[serde(default)]
    pub unique: bool,
    /// Whether the field is immutable after creation.
    #[serde(default)]
    pub immutable: bool,
    /// Whether the field contains sensitive data.
    #[serde(default)]
    pub sensitive: bool,
    /// On-delete behavior for reference fields.
    #[serde(default)]
    pub on_delete: Option<OnDelete>,
}

/// Supported field types.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    #[default]
    String,
    Text,
    Int,
    Float,
    Bool,
    Datetime,
    Date,
    Duration,
    Url,
    Email,
    Path,
    Json,
    Markdown,
    Color,
    Bytes,
    Uuid,
    #[serde(rename = "string[]")]
    StringList,
    #[serde(rename = "int[]")]
    IntList,
    #[serde(rename = "float[]")]
    FloatList,
    #[serde(rename = "bool[]")]
    BoolList,
    /// Reference to another entity, e.g. "ref:Deck" or "ref:system.File".
    #[serde(untagged)]
    Reference(String),
}

/// On-delete behavior for reference fields.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OnDelete {
    /// Set reference to null (default).
    #[default]
    Nullify,
    /// Delete this entity too.
    Cascade,
    /// Prevent deletion of the referenced entity.
    Restrict,
    /// Do nothing (orphan reference).
    NoAction,
}

/// Definition of a relation (typed edge) between entities.
#[derive(Debug, Clone, Deserialize)]
pub struct RelationDefinition {
    /// Source entity type (local name or fully qualified).
    pub from: String,
    /// Target entity type.
    pub to: String,
    /// Cardinality constraint.
    #[serde(default)]
    pub cardinality: Cardinality,
    /// Whether the relation is symmetric (A->B implies B->A).
    #[serde(default)]
    pub symmetric: bool,
    /// Human-readable description.
    #[serde(default)]
    pub description: String,
    /// Properties on the relation edge.
    #[serde(default)]
    pub properties: HashMap<String, FieldDefinition>,
}

/// Cardinality constraint for relations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Cardinality {
    OneToOne,
    OneToMany,
    ManyToOne,
    #[default]
    ManyToMany,
}

/// Entity lifecycle configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct LifecycleConfig {
    /// Days in trash before permanent deletion.
    #[serde(default = "default_trash_retention")]
    pub trash_retention_days: u32,
    /// Whether to keep history (versioning).
    #[serde(default)]
    pub history_enabled: bool,
}

fn default_trash_retention() -> u32 {
    30
}

impl Default for LifecycleConfig {
    fn default() -> Self {
        Self {
            trash_retention_days: default_trash_retention(),
            history_enabled: false,
        }
    }
}

impl SchemaFile {
    /// Load and parse a schema file from disk.
    pub fn load(path: &Path) -> Result<Self, SchemaError> {
        let content = std::fs::read_to_string(path)?;
        Self::parse(&content)
    }

    /// Parse a schema from a TOML string.
    pub fn parse(content: &str) -> Result<Self, SchemaError> {
        toml::from_str(content).map_err(|e| SchemaError::Parse(e.to_string()))
    }

    /// Get the fully qualified type name for an entity.
    pub fn full_type(&self, entity_name: &str) -> String {
        format!("{}.{}", self.meta.namespace, entity_name)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_SCHEMA: &str = r#"
[meta]
schema_version = 1
namespace = "com.anki"
description = "Anki flashcard app"

[entities.Card]
version = 1
description = "A flashcard"
icon = "book-open"

[entities.Card.fields.front]
type = "string"
required = true
description = "Question side"

[entities.Card.fields.back]
type = "string"
required = true

[entities.Card.fields.tags]
type = "string[]"

[entities.Card.fields.ease_factor]
type = "float"
default = 2.5

[entities.Card.fields.due_date]
type = "datetime"
indexed = true

[entities.Card.fields.is_suspended]
type = "bool"
default = false

[entities.Card.lifecycle]
trash_retention_days = 60
history_enabled = true

[entities.Deck]
version = 1
description = "A collection of cards"

[entities.Deck.fields.name]
type = "string"
required = true
unique = true

[entities.Deck.fields.description]
type = "text"

[relations.SIMILAR_TO]
from = "Card"
to = "Card"
cardinality = "many-to-many"
symmetric = true
description = "Cards covering similar topics"

[relations.BELONGS_TO]
from = "Card"
to = "Deck"
cardinality = "many-to-one"
"#;

    #[test]
    fn test_parse_valid_schema() {
        let schema = SchemaFile::parse(SAMPLE_SCHEMA).unwrap();
        assert_eq!(schema.meta.namespace, "com.anki");
        assert_eq!(schema.meta.schema_version, 1);
        assert_eq!(schema.entities.len(), 2);
        assert!(schema.entities.contains_key("Card"));
        assert!(schema.entities.contains_key("Deck"));
    }

    #[test]
    fn test_entity_fields() {
        let schema = SchemaFile::parse(SAMPLE_SCHEMA).unwrap();
        let card = &schema.entities["Card"];
        assert_eq!(card.fields.len(), 6);
        assert!(card.fields["front"].required);
        assert_eq!(card.fields["front"].field_type, FieldType::String);
        assert!(!card.fields["ease_factor"].required);
        assert_eq!(card.fields["ease_factor"].field_type, FieldType::Float);
        assert!(card.fields["due_date"].indexed);
    }

    #[test]
    fn test_field_types() {
        let schema = SchemaFile::parse(SAMPLE_SCHEMA).unwrap();
        let card = &schema.entities["Card"];
        assert_eq!(card.fields["front"].field_type, FieldType::String);
        assert_eq!(card.fields["ease_factor"].field_type, FieldType::Float);
        assert_eq!(card.fields["due_date"].field_type, FieldType::Datetime);
        assert_eq!(card.fields["is_suspended"].field_type, FieldType::Bool);
        assert_eq!(card.fields["tags"].field_type, FieldType::StringList);
    }

    #[test]
    fn the_shipped_obsidian_bridge_schema_parses() {
        // The Obsidian bridge ships this to /var/lib/arlen/schemas/; the daemon
        // validates every md.obsidian.* entity-write against it. Parse the SHIPPED
        // file with the real parser so a malformed bridge schema fails CI here,
        // not at the user's first vault sync (foreign-app-bridges.md).
        const OBSIDIAN: &str =
            include_str!("../../../bridge-ingest/examples/obsidian/entities.toml");
        let schema = SchemaFile::parse(OBSIDIAN).expect("obsidian entities.toml parses");
        assert_eq!(schema.meta.namespace, "md.obsidian");
        let note = &schema.entities["Note"];
        assert!(note.fields["title"].required);
        assert_eq!(note.fields["title"].field_type, FieldType::Text);
        assert_eq!(note.fields["tags"].field_type, FieldType::StringList);
        assert_eq!(note.fields["links"].field_type, FieldType::StringList);
        // And it declares its display identity, so the Library shows "Notes"
        // laid out as notes rather than `md.obsidian.Note` under `other`. The
        // title field is asserted to be one the type actually declares, which is
        // the half a shipped file can get wrong without anybody noticing.
        assert_eq!(note.display_class, DisplayClass::Notes);
        assert_eq!(note.display_label("md.obsidian.Note"), "Notes");
        let title = note.title_field.as_deref().expect("a declared title field");
        assert!(note.fields.contains_key(title));
    }

    #[test]
    fn test_lifecycle_config() {
        let schema = SchemaFile::parse(SAMPLE_SCHEMA).unwrap();
        let card = &schema.entities["Card"];
        assert_eq!(card.lifecycle.trash_retention_days, 60);
        assert!(card.lifecycle.history_enabled);

        let deck = &schema.entities["Deck"];
        assert_eq!(deck.lifecycle.trash_retention_days, 30); // default
        assert!(!deck.lifecycle.history_enabled);
    }

    #[test]
    fn test_relations() {
        let schema = SchemaFile::parse(SAMPLE_SCHEMA).unwrap();
        assert_eq!(schema.relations.len(), 2);

        let similar = &schema.relations["SIMILAR_TO"];
        assert_eq!(similar.from, "Card");
        assert_eq!(similar.to, "Card");
        assert!(similar.symmetric);
        assert_eq!(similar.cardinality, Cardinality::ManyToMany);

        let belongs = &schema.relations["BELONGS_TO"];
        assert_eq!(belongs.from, "Card");
        assert_eq!(belongs.to, "Deck");
        assert!(!belongs.symmetric);
        assert_eq!(belongs.cardinality, Cardinality::ManyToOne);
    }

    #[test]
    fn test_full_type() {
        let schema = SchemaFile::parse(SAMPLE_SCHEMA).unwrap();
        assert_eq!(schema.full_type("Card"), "com.anki.Card");
        assert_eq!(schema.full_type("Deck"), "com.anki.Deck");
    }

    #[test]
    fn test_unique_field() {
        let schema = SchemaFile::parse(SAMPLE_SCHEMA).unwrap();
        assert!(schema.entities["Deck"].fields["name"].unique);
        assert!(!schema.entities["Card"].fields["front"].unique);
    }

    #[test]
    fn test_default_values() {
        let schema = SchemaFile::parse(SAMPLE_SCHEMA).unwrap();
        let ease = &schema.entities["Card"].fields["ease_factor"];
        assert_eq!(ease.default.as_ref().unwrap().as_float(), Some(2.5));

        let suspended = &schema.entities["Card"].fields["is_suspended"];
        assert_eq!(
            suspended.default.as_ref().unwrap().as_bool(),
            Some(false)
        );
    }

    #[test]
    fn test_minimal_schema() {
        let minimal = r#"
[meta]
namespace = "com.test"

[entities.Item]
[entities.Item.fields.name]
type = "string"
"#;
        let schema = SchemaFile::parse(minimal).unwrap();
        assert_eq!(schema.meta.namespace, "com.test");
        assert_eq!(schema.meta.schema_version, 1); // default
        assert_eq!(schema.entities.len(), 1);
    }

    #[test]
    fn a_type_declares_how_it_is_displayed() {
        let declared = r#"
[meta]
namespace = "md.obsidian"

[entities.Note]
display_name = "Notes"
display_class = "notes"
title_field = "title"
subtitle_field = "folder"
[entities.Note.fields.title]
type = "string"
[entities.Note.fields.folder]
type = "string"
"#;
        let schema = SchemaFile::parse(declared).unwrap();
        let note = &schema.entities["Note"];
        assert_eq!(note.display_class, DisplayClass::Notes);
        assert_eq!(note.title_field.as_deref(), Some("title"));
        assert_eq!(note.subtitle_field.as_deref(), Some("folder"));
        assert_eq!(note.display_label("md.obsidian.Note"), "Notes");
    }

    #[test]
    fn a_type_that_declares_nothing_is_other_and_shows_its_identifier() {
        // The transition state bridge-architecture.md names: existing types gain
        // the fields, and until they do they render generically under their own
        // id - visibly plain, never wrong. So the defaults ARE the fallback, and
        // nothing here may quietly become a nicer-looking guess.
        let bare = r#"
[meta]
namespace = "com.test"

[entities.Item]
[entities.Item.fields.name]
type = "string"
"#;
        let item = &SchemaFile::parse(bare).unwrap().entities["Item"];
        assert_eq!(item.display_class, DisplayClass::Other);
        assert_eq!(item.title_field, None);
        assert_eq!(item.subtitle_field, None);
        // Not "Item", and not the `name` field it happens to have.
        assert_eq!(item.display_label("com.test.Item"), "com.test.Item");
    }

    #[test]
    fn a_display_class_outside_the_closed_set_is_refused() {
        // The whole point of a closed set: an unrecognised class must not become
        // `other` behind its author's back, or the typo ships and renders.
        let typo = r#"
[meta]
namespace = "com.test"

[entities.Item]
display_class = "notez"
[entities.Item.fields.name]
type = "string"
"#;
        assert!(SchemaFile::parse(typo).is_err());
        // And a whitespace-only display name is undeclared, not a blank label.
        let blank = r#"
[meta]
namespace = "com.test"

[entities.Item]
display_name = "   "
[entities.Item.fields.name]
type = "string"
"#;
        let item = &SchemaFile::parse(blank).unwrap().entities["Item"];
        assert_eq!(item.display_label("com.test.Item"), "com.test.Item");
    }
}
